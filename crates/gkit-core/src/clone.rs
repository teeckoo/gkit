//! Config-driven clone with explicit flag placement, built-in stateless steps, and
//! pre/post-clone hooks.
//!
//! Per repo, in order: global `pre-clone` → repo `pre-clone` → `git <PRE> clone
//! <POST> <url> <dir>` → built-ins (git identity + submodule branch-switch +
//! `direnv allow`) → global `post-clone` → repo `post-clone`.
//!
//! Git identity (`user.name`/`user.email`) is **per-invocation, not in the conf**
//! (the conf is shared across a team): it comes from `Opts` (the `clone`
//! `--user-name`/`--user-email` flags, or an interactive prompt), and is stamped
//! `git config` on each cloned repo right after clone so `post-clone` hooks see it.
//!
//! The `git clone` and built-ins are **captured** (clean status; an `.envrc` that
//! runs `glow …` can't distort output — `direnv allow` only records trust). User
//! hooks run via `sh -c` with their output **inherited** (explicit commands, shown
//! live) and `$GKIT_REPO`/`GKIT_DIR`/`GKIT_URL`/`GKIT_HOST`/`GKIT_NAMESPACE` set
//! (plus `GKIT_USER_NAME`/`GKIT_USER_EMAIL`, empty when no identity was given).

use crate::conf::{expand_path, CloneConf, Repo};
use crate::git::Git;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Cloned,
    Skipped,
    Failed(String),
}

#[derive(Debug)]
pub struct CloneReport {
    pub name: String,
    pub dir: PathBuf,
    pub outcome: Outcome,
    pub command: String,
}

pub struct Opts {
    pub submodule_branch: bool,
    pub direnv: bool,
    /// Git identity stamped on each cloned repo (`git config user.name`). Per
    /// invocation, not from the conf — `None` leaves the repo's inherited identity.
    pub user_name: Option<String>,
    /// Git identity stamped on each cloned repo (`git config user.email`).
    pub user_email: Option<String>,
    /// Absolute path to the conf file driving this clone, stamped as `gkit.conf` on
    /// each top-level repo so `gkit stamp` (run inside the repo, no arg) can later
    /// resolve its own conf. `None` (e.g. tests) skips the stamp.
    pub conf_path: Option<String>,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            submodule_branch: true,
            direnv: true,
            user_name: None,
            user_email: None,
            conf_path: None,
        }
    }
}

// Also reused by `fixsub` (re-applies this branch-switch over an existing tree).
pub(crate) const SUBMODULE_SWITCH: &str = "b=$(git config -f \"$toplevel/.gitmodules\" \"submodule.$name.branch\" 2>/dev/null || echo main); git switch \"$b\" 2>/dev/null || true";

/// Single-quote a value for safe interpolation into an `sh -c` command line
/// (each embedded `'` becomes `'\''`). Shared with `fixsub`.
pub(crate) fn sh_squote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The `git submodule foreach --recursive` body that stamps the resolved identity
/// into each submodule, values single-quoted for `sh`. `None` when no identity was
/// given (so the caller skips the recursion entirely).
fn submodule_identity_cmd(user_name: Option<&str>, user_email: Option<&str>) -> Option<String> {
    let parts: Vec<String> = [("user.name", user_name), ("user.email", user_email)]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| format!("git config {k} {}", sh_squote(v))))
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// The git-config `(key, value)` for the **namespace-scoped** `insteadOf` rewrite that
/// lets a *canonical* submodule URL route through the alias's key:
///   key   = `url.<alias>:<ns>/.insteadOf`   value = `git@<hostname>:<ns>/`
/// so git rewrites `git@<hostname>:<ns>/repo.git` → `<alias>:<ns>/repo.git` → `id_<alias>`.
/// The trailing `/` on both sides scopes the rule to the namespace (so multiple aliases
/// on the same host — different clients — each keep their own key).
pub fn insteadof_pair(alias: &str, hostname: &str, ns: &str) -> (String, String) {
    (
        format!("url.{alias}:{ns}/.insteadOf"),
        format!("git@{hostname}:{ns}/"),
    )
}

/// Distinct namespaces across a conf's repos (each repo's effective namespace), in
/// conf order, deduplicated — one `insteadOf` rule is written per distinct namespace.
pub fn distinct_namespaces(conf: &CloneConf) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in &conf.repo {
        if let Some(ns) = conf.namespace_for(r) {
            if !out.iter().any(|n| n == ns) {
                out.push(ns.to_string());
            }
        }
    }
    out
}

/// Run hook commands, **fail-fast**, with `env` set and output inherited; each printed
/// `+ <cmd>`. Shared with `stamp`, which re-runs a conf's `post-clone` over an existing tree.
///
/// **Each command runs as its own `sh -ec '<cmd>'` process with cwd = `cwd` (the repo
/// root).** Two consequences worth knowing when writing conf hooks:
/// - **`set -e` (the `-e`)** → a multi-step command fails fast *within* the line: e.g.
///   `cd sub; git config …` stops if `cd sub` fails (the `git config` never runs). So you
///   don't need defensive `&&` chaining or `|| true` to keep a bad step from doing damage.
/// - **fresh process per line, from the repo root** → cwd does **not** persist across
///   lines (a `cd` on one line can't leak into the next — equivalent to a subshell, but
///   stronger). Keep a `cd` and its command on the *same* line: `cd sub && git config …`.
///
/// The whole array is still fail-fast: the first command that exits non-zero aborts the
/// rest and returns `Err` (the caller marks the repo `FAILED`). A genuinely tolerable
/// command can still opt out with an explicit `cmd || true` — that's no longer mandatory
/// boilerplate, just an occasional, deliberate choice.
pub(crate) fn run_hooks(cmds: &[String], cwd: &Path, env: &[(&str, &str)]) -> Result<(), String> {
    for cmd in cmds {
        println!("+ {cmd}");
        let mut c = Command::new("sh");
        // `-e`: abort the command at its first failing step (within-line fail-fast).
        c.arg("-e").arg("-c").arg(cmd).current_dir(cwd);
        for (k, v) in env {
            c.env(k, v);
        }
        match c.status() {
            Ok(s) if s.success() => {}
            Ok(s) => return Err(format!("hook `{cmd}` exited {}", s.code().unwrap_or(-1))),
            Err(e) => return Err(format!("hook `{cmd}` failed to start: {e}")),
        }
    }
    Ok(())
}

/// Build the `git …` argv (everything after the program name) for one repo's clone:
/// `git <git-flags> clone [--depth N] [--branch B] [--single-branch] --recurse-submodules
/// <clone-flags> <repo clone-flags> <url> <dir>`.
///
/// `--branch` and `--single-branch` are **independent**: a plain `branch = "B"` checks
/// out `B` from a FULL clone (all branches fetched), while `single-branch = true` adds
/// `--single-branch` — paired with `branch` it fetches only `B`; on its own (no
/// `branch`) it clones only the remote's default branch, exactly as bare `git clone
/// --single-branch` does.
fn clone_args(conf: &CloneConf, r: &Repo, url: &str, dir_s: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    args.extend(conf.git_flags.iter().cloned());
    args.push("clone".into());
    if let Some(d) = r.depth {
        args.push("--depth".into());
        args.push(d.to_string());
    }
    if let Some(b) = &r.branch {
        args.push("--branch".into());
        args.push(b.clone());
    }
    if r.single_branch {
        args.push("--single-branch".into());
    }
    args.push("--recurse-submodules".into());
    args.extend(conf.clone_flags.iter().cloned());
    args.extend(r.clone_flags.iter().cloned());
    args.push(url.to_string());
    args.push(dir_s.to_string());
    args
}

/// Clone every repo in `conf`, printing each step in order. Returns a report per
/// repo (for the aggregate exit code).
pub fn clone_all<G: Git>(git: &G, conf: &CloneConf, opts: &Opts) -> Vec<CloneReport> {
    conf.repo
        .iter()
        .map(|r| {
            let name = r.name();
            let dir_s = expand_path(&r.dir, |k| std::env::var(k).ok());
            let dir = PathBuf::from(&dir_s);
            // Per-repo namespace overrides the global one; `clone_cmd` validates this
            // up front, so `None` here is a defensive backstop, not a normal path.
            let ns = match conf.namespace_for(r) {
                Some(n) => n.to_string(),
                None => {
                    let e = format!("no namespace for {}", r.dir);
                    println!("FAILED   {name:<28} {e}");
                    return CloneReport {
                        name,
                        dir,
                        outcome: Outcome::Failed(e),
                        command: String::new(),
                    };
                }
            };
            let url = format!("{}:{}/{}.git", conf.host, ns, name);

            let args = clone_args(conf, r, &url, &dir_s);
            let command = format!("git {}", args.join(" "));

            let mk = |outcome| CloneReport {
                name: name.clone(),
                dir: dir.clone(),
                outcome,
                command: command.clone(),
            };

            if dir.join(".git").exists() {
                println!("+ {command}");
                println!("skipped  {name:<28} {dir_s} (exists)");
                return mk(Outcome::Skipped);
            }

            let env = [
                ("GKIT_REPO", name.as_str()),
                ("GKIT_DIR", dir_s.as_str()),
                ("GKIT_URL", url.as_str()),
                ("GKIT_HOST", conf.host.as_str()),
                ("GKIT_NAMESPACE", ns.as_str()),
                ("GKIT_USER_NAME", opts.user_name.as_deref().unwrap_or("")),
                ("GKIT_USER_EMAIL", opts.user_email.as_deref().unwrap_or("")),
            ];

            // 1+2: pre-clone hooks (cwd = parent of target; create it first)
            let parent = dir.parent().unwrap_or(Path::new("."));
            let _ = std::fs::create_dir_all(parent);
            let pre: Vec<String> = conf
                .pre_clone
                .0
                .iter()
                .chain(r.pre_clone.0.iter())
                .cloned()
                .collect();
            if let Err(e) = run_hooks(&pre, parent, &env) {
                println!("FAILED   {name:<28} {e}");
                return mk(Outcome::Failed(e));
            }

            // 3: clone (printed; output captured)
            println!("+ {command}");
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = git.run(Path::new("."), &refs);
            if !out.success {
                let e = out.stderr.trim().to_string();
                println!("FAILED   {name:<28} {}", e.lines().next().unwrap_or(""));
                return mk(Outcome::Failed(e));
            }

            // 4: built-ins. Identity first (printed; values are explicit user input)
            // so post-clone hooks and direnv see it; a failure fails the repo.
            let identity: Vec<(&str, &str)> = [
                ("user.name", opts.user_name.as_deref()),
                ("user.email", opts.user_email.as_deref()),
            ]
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect();
            // 4a: the superproject (args passed straight to git — no shell).
            for (key, val) in &identity {
                println!("+ git config {key} {val}");
                let out = git.run(&dir, &["config", key, val]);
                if !out.success {
                    let e = format!("git config {key} failed: {}", out.stderr.trim());
                    println!("FAILED   {name:<28} {e}");
                    return mk(Outcome::Failed(e));
                }
            }
            // 4a': stamp gkit.conf (absolute conf path) on the superproject so
            // `gkit stamp` (no arg, run inside this repo) can resolve its conf later.
            if let Some(cp) = opts.conf_path.as_deref() {
                println!("+ git config gkit.conf {cp}");
                let out = git.run(&dir, &["config", "gkit.conf", cp]);
                if !out.success {
                    let e = format!("git config gkit.conf failed: {}", out.stderr.trim());
                    println!("FAILED   {name:<28} {e}");
                    return mk(Outcome::Failed(e));
                }
            }
            // 4b: the same identity into every submodule (recursive) so commits there
            // use it too — a submodule is its own repo with its own config. Runs via
            // `sh -c`, so the values are single-quoted.
            if let Some(body) =
                submodule_identity_cmd(opts.user_name.as_deref(), opts.user_email.as_deref())
            {
                println!("+ git submodule foreach --recursive {body}");
                let out = git.run(
                    &dir,
                    &["submodule", "foreach", "--recursive", body.as_str()],
                );
                if !out.success {
                    let e = format!("submodule identity failed: {}", out.stderr.trim());
                    println!("FAILED   {name:<28} {e}");
                    return mk(Outcome::Failed(e));
                }
            }
            // remaining built-ins (captured)
            if opts.submodule_branch {
                let _ = git.run(
                    &dir,
                    &["submodule", "foreach", "--recursive", SUBMODULE_SWITCH],
                );
            }
            if opts.direnv && dir.join(".envrc").exists() {
                let _ = Command::new("direnv").arg("allow").arg(&dir).output(); // trust-only, no eval
            }

            // 5+6: post-clone hooks (cwd = the cloned repo)
            let post: Vec<String> = conf
                .post_clone
                .0
                .iter()
                .chain(r.post_clone.0.iter())
                .cloned()
                .collect();
            if let Err(e) = run_hooks(&post, &dir, &env) {
                println!("FAILED   {name:<28} {e}");
                return mk(Outcome::Failed(e));
            }

            println!("cloned   {name:<28} {dir_s}");
            mk(Outcome::Cloned)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{sh_squote, submodule_identity_cmd};
    use crate::conf;

    #[test]
    fn submodule_identity_cmd_quotes_and_skips() {
        // both fields → two `git config`s, single-quoted, joined with `; `
        assert_eq!(
            submodule_identity_cmd(Some("Jane Dev"), Some("jane@acme.com")).as_deref(),
            Some("git config user.name 'Jane Dev'; git config user.email 'jane@acme.com'")
        );
        // only one field set → just that one
        assert_eq!(
            submodule_identity_cmd(Some("Jane"), None).as_deref(),
            Some("git config user.name 'Jane'")
        );
        // neither → None (caller skips the recursion)
        assert_eq!(submodule_identity_cmd(None, None), None);
        // an embedded single quote is escaped so `sh` can't break out
        assert_eq!(
            submodule_identity_cmd(Some("O'Brien"), None).as_deref(),
            Some(r"git config user.name 'O'\''Brien'")
        );
        assert_eq!(sh_squote("a b"), "'a b'");
    }

    #[test]
    fn insteadof_pair_is_namespace_scoped() {
        // bitbucket client
        assert_eq!(
            super::insteadof_pair("tlbb", "bitbucket.org", "codogenics"),
            (
                "url.tlbb:codogenics/.insteadOf".to_string(),
                "git@bitbucket.org:codogenics/".to_string()
            )
        );
        // gitlab subgroup namespace keeps its slash
        assert_eq!(
            super::insteadof_pair("ctl", "gitlab.com", "grp/sub").1,
            "git@gitlab.com:grp/sub/"
        );
    }

    #[test]
    fn distinct_namespaces_dedups_in_order() {
        let c = conf::parse(
            "host=\"h\"\nnamespace=\"glob\"\n\
             [[repo]]\ndir=\"$H/a\"\n\
             [[repo]]\ndir=\"$H/b\"\nnamespace=\"bob\"\n\
             [[repo]]\ndir=\"$H/c\"\n",
        )
        .unwrap();
        // glob (a), bob (b override), glob again (c) → [glob, bob], deduped, in order
        assert_eq!(super::distinct_namespaces(&c), vec!["glob", "bob"]);
    }

    #[test]
    fn opts_default_has_no_conf_path() {
        // gkit.conf is opt-in: the default (used by tests / non-clone callers)
        // leaves it unstamped.
        assert_eq!(super::Opts::default().conf_path, None);
    }

    #[test]
    fn builds_expected_url_shape() {
        let c = conf::parse("host = \"tlbb\"\nnamespace = \"example-org\"\n[[repo]]\ndir = \"$HOME/x/cosp\"\ndepth = 1\n").unwrap();
        assert_eq!(c.repo[0].name(), "cosp");
        assert_eq!(c.repo[0].depth, Some(1));
        let ns = c.namespace_for(&c.repo[0]).unwrap();
        let url = format!("{}:{}/{}.git", c.host, ns, c.repo[0].name());
        assert_eq!(url, "tlbb:example-org/cosp.git");
    }

    #[test]
    fn branch_is_full_clone_by_default() {
        // a plain `branch` checks out that branch WITHOUT --single-branch (full clone)
        let c = conf::parse(
            "host=\"tlbb\"\nnamespace=\"codogenics\"\n\
             [[repo]]\ndir=\"$HOME/scratch-spark\"\nname=\"spark4beginners\"\n\
             branch=\"SCB-543-spark-scala-chapter2\"\n",
        )
        .unwrap();
        let args = super::clone_args(
            &c,
            &c.repo[0],
            "tlbb:codogenics/spark4beginners.git",
            "/h/s",
        );
        assert_eq!(
            args,
            [
                "clone",
                "--branch",
                "SCB-543-spark-scala-chapter2",
                "--recurse-submodules",
                "tlbb:codogenics/spark4beginners.git",
                "/h/s",
            ]
        );
        assert!(!args.iter().any(|a| a == "--single-branch"));
    }

    #[test]
    fn single_branch_true_adds_flag() {
        // branch + single-branch=true → --branch B --single-branch (the old behavior)
        let c = conf::parse(
            "host=\"h\"\nnamespace=\"o\"\n\
             [[repo]]\ndir=\"$H/r\"\nbranch=\"dev\"\nsingle-branch=true\n",
        )
        .unwrap();
        let args = super::clone_args(&c, &c.repo[0], "h:o/r.git", "/h/r");
        assert_eq!(
            args,
            [
                "clone",
                "--branch",
                "dev",
                "--single-branch",
                "--recurse-submodules",
                "h:o/r.git",
                "/h/r"
            ]
        );
    }

    #[test]
    fn single_branch_without_branch_clones_default_only() {
        // single-branch=true alone → bare --single-branch (remote's default branch only)
        let c = conf::parse(
            "host=\"h\"\nnamespace=\"o\"\n[[repo]]\ndir=\"$H/r\"\nsingle-branch=true\n",
        )
        .unwrap();
        let args = super::clone_args(&c, &c.repo[0], "h:o/r.git", "/h/r");
        assert_eq!(
            args,
            [
                "clone",
                "--single-branch",
                "--recurse-submodules",
                "h:o/r.git",
                "/h/r"
            ]
        );
        assert!(!args.iter().any(|a| a == "--branch"));
    }

    #[test]
    fn per_repo_namespace_drives_url() {
        let c = conf::parse("host=\"gh\"\n[[repo]]\ndir=\"$HOME/x/foo\"\nnamespace=\"alice\"\n")
            .unwrap();
        let ns = c.namespace_for(&c.repo[0]).unwrap();
        let url = format!("{}:{}/{}.git", c.host, ns, c.repo[0].name());
        assert_eq!(url, "gh:alice/foo.git");
    }
}
