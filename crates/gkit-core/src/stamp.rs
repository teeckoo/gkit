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
