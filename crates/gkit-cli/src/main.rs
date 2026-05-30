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
        return die(&format!("{} already exists (use --force to overwrite)", args.file));
    }
    // Best-effort: infer host/namespace from the current repo's origin.
    let origin = SystemGit.run(Path::new("."), &["remote", "get-url", "origin"]);
    let parts = if origin.success { conf::scp_url_parts(origin.trimmed()) } else { None };
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
    let inputs: Vec<PathBuf> =
        if paths.is_empty() { vec![PathBuf::from(".")] } else { paths.iter().map(PathBuf::from).collect() };
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
    let opts = clone::Opts { submodule_branch: !args.no_submodule_branch, direnv: !args.no_direnv };

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
        if reports.iter().any(|r| matches!(r.outcome, clone::Outcome::Failed(_))) {
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
    /// Repository path to check (defaults to the current directory).
    #[arg(default_value = ".")]
    path: String,
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
    let root = canonical(&args.path);
    let entries = submodules::evaluate_tree(&git, &root, args.base_branch.as_deref(), !args.no_fetch);
    if args.verbose {
        report::print_verbose(&entries);
    } else {
        report::print_default(&entries);
    }
    if report::all_ok(&entries) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
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
    Switch { dir: PathBuf, base: String, feature: Option<String> },
    Skip { dir: PathBuf, why: String },
}

fn stmb_cmd(args: StmbArgs) -> ExitCode {
    let git = SystemGit;
    let root = canonical(&args.path);
    let repos = if args.no_recursive { vec![root.clone()] } else { submodules::repo_paths(&git, &root) };

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
                Ok(p) => Step::Switch { dir: dir.clone(), base: p.base, feature: p.delete_feature },
                Err(why) => Step::Skip { dir: dir.clone(), why },
            }
        })
        .collect();

    println!("stmb plan ({} repo(s)):", steps.len());
    for s in &steps {
        match s {
            Step::Switch { dir, base, feature } => {
                let del = feature.as_deref().map(|f| format!(", delete '{f}'")).unwrap_or_default();
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
    if failed || !report::all_ok(&entries) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn run_stmb(git: &SystemGit, dir: &Path, base: &str, feature: Option<&str>, force: bool) -> Result<(), String> {
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
                    return Err(format!("force-delete '{f}' failed: {}", force_del.stderr.trim()));
                }
            } else {
                return Err(format!("'{f}' not fully merged into {base}; rerun with --force to delete anyway"));
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
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".ssh")
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
        println!("  ssh-keygen -t ed25519 -C {} -f {}", a.email, key_path.display());
    } else {
        println!("  (key {} already exists — keeping it)", key_path.display());
    }
    println!("  upsert Host block into {}:", git_users.display());
    for l in block.lines() {
        println!("      {l}");
    }
    match &new_cfg {
        Some(_) => println!("  add `Include git_users` to {}", ssh_config.display()),
        None => println!("  (`Include git_users` already present in {})", ssh_config.display()),
    }
    println!("  ssh-add the key");

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
    if let Some(c) = new_cfg {
        if let Err(e) = std::fs::write(&ssh_config, c) {
            return die(&format!("cannot write {}: {e}", ssh_config.display()));
        }
    }
    let mut add = Command::new("ssh-add");
    if macos {
        add.arg("--apple-use-keychain");
    }
    let _ = add.arg(&key_path).status();

    println!("done. public key (upload to your provider, or `gkit key copy {}`):", a.alias);
    if let Ok(pubkey) = std::fs::read_to_string(key_path.with_extension("pub")) {
        print!("{pubkey}");
    }
    ExitCode::SUCCESS
}

fn key_copy(alias: &str) -> ExitCode {
    let pubfile = ssh_dir().join(format!("id_{alias}.pub"));
    let pubkey = match std::fs::read_to_string(&pubfile) {
        Ok(k) => k,
        Err(e) => return die(&format!("cannot read {}: {e}", pubfile.display())),
    };
    // pbcopy on macOS; fall back to printing if unavailable.
    if let Ok(mut child) = Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(pubkey.as_bytes());
        }
        let _ = child.wait();
        println!("copied id_{alias}.pub to clipboard");
    } else {
        print!("{pubkey}");
    }
    ExitCode::SUCCESS
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
