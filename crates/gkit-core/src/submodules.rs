//! Submodule traversal + parallel evaluation with deterministic output order.
//!
//! Mirrors the zsh recursion (`gitCoreLib.sh` `isEverythingCheckedIn` →
//! `git submodule foreach`): each repo's submodules are checked before the repo
//! itself, so the emit order is **post-order DFS** (children first, superproject
//! last), siblings in submodule-config order. Checks run in parallel for speed,
//! but results are buffered into fixed slots so output never depends on which
//! thread finishes first.

use crate::checks::{self, RepoStatus};
use crate::git::Git;
use std::path::{Path, PathBuf};

/// One evaluated repo (or submodule).
pub struct Entry {
    pub path: PathBuf,
    pub status: RepoStatus,
}

/// Direct submodule paths (absolute) of `dir`, in `git submodule status` order.
/// Uninitialized submodules (status `-`) are skipped — nothing to check.
fn direct_submodules(git: &dyn Git, dir: &Path) -> Vec<PathBuf> {
    git.run(dir, &["submodule", "status"])
        .stdout
        .lines()
        .filter_map(|line| {
            let status = line.chars().next()?;
            if status == '-' {
                return None; // uninitialized
            }
            // Drop the 1-char status column; remainder is "<sha> <path> (<describe>)".
            let path = line[1..].split_whitespace().nth(1)?;
            Some(dir.join(path))
        })
        .collect()
}

/// All repos to check rooted at `root`, in post-order (submodules before parent,
/// `root` last).
/// Public: repos rooted at `root` in post-order DFS (submodules before parent,
/// `root` last). Reused by `stmb` to walk the same tree.
pub fn repo_paths(git: &dyn Git, root: &Path) -> Vec<PathBuf> {
    collect_repos(git, root)
}

/// Is `dir` inside a git work tree? (`git rev-parse --is-inside-work-tree`
/// prints `true` and exits 0). False for a missing dir or a plain directory.
fn is_work_tree(git: &dyn Git, dir: &Path) -> bool {
    let r = git.run(dir, &["rev-parse", "--is-inside-work-tree"]);
    r.success && r.trimmed() == "true"
}

fn collect_repos(git: &dyn Git, root: &Path) -> Vec<PathBuf> {
    fn visit(git: &dyn Git, dir: &Path, order: &mut Vec<PathBuf>) {
        for sub in direct_submodules(git, dir) {
            visit(git, &sub, order);
        }
        order.push(dir.to_path_buf());
    }
    let mut order = Vec::new();
    visit(git, root, &mut order);
    order
}

/// Evaluate `root` and all (recursive) submodules. Checks run in parallel; the
/// returned Vec is in the fixed post-order DFS order.
///
/// `base_override` (the CLI `--base-branch`) applies only to the root; each
/// submodule resolves its own base (`gkit.baseBranch`, then remote
/// `origin/main`/`origin/master`). Like the zsh, submodules are fetched before
/// checking (when `fetch`), the root is not.
pub fn evaluate_tree<G: Git + Sync>(
    git: &G,
    root: &Path,
    base_override: Option<&str>,
    fetch: bool,
) -> Vec<Entry> {
    // Guard the root: a non-repo (or missing) dir would otherwise pass every check
    // vacuously. Only the root needs this — submodules come from a real repo's
    // `git submodule status`, so they're already work trees.
    if !is_work_tree(git, root) {
        let reason = if root.exists() {
            "not a git repository"
        } else {
            "no such directory"
        };
        return vec![Entry {
            path: root.to_path_buf(),
            status: RepoStatus::unusable(reason),
        }];
    }
    let repos = collect_repos(git, root);
    let last = repos.len().saturating_sub(1);
    let mut slots: Vec<Option<RepoStatus>> = (0..repos.len()).map(|_| None).collect();

    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(repos.len());
        for (i, path) in repos.iter().enumerate() {
            let is_root = i == last;
            let ovr = if is_root { base_override } else { None };
            let do_fetch = fetch && !is_root; // zsh fetches submodules, not the root
            let path = path.clone();
            let handle = scope.spawn(move || {
                if do_fetch {
                    let _ = git.run(&path, &["fetch", "--quiet"]);
                    let _ = git.run(&path, &["remote", "prune", "origin"]);
                }
                let base = crate::config::resolve_base(git, &path, ovr);
                checks::evaluate(git, &path, &base)
            });
            handles.push((i, handle));
        }
        for (i, handle) in handles {
            slots[i] = Some(handle.join().expect("gkit: a check thread panicked"));
        }
    });

    repos
        .into_iter()
        .zip(slots)
        .map(|(path, status)| Entry {
            path,
            status: status.expect("every slot filled"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::FakeGit;

    #[test]
    fn collect_repos_is_post_order_dfs() {
        // /r has submodules a, b ; b has submodule c. Expect children before parents.
        let git = FakeGit::new()
            .ok_in("/r", "submodule status", " sha a (x)\n sha b (x)")
            .ok_in("/r/a", "submodule status", "")
            .ok_in("/r/b", "submodule status", " sha c (x)")
            .ok_in("/r/b/c", "submodule status", "");
        let order = collect_repos(&git, Path::new("/r"));
        // Normalize separators: `Path::join` yields `\` on Windows, `/` elsewhere.
        let got: Vec<String> = order
            .iter()
            .map(|p| p.display().to_string().replace('\\', "/"))
            .collect();
        assert_eq!(got, vec!["/r/a", "/r/b/c", "/r/b", "/r"]);
    }

    #[test]
    fn non_repo_root_is_flagged_not_passed() {
        // A root that isn't a work tree (rev-parse fails) must yield ONE entry that
        // fails the gate — not a vacuous pass.
        let git = FakeGit::new().fail("rev-parse --is-inside-work-tree");
        let entries = evaluate_tree(&git, Path::new("/not/a/repo"), None, false);
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].status.ok());
        assert!(entries[0].status.problem.is_some());
    }

    #[test]
    fn skips_uninitialized_submodules() {
        let git = FakeGit::new().ok_in("/r", "submodule status", "-sha a (x)\n sha b (x)\n");
        let subs = direct_submodules(&git, Path::new("/r"));
        let got: Vec<String> = subs
            .iter()
            .map(|p| p.display().to_string().replace('\\', "/"))
            .collect();
        assert_eq!(got, vec!["/r/b"]); // 'a' (uninitialized, '-') skipped
    }
}
