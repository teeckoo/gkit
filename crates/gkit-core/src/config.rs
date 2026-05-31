//! Resolve a repo's base/integration branch — the single branch gkit treats as
//! "the trunk" for the correct-branch check. Replaces the zsh's hardcoded
//! `dev|main|master`. Resolution order:
//!   1. explicit CLI `--base-branch`
//!   2. per-repo `git config gkit.baseBranch`
//!   3. a remote-tracking branch — `origin/main`, else `origin/master`
//!   4. otherwise **unresolved**: a base couldn't be determined (e.g. a
//!      single-branch clone of a feature branch). The correct-branch check then
//!      fails rather than silently passing against the wrong base.

use crate::git::Git;
use std::collections::HashSet;
use std::path::Path;

/// Where a resolved base branch came from — surfaced by `logoff --verbose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseSource {
    /// Explicit CLI `--base-branch`.
    Flag,
    /// `git config gkit.baseBranch`.
    Config,
    /// Derived from a remote-tracking branch (`origin/main`, else `origin/master`).
    Remote,
    /// Could not be determined from any source.
    Unresolved,
}

/// A base branch plus where it was resolved from. `name` is `None` only when
/// `source` is [`BaseSource::Unresolved`].
#[derive(Debug, Clone)]
pub struct ResolvedBase {
    pub name: Option<String>,
    pub source: BaseSource,
}

impl ResolvedBase {
    fn flag(name: &str) -> Self {
        Self {
            name: Some(name.to_string()),
            source: BaseSource::Flag,
        }
    }
    fn config(name: &str) -> Self {
        Self {
            name: Some(name.to_string()),
            source: BaseSource::Config,
        }
    }
    fn remote(name: &str) -> Self {
        Self {
            name: Some(name.to_string()),
            source: BaseSource::Remote,
        }
    }
    /// The unresolved sentinel: no base, fails the correct-branch check.
    pub fn unresolved() -> Self {
        Self {
            name: None,
            source: BaseSource::Unresolved,
        }
    }

    /// Human-readable "branch (how it was derived)" for `logoff --verbose`.
    pub fn describe(&self) -> String {
        match (&self.name, self.source) {
            (Some(b), BaseSource::Flag) => format!("{b} (from --base-branch)"),
            (Some(b), BaseSource::Config) => format!("{b} (from git config gkit.baseBranch)"),
            (Some(b), BaseSource::Remote) => format!("{b} (derived from remote origin/{b})"),
            _ => "UNRESOLVED — gkit.baseBranch unset and no origin/main or origin/master \
                  (correct-branch can't be checked)"
                .to_string(),
        }
    }
}

/// Resolve the base branch for the `logoff` correct-branch check (see module docs).
pub fn resolve_base(git: &dyn Git, dir: &Path, cli_override: Option<&str>) -> ResolvedBase {
    if let Some(b) = cli_override {
        let b = b.trim();
        if !b.is_empty() {
            return ResolvedBase::flag(b);
        }
    }
    let cfg = git.run(dir, &["config", "--get", "gkit.baseBranch"]);
    if cfg.success && !cfg.trimmed().is_empty() {
        return ResolvedBase::config(cfg.trimmed());
    }
    // Derive from remote-tracking branches: main first, then master. A single-branch
    // clone of a feature branch has neither -> unresolved (correct-branch fails).
    let remotes: HashSet<String> = git
        .run(
            dir,
            &[
                "for-each-ref",
                "--format=%(refname:short)",
                "refs/remotes/origin/*",
            ],
        )
        .stdout
        .lines()
        .filter_map(|l| l.trim().strip_prefix("origin/").map(str::to_string))
        .collect();
    for cand in ["main", "master"] {
        if remotes.contains(cand) {
            return ResolvedBase::remote(cand);
        }
    }
    ResolvedBase::unresolved()
}

/// Read `gkit.solo` (a bool) for a repo. Default `false` (team workflow) when
/// unset or unparsable. When `true`, the correct-branch check additionally flags
/// sitting on an integration branch while feature branches exist on the remote —
/// meaningful for a solo developer where every remote branch is their own.
/// Honors git's config layering: `--global` for a personal default, repo config
/// to override. Stamped by `gkit clone` from the conf's `solo` field.
pub fn resolve_solo(git: &dyn Git, dir: &Path) -> bool {
    let o = git.run(dir, &["config", "--get", "--bool", "gkit.solo"]);
    o.success && o.trimmed() == "true"
}

/// Current branch, or `None` if HEAD is detached (no symbolic ref).
pub fn current_branch_opt(git: &dyn Git, dir: &Path) -> Option<String> {
    let o = git.run(dir, &["symbolic-ref", "--short", "HEAD"]);
    if o.success {
        Some(o.trimmed().to_string())
    } else {
        None
    }
}

/// Resolve the base branch to *switch to* (for stmb). Unlike [`resolve_base`]
/// this never falls back to HEAD (HEAD is the feature branch here): CLI override →
/// `gkit.baseBranch` → `origin/HEAD` default branch. `None` if undeterminable.
pub fn resolve_switch_base(
    git: &dyn Git,
    dir: &Path,
    cli_override: Option<&str>,
) -> Option<String> {
    if let Some(b) = cli_override {
        if !b.trim().is_empty() {
            return Some(b.trim().to_string());
        }
    }
    let cfg = git.run(dir, &["config", "--get", "gkit.baseBranch"]);
    if cfg.success && !cfg.trimmed().is_empty() {
        return Some(cfg.trimmed().to_string());
    }
    let head = git.run(
        dir,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    );
    if head.success {
        return head.trimmed().strip_prefix("origin/").map(str::to_string);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::FakeGit;
    use std::path::Path;

    fn d() -> &'static Path {
        Path::new("/x")
    }

    /// `for-each-ref refs/remotes/origin/*` stub listing the given remote branches.
    fn with_remotes(g: FakeGit, branches: &[&str]) -> FakeGit {
        let listing = branches
            .iter()
            .map(|b| format!("origin/{b}"))
            .collect::<Vec<_>>()
            .join("\n");
        g.ok(
            "for-each-ref --format=%(refname:short) refs/remotes/origin/*",
            &listing,
        )
    }

    #[test]
    fn cli_override_wins() {
        let g = FakeGit::new().ok("config --get gkit.baseBranch", "dev");
        let r = resolve_base(&g, d(), Some("main"));
        assert_eq!(r.name.as_deref(), Some("main"));
        assert_eq!(r.source, BaseSource::Flag);
    }

    #[test]
    fn falls_back_to_git_config() {
        let g = FakeGit::new().ok("config --get gkit.baseBranch", "dev");
        let r = resolve_base(&g, d(), None);
        assert_eq!(r.name.as_deref(), Some("dev"));
        assert_eq!(r.source, BaseSource::Config);
    }

    #[test]
    fn derives_main_from_remote_when_config_unset() {
        let g = with_remotes(
            FakeGit::new().fail("config --get gkit.baseBranch"),
            &["feature-x", "main", "master"],
        );
        let r = resolve_base(&g, d(), None);
        // main wins over master.
        assert_eq!(r.name.as_deref(), Some("main"));
        assert_eq!(r.source, BaseSource::Remote);
    }

    #[test]
    fn derives_master_when_no_main() {
        let g = with_remotes(
            FakeGit::new().fail("config --get gkit.baseBranch"),
            &["master", "feature-y"],
        );
        let r = resolve_base(&g, d(), None);
        assert_eq!(r.name.as_deref(), Some("master"));
        assert_eq!(r.source, BaseSource::Remote);
    }

    #[test]
    fn unresolved_when_no_config_and_no_main_master() {
        // e.g. a single-branch clone of a feature branch.
        let g = with_remotes(
            FakeGit::new().fail("config --get gkit.baseBranch"),
            &["feature-only"],
        );
        let r = resolve_base(&g, d(), None);
        assert_eq!(r.name, None);
        assert_eq!(r.source, BaseSource::Unresolved);
    }

    #[test]
    fn resolve_solo_defaults_false_and_reads_bool() {
        // unset / failing config -> false
        assert!(!resolve_solo(
            &FakeGit::new().fail("config --get --bool gkit.solo"),
            d()
        ));
        // explicit true / false
        assert!(resolve_solo(
            &FakeGit::new().ok("config --get --bool gkit.solo", "true"),
            d()
        ));
        assert!(!resolve_solo(
            &FakeGit::new().ok("config --get --bool gkit.solo", "false"),
            d()
        ));
    }

    #[test]
    fn current_branch_opt_detects_detached() {
        let on = FakeGit::new().ok("symbolic-ref --short HEAD", "feat");
        assert_eq!(current_branch_opt(&on, d()), Some("feat".into()));
        let detached = FakeGit::new().fail("symbolic-ref --short HEAD");
        assert_eq!(current_branch_opt(&detached, d()), None);
    }

    #[test]
    fn switch_base_uses_origin_head_not_current() {
        // config unset -> use origin/HEAD default, NOT the (feature) HEAD
        let g = FakeGit::new().fail("config --get gkit.baseBranch").ok(
            "symbolic-ref --short refs/remotes/origin/HEAD",
            "origin/dev",
        );
        assert_eq!(resolve_switch_base(&g, d(), None), Some("dev".into()));
        // override still wins
        assert_eq!(
            resolve_switch_base(&g, d(), Some("main")),
            Some("main".into())
        );
    }
}
