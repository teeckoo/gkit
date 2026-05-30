//! The five log-off checks, ported from the zsh `isEverythingCheckedIn`
//! (code-conf `gitCoreLib.sh`). Each is a pure function over a `&dyn Git`, so it
//! can be unit-tested with `FakeGit`. A repo is "ok" only if all five pass.

use crate::git::Git;
use std::collections::HashSet;
use std::path::Path;

/// Current checked-out branch (`git rev-parse --abbrev-ref HEAD`); "HEAD" if detached.
pub fn current_branch(git: &dyn Git, dir: &Path) -> String {
    git.run(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trimmed()
        .to_string()
}

/// 1. Nothing uncommitted: `git status -s` is empty.
pub fn committed(git: &dyn Git, dir: &Path) -> bool {
    git.run(dir, &["status", "-s"]).trimmed().is_empty()
}

/// 2. Every local commit exists on some remote:
///    `git log --oneline --branches --not --remotes` is empty.
pub fn all_commits_pushed(git: &dyn Git, dir: &Path) -> bool {
    git.run(
        dir,
        &["log", "--oneline", "--branches", "--not", "--remotes"],
    )
    .trimmed()
    .is_empty()
}

/// 3. Every local branch has a remote counterpart (matched by short name).
pub fn branches_have_remote(git: &dyn Git, dir: &Path) -> bool {
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
        .filter(|b| b != "HEAD")
        .collect();

    git.run(
        dir,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/*"],
    )
    .stdout
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty())
    .all(|local| remotes.contains(local))
}

/// 4. Current branch is not behind `origin/<branch>` (nothing to pull).
///    If there's no matching remote branch, there's nothing to be behind → true.
pub fn not_behind_remote(git: &dyn Git, dir: &Path) -> bool {
    let cur = current_branch(git, dir);
    if cur.is_empty() {
        return true;
    }
    let remote_ref = format!("refs/remotes/origin/{cur}");
    if !git.run(dir, &["show-ref", "--quiet", &remote_ref]).success {
        return true;
    }
    let range = format!("origin/{cur}...{cur}");
    let out = git.run(dir, &["rev-list", "--left-right", "--count", &range]);
    // Output is "<behind>\t<ahead>": left = commits in origin/cur not in cur.
    out.trimmed()
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|behind| behind == 0)
        .unwrap_or(true)
}

/// True for "integration" branches that are not feature work: the configured
/// base branch plus the universal git defaults `main`/`master`.
fn is_integration(branch: &str, base_branch: &str) -> bool {
    branch == base_branch || branch == "main" || branch == "master"
}

/// 5. Correct branch: NOT ok only if the remote has "feature" branches
///    (any head that is not an integration branch) AND we're currently sitting on
///    an integration branch (base / main / master).
pub fn correct_branch(git: &dyn Git, dir: &Path, base_branch: &str) -> bool {
    let cur = current_branch(git, dir);
    if !is_integration(&cur, base_branch) {
        return true; // on a feature branch — fine
    }
    let has_feature = git
        .run(dir, &["ls-remote", "--heads", "origin"])
        .stdout
        .lines()
        .filter_map(|l| {
            l.split_once("refs/heads/")
                .map(|(_, b)| b.trim().to_string())
        })
        .any(|b| !is_integration(&b, base_branch));
    !has_feature
}

/// Outcome of all five checks for one repo.
#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub branch: String,
    pub committed: bool,
    pub all_commits_pushed: bool,
    pub branches_have_remote: bool,
    pub not_behind_remote: bool,
    pub correct_branch: bool,
}

impl RepoStatus {
    /// True only if every check passed.
    pub fn ok(&self) -> bool {
        self.committed
            && self.all_commits_pushed
            && self.branches_have_remote
            && self.not_behind_remote
            && self.correct_branch
    }
}

/// Run all five checks for a single repo at `dir`.
pub fn evaluate(git: &dyn Git, dir: &Path, base_branch: &str) -> RepoStatus {
    RepoStatus {
        branch: current_branch(git, dir),
        committed: committed(git, dir),
        all_commits_pushed: all_commits_pushed(git, dir),
        branches_have_remote: branches_have_remote(git, dir),
        not_behind_remote: not_behind_remote(git, dir),
        correct_branch: correct_branch(git, dir, base_branch),
    }
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
    fn committed_is_true_when_status_clean() {
        assert!(committed(&FakeGit::new().ok("status -s", ""), d()));
        assert!(!committed(
            &FakeGit::new().ok("status -s", " M file.rs"),
            d()
        ));
    }

    #[test]
    fn pushed_is_true_when_no_unpushed_commits() {
        let clean = FakeGit::new().ok("log --oneline --branches --not --remotes", "");
        assert!(all_commits_pushed(&clean, d()));
        let dirty = FakeGit::new().ok("log --oneline --branches --not --remotes", "abc123 wip");
        assert!(!all_commits_pushed(&dirty, d()));
    }

    #[test]
    fn branches_have_remote_checks_every_local() {
        let ok = FakeGit::new()
            .ok(
                "for-each-ref --format=%(refname:short) refs/remotes/origin/*",
                "origin/dev\norigin/main\norigin/HEAD",
            )
            .ok("for-each-ref --format=%(refname:short) refs/heads/*", "dev");
        assert!(branches_have_remote(&ok, d()));

        let missing = FakeGit::new()
            .ok(
                "for-each-ref --format=%(refname:short) refs/remotes/origin/*",
                "origin/dev",
            )
            .ok(
                "for-each-ref --format=%(refname:short) refs/heads/*",
                "dev\nlocal-only",
            );
        assert!(!branches_have_remote(&missing, d()));
    }

    #[test]
    fn not_behind_true_when_no_remote_branch() {
        let g = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "dev")
            .fail("show-ref --quiet refs/remotes/origin/dev");
        assert!(not_behind_remote(&g, d()));
    }

    #[test]
    fn not_behind_reflects_left_count() {
        let aligned = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "dev")
            .ok("show-ref --quiet refs/remotes/origin/dev", "")
            .ok("rev-list --left-right --count origin/dev...dev", "0\t3");
        assert!(not_behind_remote(&aligned, d()));

        let behind = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "dev")
            .ok("show-ref --quiet refs/remotes/origin/dev", "")
            .ok("rev-list --left-right --count origin/dev...dev", "2\t0");
        assert!(!not_behind_remote(&behind, d()));
    }

    #[test]
    fn correct_branch_only_flags_base_with_features() {
        // On base (dev) AND remote has a feature branch -> wrong branch.
        let on_base_with_feature = FakeGit::new().ok("rev-parse --abbrev-ref HEAD", "dev").ok(
            "ls-remote --heads origin",
            "aaa\trefs/heads/dev\nbbb\trefs/heads/feature-x",
        );
        assert!(!correct_branch(&on_base_with_feature, d(), "dev"));

        // On base (dev), no feature branches -> fine.
        let on_base_no_feature = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "dev")
            .ok("ls-remote --heads origin", "aaa\trefs/heads/dev");
        assert!(correct_branch(&on_base_no_feature, d(), "dev"));

        // On a feature branch -> always fine, regardless of remote.
        let on_feature = FakeGit::new().ok("rev-parse --abbrev-ref HEAD", "feature-x");
        assert!(correct_branch(&on_feature, d(), "dev"));

        // On dev, remote has dev + main (both integration) -> NOT a feature -> fine.
        // (This is the cosp/manage-cms case that was wrongly flagged before.)
        let dev_plus_main = FakeGit::new().ok("rev-parse --abbrev-ref HEAD", "dev").ok(
            "ls-remote --heads origin",
            "aaa\trefs/heads/dev\nbbb\trefs/heads/main",
        );
        assert!(correct_branch(&dev_plus_main, d(), "dev"));

        // On main (an integration branch) with a real feature present -> flagged.
        let on_main_with_feature = FakeGit::new().ok("rev-parse --abbrev-ref HEAD", "main").ok(
            "ls-remote --heads origin",
            "aaa\trefs/heads/main\nbbb\trefs/heads/feature-y",
        );
        assert!(!correct_branch(&on_main_with_feature, d(), "dev"));
    }

    #[test]
    fn evaluate_all_clear() {
        let g = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "dev")
            .ok("status -s", "")
            .ok("log --oneline --branches --not --remotes", "")
            .ok(
                "for-each-ref --format=%(refname:short) refs/remotes/origin/*",
                "origin/dev",
            )
            .ok("for-each-ref --format=%(refname:short) refs/heads/*", "dev")
            .ok("show-ref --quiet refs/remotes/origin/dev", "")
            .ok("rev-list --left-right --count origin/dev...dev", "0\t0")
            .ok("ls-remote --heads origin", "aaa\trefs/heads/dev");
        let st = evaluate(&g, d(), "dev");
        assert!(st.ok(), "expected all-clear, got {st:?}");
        assert_eq!(st.branch, "dev");
    }
}
