//! gkit — a transparent git/ssh toolkit.
//!
//! Noun-style subcommands over `gkit-core`: `clone` (config-driven, hooked),
//! `logoff` (the log-off gate), `stmb` (switch-to-main-branch,
//! recursive + safe), `key` (ssh keys). Mutating actions support `--dry-run` and
//! confirm before acting (skip with `--yes`).

use clap::{Args, Parser, Subcommand};
use gkit_core::git::{Git, SystemGit};
use gkit_core::{checks, clone, conf, config, key, report, stmb, submodules};
use std::io::Write;
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
    /// Conf files and/or directories. A directory means every `*.toml` in it.
    /// Default: every `*.toml` in the current directory.
    paths: Vec<String>,
    /// Don't switch submodules onto their .gitmodules branch (leave detached).
    #[arg(long)]
    no_submodule_branch: bool,
    /// Don't `direnv allow` cloned repos that have an .envrc.
    #[arg(long)]
    no_direnv: bool,
}

/// Resolve paths to a sorted, de-duplicated list of conf files: a file stays
/// as-is; a directory expands to its `*.toml`; no input means the cwd.
fn resolve_confs(paths: &[String]) -> Vec<PathBuf> {
    let inputs: Vec<PathBuf> = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.iter().map(PathBuf::from).collect()
    };
    let mut out: Vec<PathBuf> = Vec::new();
    for p in inputs {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                let mut tomls: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|f| f.is_file() && f.extension().is_some_and(|x| x == "toml"))
                    .collect();
                tomls.sort();
                out.extend(tomls);
            }
        } else {
            out.push(p);
        }
    }
    out.dedup();
    out
}

fn clone_cmd(args: CloneArgs) -> ExitCode {
    let confs = resolve_confs(&args.paths);
    if confs.is_empty() {
        return die("no .toml conf files found");
    }
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
    /// Repo path(s) to check — or, with --conf, clone conf file(s)/dir(s).
    /// Default: the current directory (or, with --conf, its *.toml).
    paths: Vec<String>,
    /// Treat the args as clone confs and check every repo listed in them
    /// (files may be in different dirs; a dir expands to its *.toml).
    #[arg(long)]
    conf: bool,
    /// Per-check breakdown (one fact per line, path-first, greppable).
    #[arg(short, long)]
    verbose: bool,
    /// Skip fetching submodules before checking (faster / offline).
    #[arg(long)]
    no_fetch: bool,
    /// Override the base branch (root only). Otherwise: gkit.baseBranch, then HEAD.
    #[arg(long)]
    base_branch: Option<String>,
}

fn logoff_cmd(args: LogoffArgs) -> ExitCode {
    let git = SystemGit;
    let mut failed = false;

    // Collect the repo dirs to check: either each conf's repos, or the paths as-is.
    let mut dirs: Vec<PathBuf> = Vec::new();
    if args.conf {
        if args.paths.is_empty() {
            return die(
                "--conf needs a conf file or directory, e.g. `gkit logoff --conf repos.toml`",
            );
        }
        let confs = resolve_confs(&args.paths);
        if confs.is_empty() {
            return die("no .toml conf files found in the given path(s)");
        }
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
        // In conf mode each repo resolves its own base (gkit.baseBranch -> HEAD).
        let base = if args.conf {
            None
        } else {
            args.base_branch.as_deref()
        };
        let entries = submodules::evaluate_tree(&git, dir, base, !args.no_fetch);
        if args.verbose {
            report::print_verbose(&entries);
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
            if let Err(e) = run_stmb(&git, dir, base, feature.as_deref(), args.force) {
                eprintln!("gkit stmb: {}: {e}", short(dir, &root));
                failed = true;
            }
        }
    }

    // Verify with a (recursive) log-off check on the root.
    println!("--- logoff ---");
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
    let co = git.run(dir, &["checkout", base]);
    if !co.success {
        return Err(format!("checkout {base} failed: {}", co.stderr.trim()));
    }
    let _ = git.run(dir, &["pull", "--rebase", "origin", base]);
    if let Some(f) = feature {
        let del = git.run(dir, &["branch", "-d", f]);
        if !del.success {
            if force {
                let force_del = git.run(dir, &["branch", "-D", f]);
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
    let _ = git.run(dir, &["remote", "prune", "origin"]);
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
    /// Generate id_<alias>, add an ssh Host block to ~/.ssh/git_users, ssh-add it.
    Add(KeyAddArgs),
    /// Copy id_<alias>.pub to the clipboard.
    Copy { alias: String },
    /// List the Host aliases gkit owns in ~/.ssh/git_users.
    List,
}

#[derive(Args)]
struct KeyAddArgs {
    /// Alias = ssh Host = key name (~/.ssh/id_<alias>).
    alias: String,
    /// Email comment for the key.
    #[arg(long)]
    email: String,
    /// Provider hostname.
    #[arg(long, default_value = "github.com")]
    host: String,
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
        KeyAction::Copy { alias } => key_copy(&alias),
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
    let ssh = ssh_dir();
    let key_path = ssh.join(format!("id_{}", a.alias));
    let git_users = ssh.join("git_users");
    let ssh_config = ssh.join("config");
    let macos = cfg!(target_os = "macos");

    let block = key::host_block(&a.alias, &a.host, a.port, macos);
    let existing_gu = std::fs::read_to_string(&git_users).unwrap_or_default();
    let new_gu = key::upsert_block(&existing_gu, &a.alias, &block);
    let existing_cfg = std::fs::read_to_string(&ssh_config).unwrap_or_default();
    let new_cfg = key::ensure_include(&existing_cfg);
    let need_keygen = !key_path.exists();

    println!("gkit key add '{}':", a.alias);
    if need_keygen {
        println!(
            "  ssh-keygen -t ed25519 -C {} -f {}",
            a.email,
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
            .args(["-t", "ed25519", "-C", &a.email, "-f"])
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
                || confirm(&format!(
                    "Add `Include git_users` to {}?",
                    ssh_config.display()
                ))
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
                    "done. id_{}.pub copied to clipboard ({tool}) — paste it into {}.",
                    a.alias, a.host
                )
            }
            None => {
                println!("done. public key (upload to {}):", a.host);
                print!("{pubkey}");
            }
        },
        Err(e) => println!("done, but cannot read {}: {e}", pubfile.display()),
    }
    ExitCode::SUCCESS
}

fn key_copy(alias: &str) -> ExitCode {
    let pubfile = ssh_dir().join(format!("id_{alias}.pub"));
    let pubkey = match std::fs::read_to_string(&pubfile) {
        Ok(k) => k,
        Err(e) => return die(&format!("cannot read {}: {e}", pubfile.display())),
    };
    match clipboard_copy(&pubkey) {
        Some(tool) => println!("copied id_{alias}.pub to clipboard ({tool})"),
        None => print!("{pubkey}"),
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
    print!("{msg} [y/N]: ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    matches!(s.trim(), "y" | "Y" | "yes" | "Yes")
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

    // `--conf` (and `clone`) take a list of conf sources from ANY directory: an
    // explicit file is kept as-is, a directory expands to its sorted `*.toml`
    // (non-`.toml` ignored), and a file + a dir can be mixed.
    //
    // We assert on the resolved file *names* (and order), not full PathBufs: the
    // selection/sort/exclusion logic is what matters, and name comparison is robust
    // to OS path normalization (Windows verbatim/short-name prefixes, separators).
    #[test]
    fn resolve_confs_keeps_files_and_expands_dirs() {
        let base = std::env::temp_dir().join(format!("gkit-rc-{}", std::process::id()));
        let dir = base.join("confs");
        let _ = fs::remove_dir_all(&base); // clear any stale leftovers from a prior run
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b.toml"), "").unwrap();
        fs::write(dir.join("a.toml"), "").unwrap();
        fs::write(dir.join("note.txt"), "").unwrap();
        let lone = base.join("lone.toml");
        fs::write(&lone, "").unwrap();

        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let names = |v: Vec<PathBuf>| {
            v.iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };

        // a directory -> its *.toml, sorted; note.txt excluded
        assert_eq!(names(resolve_confs(&[s(&dir)])), ["a.toml", "b.toml"]);

        // an explicit file (from a different dir) kept as-is, mixed with a dir
        assert_eq!(
            names(resolve_confs(&[s(&lone), s(&dir)])),
            ["lone.toml", "a.toml", "b.toml"]
        );

        let _ = fs::remove_dir_all(&base);
    }
}
