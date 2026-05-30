//! Resolve a repo's base/integration branch — the single branch gkit treats as
//! "the trunk" for the correct-branch check. Replaces the zsh's hardcoded
//! `dev|main|master`. Resolution order:
//!   1. explicit CLI `--base-branch`
//!   2. per-repo `git config gkit.baseBranch`
//!   3. current HEAD (auto-detect)

use crate::git::Git;
use std::path::Path;

pub fn resolve_base_branch(git: &dyn Git, dir: &Path, cli_override: Option<&str>) -> String {
    if let Some(b) = cli_override {
        if !b.trim().is_empty() {
            return b.trim().to_string();
        }
    }
    let cfg = git.run(dir, &["config", "--get", "gkit.baseBranch"]);
    if cfg.success && !cfg.trimmed().is_empty() {
        return cfg.trimmed().to_string();
    }
    crate::checks::current_branch(git, dir)
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

/// Resolve the base branch to *switch to* (for stmb). Unlike [`resolve_base_branch`]
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

    #[test]
    fn cli_override_wins() {
        let g = FakeGit::new().ok("config --get gkit.baseBranch", "dev");
        assert_eq!(resolve_base_branch(&g, d(), Some("main")), "main");
    }

    #[test]
    fn falls_back_to_git_config() {
        let g = FakeGit::new().ok("config --get gkit.baseBranch", "dev");
        assert_eq!(resolve_base_branch(&g, d(), None), "dev");
    }

    #[test]
    fn falls_back_to_head_when_config_unset() {
        let g = FakeGit::new()
            .fail("config --get gkit.baseBranch")
            .ok("rev-parse --abbrev-ref HEAD", "trunk");
        assert_eq!(resolve_base_branch(&g, d(), None), "trunk");
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
