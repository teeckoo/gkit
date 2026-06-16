//! `stmb` — "switch to main branch": finish a feature branch by returning to the
//! base/integration branch, updating it, and deleting the (merged) feature branch.
//!
//! Port of the zsh `stmb`, with **proper, safe branch handling**: base is resolved
//! (not hardcoded `dev`), and the feature branch is **deleted only when stmb has
//! positively verified it is merged into base** — by reachability (merge-commit /
//! fast-forward) *or* by patch-id equivalence (squash / rebase merge, where the
//! commit hash changed but the changes are already in base). When it can't verify,
//! it **refuses** and tells you to discard the branch yourself with `git branch -D`,
//! rather than offering a blunt force flag that trains people to always pass it.

use crate::git::Git;
use std::path::Path;

/// Why stmb will (or won't) delete a feature branch — the verdict behind the
/// human-readable line stmb prints before acting. Computed purely from repo state.
#[derive(Debug, PartialEq, Eq)]
pub enum MergeStatus {
    /// The feature tip is an ancestor of base — a normal merge-commit or
    /// fast-forward merge. Safe to delete; `git branch -d` agrees.
    Reachable,
    /// The tip is *not* reachable from base, but every commit on the branch has an
    /// equivalent patch already in base (squash- or rebase-merged). Safe to delete;
    /// `git branch -d` would wrongly refuse, so `-D` is used after this verdict.
    Content,
    /// `unique` commit(s) on the branch have no equivalent in base — genuine
    /// unmerged work. stmb refuses to delete it.
    Unmerged { unique: u64 },
    /// stmb could not determine the answer (a git error / unparseable output).
    /// **Fail-closed**: treated like unmerged — refuse, never delete vacuously.
    Unknown(String),
}

impl MergeStatus {
    /// True only when stmb has *verified* the branch is merged (safe to delete).
    pub fn is_merged(&self) -> bool {
        matches!(self, MergeStatus::Reachable | MergeStatus::Content)
    }

    /// A readable "why" for the given branch/base — the reason half of the line
    /// stmb prints before it deletes (or declines to delete) the branch.
    pub fn reason(&self, feature: &str, base: &str) -> String {
        match self {
            MergeStatus::Reachable => format!(
                "'{feature}' is fully merged into {base} (its commits are in {base}'s history)"
            ),
            MergeStatus::Content => format!(
                "'{feature}' has no commits missing from {base} — its changes are already in \
                 {base} (squash/rebase-merged)"
            ),
            MergeStatus::Unmerged { unique } => {
                format!("'{feature}' has {unique} commit(s) not present in {base} (by content)")
            }
            MergeStatus::Unknown(why) => {
                format!("could not verify whether '{feature}' is merged into {base}: {why}")
            }
        }
    }
}

/// Decide whether `feature` is merged into `base`. Reachability first (catches
/// merge-commit / fast-forward merges, where the tip lands in base's history),
/// then patch-id equivalence (catches squash / rebase merges, where the commit
/// hash changed but the diff is already in base). **Fail-closed**: any git error
/// yields [`MergeStatus::Unknown`], never a vacuous "merged".
pub fn merge_status(git: &dyn Git, dir: &Path, base: &str, feature: &str) -> MergeStatus {
    // 1. Reachability: is the feature tip an ancestor of base?
    if git
        .run(dir, &["merge-base", "--is-ancestor", feature, base])
        .success
    {
        return MergeStatus::Reachable;
    }
    // 2. Patch-id equivalence: count commits on `feature` that are NOT in `base`
    //    *by content*. `--cherry-pick` drops commit pairs with an equal patch-id;
    //    `--right-only` keeps just the `feature` side, so the count is the branch's
    //    genuinely-unique work. Zero ⇒ everything is already in base (squashed).
    let range = format!("{base}...{feature}");
    let out = git.run(
        dir,
        &[
            "rev-list",
            "--count",
            "--cherry-pick",
            "--right-only",
            &range,
        ],
    );
    if !out.success {
        return MergeStatus::Unknown(format!("git rev-list failed: {}", out.stderr.trim()));
    }
    match out.trimmed().parse::<u64>() {
        Ok(0) => MergeStatus::Content,
        Ok(unique) => MergeStatus::Unmerged { unique },
        Err(_) => MergeStatus::Unknown(format!("unparseable rev-list output: {:?}", out.trimmed())),
    }
}

/// The decided plan, computed purely from repo state.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub base: String,
    /// Feature branch to delete after switching; `None` when already on base.
    pub delete_feature: Option<String>,
}

/// Decide what `stmb` should do. `current` is the current branch (`None` = detached).
/// Refuses states that aren't safe to auto-handle.
pub fn plan(current: Option<&str>, base: &str, dirty: bool) -> Result<Plan, String> {
    if dirty {
        return Err("working tree has uncommitted changes — commit or stash before stmb".into());
    }
    match current {
        None => Err("detached HEAD — checkout a branch before stmb".into()),
        Some(cur) if cur == base => Ok(Plan {
            base: base.to_string(),
            delete_feature: None,
        }),
        Some(cur) => Ok(Plan {
            base: base.to_string(),
            delete_feature: Some(cur.to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::FakeGit;
    use std::path::Path;

    fn d() -> &'static Path {
        Path::new("r")
    }

    #[test]
    fn merge_status_reachable_when_ancestor() {
        let g = FakeGit::new().ok("merge-base --is-ancestor feat main", "");
        assert_eq!(
            merge_status(&g, d(), "main", "feat"),
            MergeStatus::Reachable
        );
    }

    #[test]
    fn merge_status_content_when_no_unique_patches() {
        // Not an ancestor (squash-merged), but rev-list finds 0 unique commits.
        let g = FakeGit::new()
            .fail("merge-base --is-ancestor feat main")
            .ok(
                "rev-list --count --cherry-pick --right-only main...feat",
                "0",
            );
        assert_eq!(merge_status(&g, d(), "main", "feat"), MergeStatus::Content);
    }

    #[test]
    fn merge_status_unmerged_counts_unique_commits() {
        let g = FakeGit::new()
            .fail("merge-base --is-ancestor feat main")
            .ok(
                "rev-list --count --cherry-pick --right-only main...feat",
                "2",
            );
        assert_eq!(
            merge_status(&g, d(), "main", "feat"),
            MergeStatus::Unmerged { unique: 2 }
        );
    }

    #[test]
    fn merge_status_unknown_is_fail_closed_on_git_error() {
        // rev-list errors -> Unknown (refuse), never a vacuous "merged".
        let g = FakeGit::new().fail("merge-base --is-ancestor feat main");
        let s = merge_status(&g, d(), "main", "feat");
        assert!(matches!(s, MergeStatus::Unknown(_)));
        assert!(!s.is_merged());
    }

    #[test]
    fn reason_is_readable_per_verdict() {
        assert!(MergeStatus::Reachable
            .reason("feat", "main")
            .contains("merged into main"));
        assert!(MergeStatus::Content
            .reason("feat", "main")
            .contains("squash/rebase-merged"));
        assert!(MergeStatus::Unmerged { unique: 3 }
            .reason("feat", "main")
            .contains("3 commit(s) not present in main"));
        assert!(MergeStatus::Unknown("boom".into())
            .reason("feat", "main")
            .contains("could not verify"));
    }

    #[test]
    fn refuses_dirty_tree() {
        assert!(plan(Some("feat"), "dev", true)
            .unwrap_err()
            .contains("uncommitted"));
    }

    #[test]
    fn refuses_detached() {
        assert!(plan(None, "dev", false).unwrap_err().contains("detached"));
    }

    #[test]
    fn on_base_deletes_nothing() {
        assert_eq!(
            plan(Some("dev"), "dev", false).unwrap(),
            Plan {
                base: "dev".into(),
                delete_feature: None
            }
        );
    }

    #[test]
    fn on_feature_deletes_it() {
        assert_eq!(
            plan(Some("feat-x"), "dev", false).unwrap(),
            Plan {
                base: "dev".into(),
                delete_feature: Some("feat-x".into())
            }
        );
    }
}
