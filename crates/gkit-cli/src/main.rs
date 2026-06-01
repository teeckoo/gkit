//! gkit — a transparent git/ssh toolkit.
//!
//! Noun-style subcommands over `gkit-core`: `clone` (config-driven, hooked),
//! `logoff` (the log-off gate), `stmb` (switch-to-main-branch,
//! recursive + safe), `key` (ssh keys). Mutating actions support `--dry-run` and
//! confirm before acting (skip with `--yes`).

use clap::{Args, Parser, Subcommand};
use gkit_core::git::{Git, SystemGit};
use gkit_core::{checks, clone, conf, config, key, report, stmb, submodules};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[derive(Parser)]
#[command(name = "gkit", version, about = "A transparent git/ssh toolkit", long_about = None)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a starter clone conf in the current directory (host/namespace inferred
    /// from origin when possible).
    Init(InitArgs),
    /// Clone repos from a conf file (built-in submodule branch-switch + direnv trust).
    Clone(CloneArgs),
    /// Log-off check: is every repo + submodule committed and pushed? (exit 0 = clear)
    Logoff(LogoffArgs),
    /// Switch to the base branch, update it, and delete the finished feature branch
    /// — recursively across submodules, with safe (merged-only) deletion.
    Stmb(StmbArgs),
    /// Manage ssh keys / identities (the gkit-owned ~/.ssh/git_users).
    Key(KeyArgs),
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Init(a) => init_cmd(a),
        Cmd::Clone(a) => clone_cmd(a),
        Cmd::Logoff(a) => logoff_cmd(a),
        Cmd::Stmb(a) => stmb_cmd(a),
        Cmd::Key(a) => key_cmd(a),
    }
}

// ---------------------------------------------------------------- init

#[derive(Args)]
struct InitArgs {
    /// File to create (default: repos.toml in the current directory).
    #[arg(default_value = "repos.toml")]
    file: String,
    /// Overwrite if the file already exists.
    #[arg(long)]
    force: bool,
}

fn init_cmd(args: InitArgs) -> ExitCode {
    let path = PathBuf::from(&args.file);
    if path.exists() && !args.force {
        return die(&format!(
            "{} already exists (use --force to overwrite)",
            args.file
        ));
    }
    // Best-effort: infer host/namespace from the current repo's origin.
    let origin = SystemGit.run(Path::new("."), &["remote", "get-url", "origin"]);
    let parts = if origin.success {
        conf::scp_url_parts(origin.trimmed())
    } else {
        None
    };
    let (host, ns) = match &parts {
        Some((h, n)) => (Some(h.as_str()), Some(n.as_str())),
        None => (None, None),
    };
    let text = conf::template(host, ns);
    if let Err(e) = std::fs::write(&path, &text) {
        return die(&format!("cannot write {}: {e}", args.file));
    }
    println!("created {}", args.file);
    match parts {
        Some((h, n)) => println!("  host/namespace inferred from origin: {h}:{n}"),
        None => println!("  fill in `host` and `namespace`, then add [[repo]] blocks"),
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------- clone

#[derive(Args)]
struct CloneArgs {
    /// Conf file(s) to clone from (e.g. `repos.toml` or `*.toml`). At least one is
    /// required; a directory is not accepted — use a shell glob like `confs/*.toml`.
    paths: Vec<String>,
    /// Don't switch submodules onto their .gitmodules branch (leave detached).
    #[arg(long)]
    no_submodule_branch: bool,
    /// Don't `direnv allow` cloned repos that have an .envrc.
    #[arg(long)]
    no_direnv: bool,
}

/// Resolve explicit conf-file arguments to a de-duplicated list. At least one is
/// required, and each must be a file — a directory is rejected; use a shell glob
/// like `*.toml` for "every conf here". Shared by `clone` and `logoff --conf` so
/// both accept conf paths identically.
fn resolve_confs(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() {
        return Err("need at least one conf file, e.g. `repos.toml` or `*.toml`".into());
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        let pb = PathBuf::from(p);
        if pb.is_dir() {
            return Err(format!(
                "`{p}` is a directory — pass conf file(s), e.g. `{}/*.toml`",
                p.trim_end_matches('/')
            ));
        }
        out.push(pb);
    }
    out.dedup();
    Ok(out)
}

fn clone_cmd(args: CloneArgs) -> ExitCode {
    let confs = match resolve_confs(&args.paths) {
        Ok(c) => c,
        Err(e) => return die(&e),
    };
    let opts = clone::Opts {
        submodule_branch: !args.no_submodule_branch,
        direnv: !args.no_direnv,
    };

    let mut failed = false;
    for conf_path in &confs {
        if confs.len() > 1 {
            println!("== {} ==", conf_path.display());
        }
        let text = match std::fs::read_to_string(conf_path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("gkit: cannot read conf `{}`: {e}", conf_path.display());
                failed = true;
                continue;
            }
        };
        let cfg = match conf::parse(&text) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("gkit: {}: {e}", conf_path.display());
                failed = true;
                continue;
            }
        };
        // Fail before cloning anything if a repo can't resolve a namespace.
        if let Err(e) = cfg.validate() {
            eprintln!("gkit: {}: {e}", conf_path.display());
            failed = true;
            continue;
        }
        // clone_all prints each step in order (commands, hooks, status).
        let reports = clone::clone_all(&SystemGit, &cfg, &opts);
        if reports
            .iter()
            .any(|r| matches!(r.outcome, clone::Outcome::Failed(_)))
        {
            failed = true;
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// ---------------------------------------------------------------- logoff

#[derive(Args)]
struct LogoffArgs {
    /// Repo path(s) to check (default: the current directory) — or, with --conf,
    /// the clone conf file(s) to read.
    paths: Vec<String>,
    /// Treat the args as clone confs and check every repo listed in them. Takes
    /// explicit conf file(s) (e.g. `*.toml`), from any dir; a directory is not
    /// accepted.
    #[arg(long)]
    conf: bool,
    /// Per-check breakdown (one fact per line, path-first, greppable). Repeat
    /// (`-vv`) to also print an `R<n>` rule id on each line and a `reason` line for
    /// every failing check (R5 names the offending branch).
    #[arg(short = 'v', action = clap::ArgAction::Count)]
    verbose: u8,
    /// Explain the rules and exit 0. Bare `-e` prints the static rule catalog (no
    /// repo). `-e <N>` is a repo-aware deep dive on rule R<N>: what it checks, this
    /// repo's live state, and examples (single repo — cwd or the given path).
    #[arg(short = 'e', value_name = "RULE")]
    explain: Option<Option<u8>>,
    /// Skip fetching submodules before checking (faster / offline).
    #[arg(long)]
    no_fetch: bool,
    /// Override the base branch (root only). Otherwise: gkit.baseBranch, then
    /// remote origin/main or origin/master.
    #[arg(long)]
    base_branch: Option<String>,
}

fn logoff_cmd(args: LogoffArgs) -> ExitCode {
    // `-e` explains the rules and exits 0 (informational, not a gate). Bare `-e`
    // is a static catalog (no repo); `-e <N>` is a repo-aware deep dive for one
    // rule on a SINGLE repo (cwd, or the first path arg) — no recursion, no fetch.
    if let Some(which) = args.explain {
        match which {
            None => {
                report::print_rules();
                return ExitCode::SUCCESS;
            }
            Some(n) => {
                let Some(rule) = checks::RuleId::from_num(n) else {
                    return die(&format!("-e: no such rule {n} (valid rules are R1..R5)"));
                };
                let git = SystemGit;
                let dir = canonical(args.paths.first().map(String::as_str).unwrap_or("."));
                let base = config::resolve_base(&git, &dir, args.base_branch.as_deref());
                let solo = config::resolve_solo(&git, &dir);
                report::print_rule_detail(&checks::rule_report(&git, &dir, &base, solo, rule));
                return ExitCode::SUCCESS;
            }
        }
    }

    let git = SystemGit;
    let mut failed = false;

    // Collect the repo dirs to check: either each conf's repos, or the paths as-is.
    let mut dirs: Vec<PathBuf> = Vec::new();
    if args.conf {
        let confs = match resolve_confs(&args.paths) {
            Ok(c) => c,
            Err(e) => return die(&e),
        };
        for conf_path in &confs {
            if confs.len() > 1 {
                println!("== {} ==", conf_path.display());
            }
            let cfg = match std::fs::read_to_string(conf_path)
                .map_err(|e| e.to_string())
                .and_then(|t| conf::parse(&t))
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("gkit: {}: {e}", conf_path.display());
                    failed = true;
                    continue;
                }
            };
            for r in &cfg.repo {
                dirs.push(PathBuf::from(conf::expand_path(&r.dir, |k| {
                    std::env::var(k).ok()
                })));
            }
        }
    } else {
        let srcs: Vec<String> = if args.paths.is_empty() {
            vec![".".into()]
        } else {
            args.paths.clone()
        };
        dirs = srcs.iter().map(|p| canonical(p)).collect();
    }

    for dir in &dirs {
        // In conf mode each repo resolves its own base (gkit.baseBranch -> remote).
        let base = if args.conf {
            None
        } else {
            args.base_branch.as_deref()
        };
        let entries = submodules::evaluate_tree(&git, dir, base, !args.no_fetch);
        if args.verbose >= 1 {
            // -vv (>=2) adds R<n> prefixes + per-failure reason lines.
            report::print_verbose(&entries, args.verbose >= 2);
        } else {
            report::print_default(&entries);
        }
        if !report::all_ok(&entries) {
            failed = true;
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// ---------------------------------------------------------------- stmb

#[derive(Args)]
struct StmbArgs {
    /// Repository path (defaults to the current directory).
    #[arg(default_value = ".")]
    path: String,
    /// Base branch to switch to (root only). Otherwise: gkit.baseBranch, then origin/HEAD.
    #[arg(long)]
    base: Option<String>,
    /// Only the top repo; don't recurse into submodules.
    #[arg(long)]
    no_recursive: bool,
    /// Force-delete the feature branch even if not fully merged (may lose commits).
    #[arg(long)]
    force: bool,
    /// Skip the confirmation prompt.
    #[arg(short = 'y', long)]
    yes: bool,
    /// Show the plan without making changes.
    #[arg(long)]
    dry_run: bool,
}

enum Step {
    Switch {
        dir: PathBuf,
        base: String,
        feature: Option<String>,
    },
    Skip {
        dir: PathBuf,
        why: String,
    },
}

fn stmb_cmd(args: StmbArgs) -> ExitCode {
    let git = SystemGit;
    let root = canonical(&args.path);
    let repos = if args.no_recursive {
        vec![root.clone()]
    } else {
        submodules::repo_paths(&git, &root)
    };

    // Resolve ONE base for the whole tree (uniform convention) — avoids mis-resolving
    // a submodule's base and treating an integration branch as a deletable feature.
    let base = match config::resolve_switch_base(&git, &root, args.base.as_deref()) {
        Some(b) => b,
        None => return die("cannot determine base branch — pass --base <branch>"),
    };

    let steps: Vec<Step> = repos
        .iter()
        .map(|dir| {
            let cur = config::current_branch_opt(&git, dir);
            let dirty = !checks::committed(&git, dir);
            match stmb::plan(cur.as_deref(), &base, dirty) {
                Ok(p) => Step::Switch {
                    dir: dir.clone(),
                    base: p.base,
                    feature: p.delete_feature,
                },
                Err(why) => Step::Skip {
                    dir: dir.clone(),
                    why,
                },
            }
        })
        .collect();

    println!("stmb plan ({} repo(s)):", steps.len());
    for s in &steps {
        match s {
            Step::Switch { dir, base, feature } => {
                let del = feature
                    .as_deref()
                    .map(|f| format!(", delete '{f}'"))
                    .unwrap_or_default();
                println!("  {}  -> switch to '{base}', pull{del}", short(dir, &root));
            }
            Step::Skip { dir, why } => println!("  {}  -- skip: {why}", short(dir, &root)),
        }
    }

    if args.dry_run {
        return ExitCode::SUCCESS;
    }
    if !args.yes && !confirm("Proceed?") {
        println!("aborted.");
        return ExitCode::SUCCESS;
    }

    let mut failed = false;
    for s in &steps {
        if let Step::Switch { dir, base, feature } = s {
            println!("{}:", short(dir, &root));
            if let Err(e) = run_stmb(&git, dir, base, feature.as_deref(), args.force) {
                eprintln!("gkit stmb: {}: {e}", short(dir, &root));
                failed = true;
            }
        }
    }

    // Verify with a (recursive) log-off check on the root.
    println!("\n--- logoff ---");
    let entries = submodules::evaluate_tree(&git, &root, None, false);
    report::print_default(&entries);
    if failed || !report::all_ok(&entries) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run_stmb(
    git: &SystemGit,
    dir: &Path,
    base: &str,
    feature: Option<&str>,
    force: bool,
) -> Result<(), String> {
    // Print each git command before running it (transparency, like `clone`).
    let run = |args: &[&str]| {
        println!("  + git {}", args.join(" "));
        git.run(dir, args)
    };
    let co = run(&["checkout", base]);
    if !co.success {
        return Err(format!("checkout {base} failed: {}", co.stderr.trim()));
    }
    let _ = run(&["pull", "--rebase", "origin", base]);
    if let Some(f) = feature {
        let del = run(&["branch", "-d", f]);
        if !del.success {
            if force {
                let force_del = run(&["branch", "-D", f]);
                if !force_del.success {
                    return Err(format!(
                        "force-delete '{f}' failed: {}",
                        force_del.stderr.trim()
                    ));
                }
            } else {
                return Err(format!(
                    "'{f}' not fully merged into {base}; rerun with --force to delete anyway"
                ));
            }
        }
    }
    let _ = run(&["remote", "prune", "origin"]);
    Ok(())
}

// ---------------------------------------------------------------- key

#[derive(Args)]
struct KeyArgs {
    #[command(subcommand)]
    action: KeyAction,
}

#[derive(Subcommand)]
enum KeyAction {
    /// Generate id_<alias>, add an ssh Host block to ~/.ssh/git_users, ssh-add it,
    /// then copy the public key to the clipboard.
    Add(KeyAddArgs),
    /// List the Host aliases gkit owns in ~/.ssh/git_users.
    List,
}

#[derive(Args)]
struct KeyAddArgs {
    /// Alias = ssh Host = key name (~/.ssh/id_<alias>). Prompted if omitted.
    alias: Option<String>,
    /// Email comment for the key. Prompted if omitted.
    #[arg(long)]
    email: Option<String>,
    /// Provider hostname. Prompted (with a menu) if omitted; defaults to github.com.
    #[arg(long)]
    host: Option<String>,
    /// SSH port (omit for default 22).
    #[arg(long)]
    port: Option<u16>,
    /// Show the plan without making changes.
    #[arg(long)]
    dry_run: bool,
    /// Skip the confirmation prompt.
    #[arg(short = 'y', long)]
    yes: bool,
}

fn key_cmd(args: KeyArgs) -> ExitCode {
    match args.action {
        KeyAction::Add(a) => key_add(a),
        KeyAction::List => key_list(),
    }
}

fn ssh_dir() -> PathBuf {
    // Cross-OS home: HOME (Unix/macOS) → USERPROFILE / HOMEDRIVE+HOMEPATH (Windows).
    key::home_from_env(|k| std::env::var(k).ok())
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ssh")
}

fn key_add(a: KeyAddArgs) -> ExitCode {
    let interactive = std::io::stdin().is_terminal();
    // Required inputs: use the flag if given, else prompt (only in a terminal).
    let alias = match resolve_arg(
        a.alias,
        interactive,
        "alias (ssh Host = key name id_<alias>)",
    ) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let email = match resolve_arg(a.email, interactive, "email (key comment)") {
        Ok(v) => v,
        Err(c) => return c,
    };
    // Provider hostname: explicit --host wins; else a menu in a terminal; else default.
    let host = match a.host {
        Some(h) => h,
        None if interactive => prompt_provider(),
        None => key::PROVIDERS[0].to_string(),
    };

    let ssh = ssh_dir();
    let key_path = ssh.join(format!("id_{alias}"));
    let git_users = ssh.join("git_users");
    let ssh_config = ssh.join("config");
    let macos = cfg!(target_os = "macos");

    let block = key::host_block(&alias, &host, a.port, macos);
    let existing_gu = std::fs::read_to_string(&git_users).unwrap_or_default();
    let new_gu = key::upsert_block(&existing_gu, &alias, &block);
    let existing_cfg = std::fs::read_to_string(&ssh_config).unwrap_or_default();
    let new_cfg = key::ensure_include(&existing_cfg);
    let need_keygen = !key_path.exists();

    println!("gkit key add '{alias}':");
    if need_keygen {
        println!(
            "  ssh-keygen -t ed25519 -C {} -f {}",
            email,
            key_path.display()
        );
    } else {
        println!("  (key {} already exists — keeping it)", key_path.display());
    }
    println!("  upsert Host block into {}:", git_users.display());
    for l in block.lines() {
        println!("      {l}");
    }
    match &new_cfg {
        Some(_) => println!(
            "  ensure `Include git_users` in {} (asks first)",
            ssh_config.display()
        ),
        None => println!(
            "  (`Include git_users` already present in {})",
            ssh_config.display()
        ),
    }
    println!("  ssh-add the key, then copy the public key to the clipboard");

    if a.dry_run {
        return ExitCode::SUCCESS;
    }
    if !a.yes && !confirm("Proceed?") {
        println!("aborted.");
        return ExitCode::SUCCESS;
    }

    if let Err(e) = std::fs::create_dir_all(&ssh) {
        return die(&format!("cannot create {}: {e}", ssh.display()));
    }
    if need_keygen {
        // interactive (passphrase) -> inherit stdio
        let st = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-C", &email, "-f"])
            .arg(&key_path)
            .status();
        if !matches!(st, Ok(s) if s.success()) {
            return die("ssh-keygen failed");
        }
    }
    if let Err(e) = std::fs::write(&git_users, &new_gu) {
        return die(&format!("cannot write {}: {e}", git_users.display()));
    }
    // The sensitive edit is to the user's OWN ~/.ssh/config — check for the
    // `Include git_users` line, explain why it matters, and ask before touching it.
    match new_cfg {
        None => println!(
            "✓ `Include git_users` already present in {}",
            ssh_config.display()
        ),
        Some(c) => {
            println!(
                "! {} does not `Include git_users` — without it, ssh ignores the Host",
                ssh_config.display()
            );
            println!("  block(s) gkit manages in {}.", git_users.display());
            if a.yes
                || confirm_default(
                    &format!("Add `Include git_users` to {}?", ssh_config.display()),
                    true,
                )
            {
                if let Err(e) = std::fs::write(&ssh_config, c) {
                    return die(&format!("cannot write {}: {e}", ssh_config.display()));
                }
                println!("  added `Include git_users`.");
            } else {
                println!(
                    "  skipped — add `Include git_users` to {} yourself to activate the key.",
                    ssh_config.display()
                );
            }
        }
    }

    let mut add = Command::new("ssh-add");
    if macos {
        add.arg("--apple-use-keychain");
    }
    let _ = add.arg(&key_path).status();

    // Copy the public key to the clipboard, ready to paste into the provider.
    let pubfile = key_path.with_extension("pub");
    match std::fs::read_to_string(&pubfile) {
        Ok(pubkey) => match clipboard_copy(&pubkey) {
            Some(tool) => {
                println!(
                    "done. id_{alias}.pub copied to clipboard ({tool}) — paste it into {host}."
                )
            }
            None => {
                println!("done. public key (upload to {host}):");
                print!("{pubkey}");
            }
        },
        Err(e) => println!("done, but cannot read {}: {e}", pubfile.display()),
    }
    ExitCode::SUCCESS
}

/// Copy `text` to the OS clipboard via the first available per-OS tool
/// (pbcopy / clip / wl-copy|xclip|xsel). Returns the tool that succeeded, or
/// `None` if no clipboard tool is available (caller then prints the text).
fn clipboard_copy(text: &str) -> Option<&'static str> {
    for (prog, pargs) in key::clipboard_candidates(std::env::consts::OS) {
        let Ok(mut child) = Command::new(prog)
            .args(&pargs)
            .stdin(std::process::Stdio::piped())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().map(|s| s.success()).unwrap_or(false) {
            return Some(prog);
        }
    }
    None
}

fn key_list() -> ExitCode {
    let git_users = ssh_dir().join("git_users");
    let content = std::fs::read_to_string(&git_users).unwrap_or_default();
    let hosts = key::list_hosts(&content);
    if hosts.is_empty() {
        println!("(no Host blocks in {})", git_users.display());
    } else {
        for (alias, identity) in hosts {
            println!("{alias:<20} {identity}");
        }
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------- helpers

fn canonical(p: &str) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p))
}

fn short(dir: &Path, root: &Path) -> String {
    dir.strip_prefix(root)
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn confirm(msg: &str) -> bool {
    confirm_default(msg, false)
}

/// Yes/no prompt with an explicit default taken on empty input or EOF (so a bare
/// Enter, or a non-interactive run, follows `default_yes`).
fn confirm_default(msg: &str, default_yes: bool) -> bool {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("{msg} {hint}: ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    if std::io::stdin().read_line(&mut s).unwrap_or(0) == 0 {
        return default_yes; // EOF — take the default
    }
    match s.trim() {
        "" => default_yes,
        t => matches!(t, "y" | "Y" | "yes" | "Yes"),
    }
}

/// Resolve a value that may have been passed as a flag: return it if present,
/// else prompt for it (only when interactive). A missing value with no terminal
/// to read from is a hard error rather than a hang.
fn resolve_arg(val: Option<String>, interactive: bool, label: &str) -> Result<String, ExitCode> {
    if let Some(v) = val {
        return Ok(v);
    }
    let what = label.split_whitespace().next().unwrap_or("value");
    if !interactive {
        return Err(die(&format!("missing {what} (pass it as an argument)")));
    }
    read_line(label).ok_or_else(|| die(&format!("missing {what}")))
}

/// Read one trimmed, non-empty line. `None` on EOF or empty input.
fn read_line(label: &str) -> Option<String> {
    print!("{label}: ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    if std::io::stdin().read_line(&mut s).unwrap_or(0) == 0 {
        return None; // EOF / no terminal
    }
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Show the provider menu and return the chosen hostname. Standard options are
/// numbered, with a final "other" entry for a custom/private hostname; a bare
/// Enter takes the default. Re-asks on an out-of-range number.
fn prompt_provider() -> String {
    loop {
        println!("provider:");
        for (i, p) in key::PROVIDERS.iter().enumerate() {
            let tag = if i == 0 { "  (default)" } else { "" };
            println!("  {}) {p}{tag}", i + 1);
        }
        println!("  {}) other (custom hostname)", key::PROVIDERS.len() + 1);
        let raw =
            read_line(&format!("choose [1-{}]", key::PROVIDERS.len() + 1)).unwrap_or_default();
        match key::provider_choice(&raw) {
            key::ProviderChoice::Host(h) => return h,
            key::ProviderChoice::Custom => {
                if let Some(h) = read_line("hostname (e.g. git.mycorp.com)") {
                    return h;
                }
                // empty/EOF on the custom prompt → fall back to the default
                return key::PROVIDERS[0].to_string();
            }
            key::ProviderChoice::Invalid => {
                println!("  ? not a listed option — try again");
                continue;
            }
        }
    }
}

fn die(msg: &str) -> ExitCode {
    eprintln!("gkit: {msg}");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::resolve_confs;
    use std::fs;
    use std::path::PathBuf;

    // `clone` and `logoff --conf` accept ONLY explicit conf file(s): at least one
    // is required, a directory is rejected (use a `*.toml` shell glob instead), and
    // explicit files are kept in order, deduped.
    //
    // Assertions compare Ok/Err and file *names* (not full PathBufs), which is
    // robust to OS path normalization (Windows verbatim/short-name prefixes).
    #[test]
    fn resolve_confs_requires_explicit_files() {
        let base = std::env::temp_dir().join(format!("gkit-rc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base); // clear any stale leftovers from a prior run
        fs::create_dir_all(&base).unwrap();
        let a = base.join("a.toml");
        let b = base.join("b.toml");
        fs::write(&a, "").unwrap();
        fs::write(&b, "").unwrap();

        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let names = |r: Result<Vec<PathBuf>, String>| {
            r.unwrap()
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };

        // no args -> error (no cwd default)
        assert!(resolve_confs(&[]).is_err());

        // a directory -> error (no expansion)
        assert!(resolve_confs(&[s(&base)]).is_err());

        // explicit files -> kept in order, deduped
        assert_eq!(names(resolve_confs(&[s(&a), s(&b)])), ["a.toml", "b.toml"]);
        assert_eq!(names(resolve_confs(&[s(&a), s(&a)])), ["a.toml"]);

        let _ = fs::remove_dir_all(&base);
    }
}
