//! `stmb` — "switch to main branch": finish a feature branch by returning to the
//! base/integration branch, updating it, and deleting the (merged) feature branch.
//!
//! Port of the zsh `stmb`, with **proper, safe branch handling**: base is resolved
//! (not hardcoded `dev`), and the feature branch is **safe-deleted** (`git branch
//! -d`, which refuses an unmerged branch) instead of the original force `-D` — so
//! you can't silently lose unpushed/unmerged work. Force is opt-in.

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
