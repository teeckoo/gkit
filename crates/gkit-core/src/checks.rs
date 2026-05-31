//! The five log-off checks, ported from the zsh `isEverythingCheckedIn`
//! (code-conf `gitCoreLib.sh`). Each is a pure function over a `&dyn Git`, so it
//! can be unit-tested with `FakeGit`. A repo is "ok" only if all five pass.

use crate::config::ResolvedBase;
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

/// The ref to compare "merged into base" against: the local `<base>` branch if it
/// exists, else the remote-tracking `origin/<base>`. After a normal clone you
/// often only have the default branch locally, so the remote-tracking ref is the
/// usable stand-in.
fn base_ref_for(git: &dyn Git, dir: &Path, base_branch: &str) -> String {
    let local = format!("refs/heads/{base_branch}");
    if git
        .run(dir, &["show-ref", "--verify", "--quiet", &local])
        .success
    {
        base_branch.to_string()
    } else {
        format!("origin/{base_branch}")
    }
}

/// Which correct-branch rule set applies — selected by `gkit.solo`. The two are
/// **mutually exclusive**: exactly one runs. This is the single place that decides
/// "when to use which rule".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchRule {
    /// Default (`gkit.solo` off). Flags only a **local** branch unmerged into base
    /// (your own unfinished work); others' branches on the remote are ignored.
    Team,
    /// `gkit.solo` on. Flags **any** feature branch on the **remote** (for a solo
    /// developer every remote branch is theirs, so a leftover one = unfinished
    /// work). The original strict behavior.
    Solo,
}

impl BranchRule {
    pub fn from_solo(solo: bool) -> Self {
        if solo {
            BranchRule::Solo
        } else {
            BranchRule::Team
        }
    }

    /// One-line "which rule + why" for `logoff -v` — its own line, so the
    /// `correct-branch` line stays a bare boolean.
    pub fn describe(&self) -> &'static str {
        match self {
            BranchRule::Team => "team (gkit.solo off) — flags a local branch unmerged into base",
            BranchRule::Solo => "solo (gkit.solo on) — flags any feature branch on the remote",
        }
    }
}

/// TEAM rule helper: is there a **local** non-integration branch with commits not
/// merged into base? (Can't determine the base ref → not flagged.)
fn local_unmerged_feature(git: &dyn Git, dir: &Path, base_branch: &str) -> bool {
    let base_ref = base_ref_for(git, dir, base_branch);
    let merged = git.run(
        dir,
        &["branch", "--merged", &base_ref, "--format=%(refname:short)"],
    );
    if !merged.success {
        return false;
    }
    let merged: HashSet<&str> = merged
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    git.run(
        dir,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/*"],
    )
    .stdout
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty())
    .any(|b| !is_integration(b, base_branch) && !merged.contains(b))
}

/// SOLO rule helper: does the **remote** have any non-integration (feature) branch?
fn remote_has_feature(git: &dyn Git, dir: &Path, base_branch: &str) -> bool {
    git.run(dir, &["ls-remote", "--heads", "origin"])
        .stdout
        .lines()
        .filter_map(|l| {
            l.split_once("refs/heads/")
                .map(|(_, b)| b.trim().to_string())
        })
        .any(|b| !is_integration(&b, base_branch))
}

/// 5. Correct branch — a real-life "are you parked safely?" check (see
///    `docs/commands/logoff.md`). Shared preamble for both rules:
///    - **detached HEAD** → false (risky resting state; commits easily lost).
///    - on a **feature** branch (not base/main/master) → true (actively on work).
///
///    On an **integration** branch, exactly one rule runs (see [`BranchRule`]):
///    `Team` flags a local unmerged feature branch; `Solo` flags any remote
///    feature branch.
pub fn correct_branch(git: &dyn Git, dir: &Path, base_branch: &str, rule: BranchRule) -> bool {
    // Detached HEAD: `symbolic-ref --short HEAD` fails when not on a branch.
    if !git.run(dir, &["symbolic-ref", "--short", "HEAD"]).success {
        return false;
    }
    let cur = current_branch(git, dir);
    if !is_integration(&cur, base_branch) {
        return true; // on a feature branch — fine
    }
    match rule {
        BranchRule::Team => !local_unmerged_feature(git, dir, base_branch),
        BranchRule::Solo => !remote_has_feature(git, dir, base_branch),
    }
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
    /// The base branch used for the correct-branch check + how it was resolved.
    /// When `base.name` is `None` (unresolved), `correct_branch` is forced `false`.
    pub base: ResolvedBase,
    /// Which correct-branch rule applied (`gkit.solo` selects it). Surfaced in
    /// verbose only when [`BranchRule::Solo`] (the non-default rule).
    pub rule: BranchRule,
    /// Set when the path couldn't be checked at all (missing dir / not a git
    /// repo). When present, the gate FAILS and `problem` is shown in place of the
    /// checks — otherwise a non-repo would pass every check vacuously (empty git
    /// output reads as "nothing pending").
    pub problem: Option<String>,
}

impl RepoStatus {
    /// A path that couldn't be checked (missing dir / not a git repo). Fails the
    /// gate; `reason` is rendered in place of the per-check results.
    pub fn unusable(reason: impl Into<String>) -> Self {
        RepoStatus {
            branch: String::new(),
            committed: false,
            all_commits_pushed: false,
            branches_have_remote: false,
            not_behind_remote: false,
            correct_branch: false,
            base: ResolvedBase::unresolved(),
            rule: BranchRule::Team,
            problem: Some(reason.into()),
        }
    }

    /// True only if the repo was checkable AND every check passed.
    pub fn ok(&self) -> bool {
        self.problem.is_none()
            && self.committed
            && self.all_commits_pushed
            && self.branches_have_remote
            && self.not_behind_remote
            && self.correct_branch
    }
}

/// Run all five checks for a single repo at `dir`. An unresolved base
/// (`base.name == None`) forces the correct-branch check to fail — the base
/// couldn't be determined, so we can't certify the right branch is checked out.
/// `solo` selects the correct-branch rule (`gkit.solo`; see [`BranchRule`]).
pub fn evaluate(git: &dyn Git, dir: &Path, base: &ResolvedBase, solo: bool) -> RepoStatus {
    let rule = BranchRule::from_solo(solo);
    let correct_branch = match &base.name {
        Some(b) => correct_branch(git, dir, b, rule),
        None => false,
    };
    RepoStatus {
        branch: current_branch(git, dir),
        committed: committed(git, dir),
        all_commits_pushed: all_commits_pushed(git, dir),
        branches_have_remote: branches_have_remote(git, dir),
        not_behind_remote: not_behind_remote(git, dir),
        correct_branch,
        base: base.clone(),
        rule,
        problem: None,
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

    /// Stub the on-integration path: HEAD attached on `cur`, local base `dev`
    /// exists, with the given local branches + merged set.
    fn on_integration(cur: &str, local_heads: &str, merged: &str) -> FakeGit {
        FakeGit::new()
            .ok("symbolic-ref --short HEAD", cur)
            .ok("rev-parse --abbrev-ref HEAD", cur)
            .ok("show-ref --verify --quiet refs/heads/dev", "")
            .ok("branch --merged dev --format=%(refname:short)", merged)
            .ok(
                "for-each-ref --format=%(refname:short) refs/heads/*",
                local_heads,
            )
    }

    #[test]
    fn correct_branch_detached_head_fails() {
        // Not on any branch -> risky resting state -> false (both rules; shared preamble).
        let g = FakeGit::new().fail("symbolic-ref --short HEAD");
        assert!(!correct_branch(&g, d(), "dev", BranchRule::Team));
        assert!(!correct_branch(&g, d(), "dev", BranchRule::Solo));
    }

    #[test]
    fn correct_branch_on_feature_is_fine() {
        let g = FakeGit::new()
            .ok("symbolic-ref --short HEAD", "feature-x")
            .ok("rev-parse --abbrev-ref HEAD", "feature-x");
        assert!(correct_branch(&g, d(), "dev", BranchRule::Team));
        assert!(correct_branch(&g, d(), "dev", BranchRule::Solo));
    }

    #[test]
    fn team_rule_ignores_others_remote_branches() {
        // On dev; your only LOCAL branch is dev. Others' branches live on the
        // remote, but the team rule never scans the remote -> PASS.
        // (The real-life win: the ideal logged-off state isn't flagged.)
        let g = on_integration("dev", "dev", "dev");
        assert!(correct_branch(&g, d(), "dev", BranchRule::Team));
    }

    #[test]
    fn team_rule_flags_local_unmerged_feature() {
        // On dev with a LOCAL feature branch not merged into dev -> unfinished work.
        let g = on_integration("dev", "dev\nfeature-x", "dev");
        assert!(!correct_branch(&g, d(), "dev", BranchRule::Team));
    }

    #[test]
    fn team_rule_allows_local_merged_feature() {
        // A local feature branch already merged into dev (just not deleted) -> PASS.
        let g = on_integration("dev", "dev\nfeature-x", "dev\nfeature-x");
        assert!(correct_branch(&g, d(), "dev", BranchRule::Team));
    }

    #[test]
    fn solo_rule_flags_remote_feature_branch() {
        // Solo rule: on dev, but the remote has a feature branch -> FAIL. The team
        // rule on the same repo (local dev only) -> PASS (mutually exclusive).
        let g = on_integration("dev", "dev", "dev").ok(
            "ls-remote --heads origin",
            "aaa\trefs/heads/dev\nbbb\trefs/heads/alice-x",
        );
        assert!(correct_branch(&g, d(), "dev", BranchRule::Team));
        assert!(!correct_branch(&g, d(), "dev", BranchRule::Solo));
    }

    #[test]
    fn solo_rule_passes_when_remote_is_integration_only() {
        // Solo rule, remote has only dev + main (both integration) -> PASS.
        let g = on_integration("dev", "dev", "dev").ok(
            "ls-remote --heads origin",
            "aaa\trefs/heads/dev\nbbb\trefs/heads/main",
        );
        assert!(correct_branch(&g, d(), "dev", BranchRule::Solo));
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
            // correct-branch (default rule): attached on dev, local dev merged.
            .ok("symbolic-ref --short HEAD", "dev")
            .ok("show-ref --verify --quiet refs/heads/dev", "")
            .ok("branch --merged dev --format=%(refname:short)", "dev");
        let base = ResolvedBase {
            name: Some("dev".into()),
            source: crate::config::BaseSource::Config,
        };
        let st = evaluate(&g, d(), &base, false);
        assert!(st.ok(), "expected all-clear, got {st:?}");
        assert_eq!(st.branch, "dev");
    }

    #[test]
    fn unresolved_base_fails_correct_branch() {
        // Everything else is clean, but the base couldn't be resolved → the gate
        // fails on correct-branch rather than passing vacuously.
        let g = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "feature-x")
            .ok("status -s", "")
            .ok("log --oneline --branches --not --remotes", "")
            .ok(
                "for-each-ref --format=%(refname:short) refs/remotes/origin/*",
                "origin/feature-x",
            )
            .ok(
                "for-each-ref --format=%(refname:short) refs/heads/*",
                "feature-x",
            )
            .ok("show-ref --quiet refs/remotes/origin/feature-x", "")
            .ok(
                "rev-list --left-right --count origin/feature-x...feature-x",
                "0\t0",
            );
        let st = evaluate(&g, d(), &ResolvedBase::unresolved(), false);
        assert!(!st.correct_branch);
        assert!(!st.ok());
    }
}
