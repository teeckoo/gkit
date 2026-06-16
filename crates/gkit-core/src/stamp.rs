//! Re-apply a clone conf's `post-clone` hooks over an **existing** tree, without
//! cloning. `gkit clone` runs `post-clone` once, right after cloning; `gkit stamp`
//! re-runs the same hooks on repos that are already on disk.
//!
//! Why this exists: `post-clone` is where teams stamp per-repo git config the gate
//! reads — `git config gkit.baseBranch …`, `git config gkit.solo …`, usually with a
//! `git submodule foreach --recursive '…'` so submodules get it too. But that runs
//! only at clone time, so a submodule **added later** (e.g. on a feature branch that
//! pins a new submodule) is never stamped: it comes up with no `gkit.baseBranch`
//! (base falls back to `origin/main`/`master`) and no `gkit.solo` (the team rule).
//! `gkit stamp <conf>` re-runs the conf's `post-clone` over the existing repos so
//! those values converge — idempotently, since they're `git config` writes.
//!
//! `stamp` does **not** clone, fetch, or run `pre-clone`/built-ins. Identity
//! (`--user-name`/`--user-email`) is a `clone` concern, so the `$GKIT_USER_NAME`/
//! `$GKIT_USER_EMAIL` hook env is empty here; the other `$GKIT_*` vars mirror clone.

use crate::clone::run_hooks;
use crate::conf::{expand_path, CloneConf, Repo};
use crate::config;
use crate::git::Git;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `post-clone` hooks ran successfully over the repo.
    Stamped,
    /// Nothing to do (the conf has no `post-clone` for this repo).
    Skipped,
    /// The dir is missing / not a git repo, or a hook failed.
    Failed(String),
}

#[derive(Debug)]
pub struct StampReport {
    pub name: String,
    pub dir: PathBuf,
    pub outcome: Outcome,
}

/// The effective `post-clone` hooks for a repo: the global ones, then the repo's
/// own (same order `clone` runs them). Shared with the CLI's dry-run plan.
pub fn effective_post_clone(conf: &CloneConf, repo: &Repo) -> Vec<String> {
    conf.post_clone
        .0
        .iter()
        .chain(repo.post_clone.0.iter())
        .cloned()
        .collect()
}

/// Is `dir` inside a git work tree? (Same probe as the gate uses for the root.)
fn is_git_repo(git: &dyn Git, dir: &Path) -> bool {
    let r = git.run(dir, &["rev-parse", "--is-inside-work-tree"]);
    r.success && r.trimmed() == "true"
}

/// Re-run each repo's effective `post-clone` over its existing dir, printing each
/// step in order. Returns a report per repo (for the aggregate exit code).
///
/// Per repo: a missing dir or non-repo **fails** (we want to know — never a silent
/// skip); a repo with no `post-clone` is skipped; otherwise the hooks run with the
/// same `$GKIT_*` env `clone` sets (identity vars empty — see module docs).
pub fn stamp_all<G: Git>(git: &G, conf: &CloneConf) -> Vec<StampReport> {
    conf.repo
        .iter()
        .map(|r| {
            let name = r.name();
            let dir_s = expand_path(&r.dir, |k| std::env::var(k).ok());
            let dir = PathBuf::from(&dir_s);
            let mk = |outcome| StampReport {
                name: name.clone(),
                dir: dir.clone(),
                outcome,
            };

            // Must be an existing git repo — otherwise there's nothing to stamp and a
            // silent pass would hide a missing/foreign dir.
            if !dir.exists() {
                let e = "no such directory".to_string();
                println!("FAILED   {name:<28} {dir_s} ({e})");
                return mk(Outcome::Failed(e));
            }
            if !is_git_repo(git, &dir) {
                let e = "not a git repository".to_string();
                println!("FAILED   {name:<28} {dir_s} ({e})");
                return mk(Outcome::Failed(e));
            }

            let post = effective_post_clone(conf, r);
            if post.is_empty() {
                println!("skipped  {name:<28} {dir_s} (no post-clone hooks)");
                return mk(Outcome::Skipped);
            }

            // The `$GKIT_*` hook env, mirroring clone. Namespace is only needed to
            // build `$GKIT_URL`; if it can't resolve (validated away by the CLI) we
            // still stamp — the config hooks don't depend on it.
            let ns = conf.namespace_for(r).unwrap_or_default().to_string();
            let url = format!("{}:{}/{}.git", conf.host, ns, name);
            let env = [
                ("GKIT_REPO", name.as_str()),
                ("GKIT_DIR", dir_s.as_str()),
                ("GKIT_URL", url.as_str()),
                ("GKIT_HOST", conf.host.as_str()),
                ("GKIT_NAMESPACE", ns.as_str()),
                ("GKIT_USER_NAME", ""),
                ("GKIT_USER_EMAIL", ""),
            ];

            if let Err(e) = run_hooks(&post, &dir, &env) {
                println!("FAILED   {name:<28} {e}");
                return mk(Outcome::Failed(e));
            }
            println!("stamped  {name:<28} {dir_s}");
            mk(Outcome::Stamped)
        })
        .collect()
}

/// Find the `[[repo]]` whose `expand_path(dir)` canonicalizes to `repo_dir`.
/// Matching on canonicalized paths absorbs `$VAR`/`~` expansion, symlinks, and
/// trailing-slash differences. `repo_dir` must already be canonicalized.
pub fn match_repo<'a>(conf: &'a CloneConf, repo_dir: &Path) -> Option<&'a Repo> {
    conf.repo.iter().find(|r| {
        let d = expand_path(&r.dir, |k| std::env::var(k).ok());
        std::fs::canonicalize(&d)
            .map(|c| c == *repo_dir)
            .unwrap_or(false)
    })
}

/// What a repo-mode stamp resolved: the conf it came from, the hooks to run, whether
/// a `[[repo]]` matched (false → global `post-clone` only), and the `$GKIT_*` env
/// bits (empty when unmatched).
pub struct RepoPlan {
    pub conf_path: String,
    pub hooks: Vec<String>,
    pub matched: bool,
    pub env_repo: String,
    pub env_url: String,
    pub env_host: String,
    pub env_namespace: String,
}

/// Resolve a repo's own `gkit.conf`, parse it, and compute the hooks to run in this
/// repo (the matched `[[repo]]`'s effective post-clone, else the conf's global
/// post-clone). `Err` when `gkit.conf` is unset (actionable) or the conf can't be
/// read/parsed. `repo_dir` must already be canonicalized by the caller.
pub fn plan_repo<G: Git>(git: &G, repo_dir: &Path) -> Result<RepoPlan, String> {
    let conf_path = config::resolve_conf(git, repo_dir).ok_or_else(|| {
        format!(
            "gkit.conf not set in {}; run `gkit stamp --conf <conf>` once to back-fill, or pass the conf",
            repo_dir.display()
        )
    })?;
    let text = std::fs::read_to_string(&conf_path)
        .map_err(|e| format!("cannot read gkit.conf `{conf_path}`: {e}"))?;
    let cfg = crate::conf::parse(&text).map_err(|e| format!("{conf_path}: {e}"))?;

    let basename = repo_dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo_dir.display().to_string());
    match match_repo(&cfg, repo_dir) {
        Some(r) => {
            let host = cfg.host.clone();
            let ns = cfg.namespace_for(r).unwrap_or_default().to_string();
            let repo = r.name();
            let url = format!("{host}:{ns}/{repo}.git");
            Ok(RepoPlan {
                conf_path,
                hooks: effective_post_clone(&cfg, r),
                matched: true,
                env_repo: repo,
                env_url: url,
                env_host: host,
                env_namespace: ns,
            })
        }
        None => Ok(RepoPlan {
            conf_path,
            hooks: cfg.post_clone.0.clone(),
            matched: false,
            env_repo: basename,
            env_url: String::new(),
            env_host: String::new(),
            env_namespace: String::new(),
        }),
    }
}

/// Repo-mode stamp: resolve the repo's own `gkit.conf` ([`plan_repo`]) and run its
/// hooks in `repo_dir`, with the same `$GKIT_*` env shape `clone`/`stamp_all` use
/// (identity vars empty — identity is a `clone`/`fixsub` concern). `repo_dir` must
/// already be canonicalized.
pub fn stamp_repo<G: Git>(git: &G, repo_dir: &Path) -> StampReport {
    let dir_s = repo_dir.display().to_string();
    let basename = repo_dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir_s.clone());
    let mk = |name: &str, outcome| StampReport {
        name: name.to_string(),
        dir: repo_dir.to_path_buf(),
        outcome,
    };

    if !is_git_repo(git, repo_dir) {
        let e = "not a git repository".to_string();
        println!("FAILED   {basename:<28} {dir_s} ({e})");
        return mk(&basename, Outcome::Failed(e));
    }
    let plan = match plan_repo(git, repo_dir) {
        Ok(p) => p,
        Err(e) => {
            println!("FAILED   {basename:<28} {e}");
            return mk(&basename, Outcome::Failed(e));
        }
    };
    if !plan.matched {
        println!(
            "note: {dir_s} not listed in {} — running global post-clone only",
            plan.conf_path
        );
    }
    if plan.hooks.is_empty() {
        println!(
            "skipped  {:<28} {dir_s} (no post-clone hooks)",
            plan.env_repo
        );
        return mk(&plan.env_repo, Outcome::Skipped);
    }
    let env = [
        ("GKIT_REPO", plan.env_repo.as_str()),
        ("GKIT_DIR", dir_s.as_str()),
        ("GKIT_URL", plan.env_url.as_str()),
        ("GKIT_HOST", plan.env_host.as_str()),
        ("GKIT_NAMESPACE", plan.env_namespace.as_str()),
        ("GKIT_USER_NAME", ""),
        ("GKIT_USER_EMAIL", ""),
    ];
    if let Err(e) = run_hooks(&plan.hooks, repo_dir, &env) {
        println!("FAILED   {:<28} {e}", plan.env_repo);
        return mk(&plan.env_repo, Outcome::Failed(e));
    }
    println!("stamped  {:<28} {dir_s}", plan.env_repo);
    mk(&plan.env_repo, Outcome::Stamped)
}

/// Conf-mode back-fill: set `gkit.conf` (the absolute conf path) on each `[[repo]]`
/// that is a git repo and lacks it. **Never overwrites** an existing value (one-way
/// migration; idempotent). Prints each `git config` it runs. Non-fatal — a set
/// failure is warned but doesn't fail the run (the post-clone stamp is the job).
pub fn backfill_conf<G: Git>(git: &G, conf: &CloneConf, abs_conf_path: &str) {
    for r in &conf.repo {
        let dir_s = expand_path(&r.dir, |k| std::env::var(k).ok());
        let dir = PathBuf::from(&dir_s);
        if !dir.exists() || !is_git_repo(git, &dir) {
            continue;
        }
        if config::resolve_conf(git, &dir).is_none() {
            println!("+ git config gkit.conf {abs_conf_path}  ({dir_s})");
            let out = git.run(&dir, &["config", "gkit.conf", abs_conf_path]);
            if !out.success {
                println!(
                    "warning: could not set gkit.conf in {dir_s}: {}",
                    out.stderr.trim()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conf::{Hooks, Repo};
    use crate::git::test_support::FakeGit;

    fn repo(dir: &str, post: &[&str]) -> Repo {
        Repo {
            dir: dir.to_string(),
            namespace: None,
            name: None,
            depth: None,
            branch: None,
            single_branch: false,
            clone_flags: vec![],
            pre_clone: Hooks(vec![]),
            post_clone: Hooks(post.iter().map(|s| s.to_string()).collect()),
        }
    }

    fn conf(global_post: &[&str], repos: Vec<Repo>) -> CloneConf {
        CloneConf {
            host: "h".into(),
            namespace: Some("ns".into()),
            git_flags: vec![],
            clone_flags: vec![],
            pre_clone: Hooks(vec![]),
            post_clone: Hooks(global_post.iter().map(|s| s.to_string()).collect()),
            repo: repos,
        }
    }

    #[test]
    fn effective_post_clone_chains_global_then_repo() {
        let c = conf(
            &["git config gkit.solo true"],
            vec![repo("/x", &["git config gkit.baseBranch dev"])],
        );
        assert_eq!(
            effective_post_clone(&c, &c.repo[0]),
            [
                "git config gkit.solo true",
                "git config gkit.baseBranch dev"
            ]
        );
    }

    #[test]
    fn match_repo_by_canonical_dir() {
        // A [[repo]] dir matches when it canonicalizes to the queried repo path; a
        // dir not listed in the conf returns None.
        let base = std::env::temp_dir().join(format!("gkit-match-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let a = base.join("a");
        let b = base.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let c = conf(&[], vec![repo(a.to_str().unwrap(), &[])]);
        let a_canon = std::fs::canonicalize(&a).unwrap();
        let b_canon = std::fs::canonicalize(&b).unwrap();
        assert!(match_repo(&c, &a_canon).is_some(), "listed dir matches");
        assert!(match_repo(&c, &b_canon).is_none(), "unlisted dir → None");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_dir_is_failed() {
        // A dir that doesn't exist must FAIL (not silently pass) — no FakeGit response
        // is needed because `dir.exists()` short-circuits before any git call.
        let c = conf(
            &["git config gkit.solo true"],
            vec![repo("/no/such/gkit-stamp-xyz", &[])],
        );
        let reports = stamp_all(&FakeGit::new(), &c);
        assert_eq!(reports.len(), 1);
        assert!(matches!(reports[0].outcome, Outcome::Failed(ref e) if e.contains("no such")));
    }
}
