//! The six log-off checks, ported from the zsh `isEverythingCheckedIn`
//! (code-conf `gitCoreLib.sh`). Each is a pure function over a `&dyn Git`, so it
//! can be unit-tested with `FakeGit`. A repo is "ok" only if all six pass.

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
///    **Fail-closed**: if we can't determine behind-ness — no current branch
///    (detached / unborn), no matching remote-tracking ref, or an unparseable
///    `rev-list` — the check **fails** rather than passing vacuously. It only
///    passes when there's a remote ref and the branch is genuinely not behind it.
pub fn not_behind_remote(git: &dyn Git, dir: &Path) -> bool {
    let cur = current_branch(git, dir);
    if cur.is_empty() {
        return false;
    }
    let remote_ref = format!("refs/remotes/origin/{cur}");
    if !git.run(dir, &["show-ref", "--quiet", &remote_ref]).success {
        return false;
    }
    let range = format!("origin/{cur}...{cur}");
    let out = git.run(dir, &["rev-list", "--left-right", "--count", &range]);
    // Output is "<behind>\t<ahead>": left = commits in origin/cur not in cur.
    out.trimmed()
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|behind| behind == 0)
        .unwrap_or(false)
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

/// TEAM rule helper: the first **local** non-integration branch with commits not
/// merged into base (your unfinished work), or `None`. (Can't determine the base
/// ref → not flagged.)
fn local_unmerged_feature(git: &dyn Git, dir: &Path, base_branch: &str) -> Option<String> {
    let base_ref = base_ref_for(git, dir, base_branch);
    let merged = git.run(
        dir,
        &["branch", "--merged", &base_ref, "--format=%(refname:short)"],
    );
    if !merged.success {
        return None;
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
    .find(|b| !is_integration(b, base_branch) && !merged.contains(*b))
    .map(str::to_string)
}

/// SOLO rule helper: the first non-integration (feature) branch on the **remote**,
/// or `None`.
fn remote_feature(git: &dyn Git, dir: &Path, base_branch: &str) -> Option<String> {
    git.run(dir, &["ls-remote", "--heads", "origin"])
        .stdout
        .lines()
        .filter_map(|l| {
            l.split_once("refs/heads/")
                .map(|(_, b)| b.trim().to_string())
        })
        .find(|b| !is_integration(b, base_branch))
}

/// The outcome of the correct-branch check, rich enough to explain *why* it
/// failed (surfaced by `logoff -vv`'s `R5 reason` line). Only the two passing
/// variants make [`BranchVerdict::passed`] true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchVerdict {
    /// On a feature branch — actively on your work (passes).
    OnFeature,
    /// On an integration branch with nothing pending under the active rule (passes).
    IntegrationClean,
    /// Detached HEAD — not on any branch (a risky resting state).
    DetachedHead,
    /// Base branch couldn't be resolved, so the check can't certify anything.
    BaseUnresolved,
    /// TEAM rule: this local branch isn't merged into base (your unfinished work).
    LocalUnmerged(String),
    /// SOLO rule: the remote has this feature branch.
    RemoteFeature(String),
}

impl BranchVerdict {
    /// Did the correct-branch check pass?
    pub fn passed(&self) -> bool {
        matches!(
            self,
            BranchVerdict::OnFeature | BranchVerdict::IntegrationClean
        )
    }

    /// One-line reason for a **failing** verdict (empty string for the passing
    /// ones) — the text shown after `R5 reason` at `logoff -vv`.
    pub fn reason(&self) -> String {
        match self {
            BranchVerdict::OnFeature | BranchVerdict::IntegrationClean => String::new(),
            BranchVerdict::DetachedHead => {
                "detached HEAD — not on any branch (commits are easily lost here)".to_string()
            }
            BranchVerdict::BaseUnresolved => {
                "base branch unresolved — set gkit.baseBranch or fetch origin/main|master"
                    .to_string()
            }
            BranchVerdict::LocalUnmerged(b) => {
                format!(
                    "local branch '{b}' is not merged into base (team rule: your unfinished work)"
                )
            }
            BranchVerdict::RemoteFeature(b) => {
                format!("remote has feature branch '{b}' (solo rule: every remote branch is yours)")
            }
        }
    }
}

/// 5. Correct branch — a real-life "are you parked safely?" check (see
///    `docs/commands/logoff.md`), returning a [`BranchVerdict`] that also explains
///    a failure. Shared preamble for both rules:
///    - **detached HEAD** → fails (risky resting state; commits easily lost).
///    - on a **feature** branch (not base/main/master) → passes (actively on work).
///
///    On an **integration** branch, exactly one rule runs (see [`BranchRule`]):
///    `Team` flags a local unmerged feature branch; `Solo` flags any remote
///    feature branch.
pub fn branch_verdict(
    git: &dyn Git,
    dir: &Path,
    base_branch: &str,
    rule: BranchRule,
) -> BranchVerdict {
    // Detached HEAD: `symbolic-ref --short HEAD` fails when not on a branch.
    if !git.run(dir, &["symbolic-ref", "--short", "HEAD"]).success {
        return BranchVerdict::DetachedHead;
    }
    let cur = current_branch(git, dir);
    if !is_integration(&cur, base_branch) {
        return BranchVerdict::OnFeature; // on a feature branch — fine
    }
    match rule {
        BranchRule::Team => match local_unmerged_feature(git, dir, base_branch) {
            Some(b) => BranchVerdict::LocalUnmerged(b),
            None => BranchVerdict::IntegrationClean,
        },
        BranchRule::Solo => match remote_feature(git, dir, base_branch) {
            Some(b) => BranchVerdict::RemoteFeature(b),
            None => BranchVerdict::IntegrationClean,
        },
    }
}

/// Boolean form of [`branch_verdict`] — for callers that only need pass/fail.
pub fn correct_branch(git: &dyn Git, dir: &Path, base_branch: &str, rule: BranchRule) -> bool {
    branch_verdict(git, dir, base_branch, rule).passed()
}

/// Whether a behind-base feature branch also carries unique commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehindKind {
    /// Has commits base lacks AND is behind base — history split (rebase onto base).
    Diverged,
    /// No unique commits, just behind base — merged/stale (switch to base & delete).
    Stale,
}

/// Outcome of R6 (`not-behind-base`): is the current **feature** branch up to date
/// with base? Fail-closed — anything we can't determine ([`BaseSyncVerdict::Undeterminable`])
/// fails. `Behind` fails unless `allowed` (`gkit.allowDiverged`), in which case it
/// passes but is surfaced as a default-level marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseSyncVerdict {
    /// On an integration branch (base/main/master) — not feature work (passes).
    NotApplicable,
    /// Feature branch, not behind base (passes).
    Current,
    /// Feature branch behind base. Fails unless `allowed`.
    Behind {
        kind: BehindKind,
        ahead: u64,
        behind: u64,
        base: String,
        allowed: bool,
    },
    /// Couldn't determine sync vs base (absent base ref, detached) — fails.
    Undeterminable { why: String },
}

impl BaseSyncVerdict {
    /// Did R6 pass? (`Behind { allowed: true }` passes but is marked.)
    pub fn passed(&self) -> bool {
        match self {
            BaseSyncVerdict::NotApplicable | BaseSyncVerdict::Current => true,
            BaseSyncVerdict::Behind { allowed, .. } => *allowed,
            BaseSyncVerdict::Undeterminable { .. } => false,
        }
    }

    /// One-line reason for a **failing** verdict (empty for passing ones) — the
    /// `R6 reason` text at `logoff -vv`.
    pub fn reason(&self) -> String {
        match self {
            BaseSyncVerdict::Behind {
                kind,
                ahead,
                behind,
                base,
                allowed: false,
            } => match kind {
                BehindKind::Diverged => format!(
                    "diverged from base '{base}': {ahead} ahead, {behind} behind — rebase onto base"
                ),
                BehindKind::Stale => format!(
                    "behind base '{base}' by {behind} (no unique commits — switch to base & delete)"
                ),
            },
            BaseSyncVerdict::Undeterminable { why } => why.clone(),
            _ => String::new(),
        }
    }

    /// The default-level marker for a **tolerated** divergence
    /// (`Behind { allowed: true }`), else `None`. ASCII, carrying the stable
    /// `gkit.allowDiverged` token so suppressed repos stay greppable.
    pub fn marker(&self) -> Option<String> {
        match self {
            BaseSyncVerdict::Behind {
                kind,
                allowed: true,
                ..
            } => Some(match kind {
                BehindKind::Diverged => "(diverged, allowed by gkit.allowDiverged)".to_string(),
                BehindKind::Stale => {
                    "(behind base, merged, allowed by gkit.allowDiverged)".to_string()
                }
            }),
            _ => None,
        }
    }
}

/// 6. Not behind base — the base-side twin of [`not_behind_remote`] (R4). On a
///    **feature** branch, fails when the branch is **behind** base (either
///    *diverged* — also ahead — or *merged/stale* — no unique commits). Integration
///    branches are skipped ([`BaseSyncVerdict::NotApplicable`]). Fail-closed:
///    detached HEAD or a base whose ref can't be located →
///    [`BaseSyncVerdict::Undeterminable`]. `allow_diverged` (`gkit.allowDiverged`)
///    downgrades a `Behind` failure to a marked pass.
pub fn base_sync_verdict(
    git: &dyn Git,
    dir: &Path,
    base_branch: &str,
    allow_diverged: bool,
) -> BaseSyncVerdict {
    // Detached HEAD: not on a branch → can't certify a feature branch vs base.
    if !git.run(dir, &["symbolic-ref", "--short", "HEAD"]).success {
        return BaseSyncVerdict::Undeterminable {
            why: "detached HEAD — not on a branch (can't compare to base)".to_string(),
        };
    }
    let cur = current_branch(git, dir);
    if is_integration(&cur, base_branch) {
        return BaseSyncVerdict::NotApplicable;
    }
    // Locate the base ref: prefer local `refs/heads/<base>`, else `origin/<base>`.
    // Fail-closed if neither exists (e.g. single-branch clone, never fetched).
    let local = format!("refs/heads/{base_branch}");
    let base_ref = if git
        .run(dir, &["show-ref", "--verify", "--quiet", &local])
        .success
    {
        base_branch.to_string()
    } else {
        let remote = format!("refs/remotes/origin/{base_branch}");
        if !git
            .run(dir, &["show-ref", "--verify", "--quiet", &remote])
            .success
        {
            return BaseSyncVerdict::Undeterminable {
                why: format!(
                    "base '{base_branch}' not found locally or on origin — fetch or set gkit.baseBranch"
                ),
            };
        }
        format!("origin/{base_branch}")
    };
    // `<base>...HEAD`: left = behind base, right = ahead of base (same orientation
    // as R4's `origin/<cur>...<cur>`).
    let range = format!("{base_ref}...HEAD");
    let out = git.run(dir, &["rev-list", "--left-right", "--count", &range]);
    let mut it = out.trimmed().split_whitespace();
    let counts = (
        it.next().and_then(|s| s.parse::<u64>().ok()),
        it.next().and_then(|s| s.parse::<u64>().ok()),
    );
    let (behind, ahead) = match counts {
        (Some(b), Some(a)) => (b, a),
        _ => {
            return BaseSyncVerdict::Undeterminable {
                why: format!("could not compare to base '{base_branch}'"),
            }
        }
    };
    if behind == 0 {
        BaseSyncVerdict::Current
    } else {
        BaseSyncVerdict::Behind {
            kind: if ahead > 0 {
                BehindKind::Diverged
            } else {
                BehindKind::Stale
            },
            ahead,
            behind,
            base: base_branch.to_string(),
            allowed: allow_diverged,
        }
    }
}

/// The five logoff checks, in run order, with stable `R<n>` ids. Single source of
/// truth for `logoff -vv` line prefixes and the `logoff -e` catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleId {
    Committed,
    AllCommitsPushed,
    BranchesHaveRemote,
    NotBehindRemote,
    CorrectBranch,
    NotBehindBase,
}

impl RuleId {
    /// All six, in the order they run and print.
    pub const ALL: [RuleId; 6] = [
        RuleId::Committed,
        RuleId::AllCommitsPushed,
        RuleId::BranchesHaveRemote,
        RuleId::NotBehindRemote,
        RuleId::CorrectBranch,
        RuleId::NotBehindBase,
    ];

    /// 1-based rule number (the `<n>` in `R<n>`).
    pub fn num(self) -> u8 {
        match self {
            RuleId::Committed => 1,
            RuleId::AllCommitsPushed => 2,
            RuleId::BranchesHaveRemote => 3,
            RuleId::NotBehindRemote => 4,
            RuleId::CorrectBranch => 5,
            RuleId::NotBehindBase => 6,
        }
    }

    /// The `R<n>` tag shown as a line prefix at `-vv` and in `-e`.
    pub fn tag(self) -> String {
        format!("R{}", self.num())
    }

    /// The stable, greppable check key — identical to the `-v` output keys.
    pub fn key(self) -> &'static str {
        match self {
            RuleId::Committed => "committed",
            RuleId::AllCommitsPushed => "all-commits-pushed",
            RuleId::BranchesHaveRemote => "branches-have-remote",
            RuleId::NotBehindRemote => "not-behind-remote",
            RuleId::CorrectBranch => "correct-branch",
            RuleId::NotBehindBase => "not-behind-base",
        }
    }

    /// One-line description for `logoff -e`.
    pub fn description(self) -> &'static str {
        match self {
            RuleId::Committed => {
                "no uncommitted changes in the working tree (git status -s is empty)"
            }
            RuleId::AllCommitsPushed => {
                "every local commit exists on some remote (nothing unpushed)"
            }
            RuleId::BranchesHaveRemote => "every local branch has a remote-tracking counterpart",
            RuleId::NotBehindRemote => {
                "the current branch tracks a remote and is not behind it (no pull needed); \
                 fail-closed — a detached/unborn HEAD or a missing remote-tracking branch fails \
                 rather than passing vacuously"
            }
            RuleId::CorrectBranch => {
                "parked on a safe branch: a feature branch always passes; on an integration \
                 branch the team rule (default) flags a local branch unmerged into base, while \
                 the solo rule (gkit.solo=true) flags any remote feature branch; detached HEAD \
                 or an unresolved base always fail"
            }
            RuleId::NotBehindBase => {
                "on a feature branch, not behind the integration base (the base-side twin of \
                 not-behind-remote): fails when the branch is behind base — diverged (also ahead, \
                 rebase) or merged/stale (no unique commits, switch to base & delete). Integration \
                 branches are skipped; fail-closed on detached HEAD or a base whose ref can't be \
                 located. Suppress with git config gkit.allowDiverged true (still shown as a marker)"
            }
        }
    }

    /// Static teaching examples — `(scenario, outcome)` pairs shown after the
    /// live state in the `-e <N>` deep dive. Illustrative, not derived from any repo.
    pub fn examples(self) -> &'static [(&'static str, &'static str)] {
        match self {
            RuleId::Committed => &[
                ("clean working tree", "PASS (nothing to commit)"),
                ("edited file, not committed", "FAIL (commit or stash it)"),
                ("staged but uncommitted file", "FAIL (still uncommitted)"),
            ],
            RuleId::AllCommitsPushed => &[
                ("every commit pushed", "PASS"),
                ("local-only commit on any branch", "FAIL (push it)"),
                ("amended commit not force-pushed", "FAIL (push the rewrite)"),
            ],
            RuleId::BranchesHaveRemote => &[
                ("every local branch tracks a remote", "PASS"),
                (
                    "local 'wip' branch never pushed",
                    "FAIL (push or delete it)",
                ),
            ],
            RuleId::NotBehindRemote => &[
                ("up to date with origin", "PASS"),
                ("no remote-tracking branch", "FAIL (push it / fix tracking)"),
                ("origin has commits you don't", "FAIL (pull --rebase)"),
            ],
            RuleId::CorrectBranch => &[
                ("on a feature branch", "PASS (actively on your work)"),
                (
                    "on base/main, all local branches merged",
                    "PASS (parked clean)",
                ),
                (
                    "on base/main, local 'wip' unmerged",
                    "FAIL (team: unfinished work)",
                ),
                (
                    "on base/main, remote feature branch exists",
                    "FAIL (solo only)",
                ),
                ("detached HEAD", "FAIL (risky resting state)"),
            ],
            RuleId::NotBehindBase => &[
                ("feature 2 ahead, 0 behind base", "PASS (on top of base)"),
                ("feature 1 ahead, 2 behind base", "FAIL (diverged — rebase)"),
                (
                    "feature 0 ahead, 3 behind base",
                    "FAIL (merged/stale — delete)",
                ),
                ("on base/main/master", "PASS (integration branch skipped)"),
                (
                    "gkit.allowDiverged=true, diverged",
                    "PASS (tolerated, marked)",
                ),
            ],
        }
    }

    /// Look up a rule by its 1-based number (for `-e <N>`).
    pub fn from_num(n: u8) -> Option<RuleId> {
        RuleId::ALL.into_iter().find(|r| r.num() == n)
    }
}

/// Outcome of all six checks for one repo.
#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub branch: String,
    pub committed: bool,
    pub all_commits_pushed: bool,
    pub branches_have_remote: bool,
    pub not_behind_remote: bool,
    pub correct_branch: bool,
    /// The detailed correct-branch verdict (drives `correct_branch` + the `-vv`
    /// `R5 reason` line).
    pub branch_verdict: BranchVerdict,
    /// The base branch used for the correct-branch check + how it was resolved.
    /// When `base.name` is `None` (unresolved), `correct_branch` is forced `false`.
    pub base: ResolvedBase,
    /// R6 (`not-behind-base`) verdict: is the current feature branch behind base?
    /// Drives the `not_behind_base` pass/fail, the `-vv` `R6 reason`, and the
    /// default-level `gkit.allowDiverged` marker.
    pub base_sync: BaseSyncVerdict,
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
            branch_verdict: BranchVerdict::BaseUnresolved,
            base: ResolvedBase::unresolved(),
            base_sync: BaseSyncVerdict::NotApplicable,
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
            && self.base_sync.passed()
    }

    /// Pass/fail for a single rule (used by the `-vv` per-rule lines).
    pub fn rule_passed(&self, rule: RuleId) -> bool {
        match rule {
            RuleId::Committed => self.committed,
            RuleId::AllCommitsPushed => self.all_commits_pushed,
            RuleId::BranchesHaveRemote => self.branches_have_remote,
            RuleId::NotBehindRemote => self.not_behind_remote,
            RuleId::CorrectBranch => self.correct_branch,
            RuleId::NotBehindBase => self.base_sync.passed(),
        }
    }

    /// The reason a rule **failed**, or `None` if it passed — the text shown after
    /// `R<n> reason` at `logoff -vv`.
    pub fn failure_reason(&self, rule: RuleId) -> Option<String> {
        if self.rule_passed(rule) {
            return None;
        }
        Some(match rule {
            RuleId::Committed => "uncommitted changes in the working tree".to_string(),
            RuleId::AllCommitsPushed => "local commits are not pushed to any remote".to_string(),
            RuleId::BranchesHaveRemote => {
                "a local branch has no remote-tracking counterpart".to_string()
            }
            RuleId::NotBehindRemote => {
                "the branch is behind its remote, or has no remote-tracking branch to compare \
                 (push it / pull --rebase)"
                    .to_string()
            }
            RuleId::CorrectBranch => self.branch_verdict.reason(),
            RuleId::NotBehindBase => self.base_sync.reason(),
        })
    }
}

/// Run all six checks for a single repo at `dir`. An unresolved base
/// (`base.name == None`) forces both base-dependent checks to fail — the base
/// couldn't be determined, so we can't certify the right branch is checked out
/// (R5) nor that it's current with base (R6). The two are **independent**: each
/// reports its own verdict. `solo` selects the correct-branch rule (`gkit.solo`;
/// see [`BranchRule`]); `allow_diverged` (`gkit.allowDiverged`) downgrades an R6
/// behind-base failure to a marked pass.
pub fn evaluate(
    git: &dyn Git,
    dir: &Path,
    base: &ResolvedBase,
    solo: bool,
    allow_diverged: bool,
) -> RepoStatus {
    let rule = BranchRule::from_solo(solo);
    let verdict = match &base.name {
        Some(b) => branch_verdict(git, dir, b, rule),
        None => BranchVerdict::BaseUnresolved,
    };
    // R6 is independent and fail-closed: an unresolved base fails it too (not a
    // vacuous pass), with its own reason — it does not defer to R5.
    let base_sync = match &base.name {
        Some(b) => base_sync_verdict(git, dir, b, allow_diverged),
        None => BaseSyncVerdict::Undeterminable {
            why: "base unresolved — set gkit.baseBranch or fetch origin/main|master".to_string(),
        },
    };
    let correct_branch = verdict.passed();
    RepoStatus {
        branch: current_branch(git, dir),
        committed: committed(git, dir),
        all_commits_pushed: all_commits_pushed(git, dir),
        branches_have_remote: branches_have_remote(git, dir),
        not_behind_remote: not_behind_remote(git, dir),
        correct_branch,
        branch_verdict: verdict,
        base: base.clone(),
        base_sync,
        rule,
        problem: None,
    }
}

/// One rule's deep-dive report for `logoff -e <N>`: the live, per-repo state behind
/// a single check, ready for [`crate::report::print_rule_detail`] to render.
#[derive(Debug, Clone)]
pub struct RuleReport {
    pub id: RuleId,
    pub passed: bool,
    /// "This repo now" label/value lines (rule-specific live state).
    pub facts: Vec<(String, String)>,
    /// One-line verdict: the failure reason, or a short "PASS …".
    pub verdict: String,
}

/// Gather the live, per-repo state behind one rule for the `-e <N>` deep dive.
/// Reads git for a **single** repo (no submodule recursion, no fetch) and reuses
/// the same git commands as the corresponding check, so the two can't drift.
pub fn rule_report(
    git: &dyn Git,
    dir: &Path,
    base: &ResolvedBase,
    solo: bool,
    allow_diverged: bool,
    id: RuleId,
) -> RuleReport {
    let lines = |out: crate::git::GitOutput| -> Vec<String> {
        out.stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    };
    let or_none = |v: &[String]| {
        if v.is_empty() {
            "(none)".to_string()
        } else {
            v.join(", ")
        }
    };

    let mut facts: Vec<(String, String)> = Vec::new();
    let (passed, verdict) = match id {
        RuleId::Committed => {
            let dirty = lines(git.run(dir, &["status", "-s"]));
            for f in &dirty {
                facts.push(("dirty".to_string(), f.clone()));
            }
            if dirty.is_empty() {
                (true, "PASS — working tree clean".to_string())
            } else {
                (
                    false,
                    format!("FAIL — {} uncommitted change(s)", dirty.len()),
                )
            }
        }
        RuleId::AllCommitsPushed => {
            let unpushed = lines(git.run(
                dir,
                &["log", "--oneline", "--branches", "--not", "--remotes"],
            ));
            for c in &unpushed {
                facts.push(("unpushed".to_string(), c.clone()));
            }
            if unpushed.is_empty() {
                (true, "PASS — nothing unpushed".to_string())
            } else {
                (
                    false,
                    format!("FAIL — {} commit(s) not on any remote", unpushed.len()),
                )
            }
        }
        RuleId::BranchesHaveRemote => {
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
            let locals = lines(git.run(
                dir,
                &["for-each-ref", "--format=%(refname:short)", "refs/heads/*"],
            ));
            facts.push(("local branches".to_string(), or_none(&locals)));
            let missing: Vec<String> = locals
                .iter()
                .filter(|b| !remotes.contains(*b))
                .cloned()
                .collect();
            if missing.is_empty() {
                (
                    true,
                    "PASS — every local branch tracks a remote".to_string(),
                )
            } else {
                facts.push(("missing remote".to_string(), missing.join(", ")));
                (
                    false,
                    format!("FAIL — no remote for: {}", missing.join(", ")),
                )
            }
        }
        RuleId::NotBehindRemote => {
            let cur = current_branch(git, dir);
            facts.push((
                "branch".to_string(),
                if cur.is_empty() {
                    "(detached)".to_string()
                } else {
                    cur.clone()
                },
            ));
            if cur.is_empty() {
                (
                    false,
                    "FAIL — no current branch (detached/unborn); can't compare to a remote"
                        .to_string(),
                )
            } else {
                let remote_ref = format!("refs/remotes/origin/{cur}");
                if !git.run(dir, &["show-ref", "--quiet", &remote_ref]).success {
                    facts.push(("remote branch".to_string(), "none".to_string()));
                    (
                        false,
                        "FAIL — no remote-tracking branch (push it / fix tracking)".to_string(),
                    )
                } else {
                    let range = format!("origin/{cur}...{cur}");
                    let behind = git
                        .run(dir, &["rev-list", "--left-right", "--count", &range])
                        .trimmed()
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);
                    facts.push(("behind by".to_string(), behind.to_string()));
                    if behind == 0 {
                        (true, "PASS — up to date with origin".to_string())
                    } else {
                        (
                            false,
                            format!("FAIL — behind by {behind} commit(s); pull --rebase"),
                        )
                    }
                }
            }
        }
        RuleId::CorrectBranch => {
            let rule = BranchRule::from_solo(solo);
            let cur = current_branch(git, dir);
            let verdict_enum = match &base.name {
                Some(b) => branch_verdict(git, dir, b, rule),
                None => BranchVerdict::BaseUnresolved,
            };
            let locals = lines(git.run(
                dir,
                &["for-each-ref", "--format=%(refname:short)", "refs/heads/*"],
            ));
            facts.push((
                "branch".to_string(),
                if cur.is_empty() {
                    "(detached)".to_string()
                } else {
                    cur.clone()
                },
            ));
            facts.push(("base".to_string(), base.describe()));
            facts.push(("rule".to_string(), rule.describe().to_string()));
            facts.push(("local branches".to_string(), or_none(&locals)));
            if verdict_enum.passed() {
                (true, "PASS — parked safely".to_string())
            } else {
                (false, format!("FAIL — {}", verdict_enum.reason()))
            }
        }
        RuleId::NotBehindBase => {
            let cur = current_branch(git, dir);
            facts.push((
                "branch".to_string(),
                if cur.is_empty() {
                    "(detached)".to_string()
                } else {
                    cur.clone()
                },
            ));
            facts.push(("base".to_string(), base.describe()));
            let verdict = match &base.name {
                Some(b) => base_sync_verdict(git, dir, b, allow_diverged),
                None => BaseSyncVerdict::Undeterminable {
                    why: "base unresolved — set gkit.baseBranch or fetch origin/main|master"
                        .to_string(),
                },
            };
            if let BaseSyncVerdict::Behind { ahead, behind, .. } = &verdict {
                facts.push(("ahead of base".to_string(), ahead.to_string()));
                facts.push(("behind base".to_string(), behind.to_string()));
            }
            match &verdict {
                BaseSyncVerdict::NotApplicable => (
                    true,
                    "PASS — on an integration branch (not feature work)".to_string(),
                ),
                BaseSyncVerdict::Current => (
                    true,
                    "PASS — feature branch is current with base".to_string(),
                ),
                BaseSyncVerdict::Behind {
                    kind,
                    ahead,
                    behind,
                    base: b,
                    allowed: true,
                } => {
                    let what = match kind {
                        BehindKind::Diverged => {
                            format!("diverged from '{b}' ({ahead} ahead, {behind} behind)")
                        }
                        BehindKind::Stale => format!("behind '{b}' by {behind} (merged/stale)"),
                    };
                    (
                        true,
                        format!("PASS — {what} but allowed by gkit.allowDiverged"),
                    )
                }
                BaseSyncVerdict::Behind { allowed: false, .. }
                | BaseSyncVerdict::Undeterminable { .. } => {
                    (false, format!("FAIL — {}", verdict.reason()))
                }
            }
        }
    };
    RuleReport {
        id,
        passed,
        facts,
        verdict,
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
    fn not_behind_false_when_no_remote_branch() {
        // Fail-closed: no remote-tracking ref to compare against -> fail, not a
        // vacuous pass (R3 owns "branch has no remote"; R4 stays independent).
        let g = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "dev")
            .fail("show-ref --quiet refs/remotes/origin/dev");
        assert!(!not_behind_remote(&g, d()));
    }

    #[test]
    fn not_behind_false_when_detached_or_unborn() {
        // No current branch name -> can't determine behind-ness -> fail-closed.
        let g = FakeGit::new().ok("rev-parse --abbrev-ref HEAD", "");
        assert!(!not_behind_remote(&g, d()));
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
        let st = evaluate(&g, d(), &base, false, false);
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
        let st = evaluate(&g, d(), &ResolvedBase::unresolved(), false, false);
        assert!(!st.correct_branch);
        assert!(!st.ok());
    }

    // ---- branch_verdict: the reason behind a correct-branch verdict (for -vv) ----

    #[test]
    fn verdict_detached_head() {
        let g = FakeGit::new().fail("symbolic-ref --short HEAD");
        assert_eq!(
            branch_verdict(&g, d(), "dev", BranchRule::Team),
            BranchVerdict::DetachedHead
        );
    }

    #[test]
    fn verdict_on_feature_branch() {
        let g = FakeGit::new()
            .ok("symbolic-ref --short HEAD", "feature-x")
            .ok("rev-parse --abbrev-ref HEAD", "feature-x");
        assert_eq!(
            branch_verdict(&g, d(), "dev", BranchRule::Team),
            BranchVerdict::OnFeature
        );
    }

    #[test]
    fn verdict_team_names_the_unmerged_local_branch() {
        let g = on_integration("dev", "dev\nfeature-x", "dev");
        assert_eq!(
            branch_verdict(&g, d(), "dev", BranchRule::Team),
            BranchVerdict::LocalUnmerged("feature-x".into())
        );
    }

    #[test]
    fn verdict_team_clean_integration() {
        let g = on_integration("dev", "dev", "dev");
        assert_eq!(
            branch_verdict(&g, d(), "dev", BranchRule::Team),
            BranchVerdict::IntegrationClean
        );
    }

    #[test]
    fn verdict_solo_names_the_remote_feature_branch() {
        let g = on_integration("dev", "dev", "dev").ok(
            "ls-remote --heads origin",
            "aaa\trefs/heads/dev\nbbb\trefs/heads/alice-x",
        );
        assert_eq!(
            branch_verdict(&g, d(), "dev", BranchRule::Solo),
            BranchVerdict::RemoteFeature("alice-x".into())
        );
    }

    #[test]
    fn verdict_reason_is_empty_only_when_passing() {
        assert!(BranchVerdict::OnFeature.reason().is_empty());
        assert!(BranchVerdict::IntegrationClean.reason().is_empty());
        assert!(BranchVerdict::DetachedHead
            .reason()
            .contains("detached HEAD"));
        assert!(BranchVerdict::BaseUnresolved
            .reason()
            .contains("unresolved"));
        assert!(BranchVerdict::LocalUnmerged("x".into())
            .reason()
            .contains("'x'"));
        assert!(BranchVerdict::RemoteFeature("x".into())
            .reason()
            .contains("'x'"));
    }

    #[test]
    fn evaluate_unresolved_sets_base_unresolved_verdict() {
        // The forced-fail path records *why* (for the -vv reason line), not a bare
        // false.
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
        let st = evaluate(&g, d(), &ResolvedBase::unresolved(), false, false);
        assert_eq!(st.branch_verdict, BranchVerdict::BaseUnresolved);
        assert_eq!(
            st.failure_reason(RuleId::CorrectBranch),
            Some(BranchVerdict::BaseUnresolved.reason())
        );
    }

    // ---- RuleId catalog + per-rule reasons (for -vv and -e) ----

    #[test]
    fn rule_ids_are_stable_and_round_trip() {
        let nums: Vec<u8> = RuleId::ALL.iter().map(|r| r.num()).collect();
        assert_eq!(nums, vec![1, 2, 3, 4, 5, 6]);
        for r in RuleId::ALL {
            assert_eq!(RuleId::from_num(r.num()), Some(r));
            assert_eq!(r.tag(), format!("R{}", r.num()));
            assert!(!r.key().is_empty() && !r.description().is_empty());
        }
        assert_eq!(RuleId::from_num(0), None);
        assert_eq!(RuleId::from_num(7), None);
        assert_eq!(RuleId::CorrectBranch.key(), "correct-branch");
        assert_eq!(RuleId::NotBehindBase.key(), "not-behind-base");
        assert_eq!(RuleId::from_num(6), Some(RuleId::NotBehindBase));
    }

    #[test]
    fn failure_reason_is_some_only_for_failing_rules() {
        // A repo failing only committed: that rule reports a reason, the rest don't.
        let g = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "dev")
            .ok("status -s", " M file.txt") // dirty -> committed fails
            .ok("log --oneline --branches --not --remotes", "")
            .ok(
                "for-each-ref --format=%(refname:short) refs/remotes/origin/*",
                "origin/dev",
            )
            .ok("for-each-ref --format=%(refname:short) refs/heads/*", "dev")
            .ok("show-ref --quiet refs/remotes/origin/dev", "")
            .ok("rev-list --left-right --count origin/dev...dev", "0\t0")
            .ok("symbolic-ref --short HEAD", "dev")
            .ok("show-ref --verify --quiet refs/heads/dev", "")
            .ok("branch --merged dev --format=%(refname:short)", "dev");
        let base = ResolvedBase {
            name: Some("dev".into()),
            source: crate::config::BaseSource::Config,
        };
        let st = evaluate(&g, d(), &base, false, false);
        assert!(st.failure_reason(RuleId::Committed).is_some());
        assert!(st.failure_reason(RuleId::AllCommitsPushed).is_none());
        assert!(st.failure_reason(RuleId::CorrectBranch).is_none());
    }

    // ---- RuleId::examples + rule_report (the `-e <N>` deep dive) ----

    #[test]
    fn every_rule_has_examples() {
        for r in RuleId::ALL {
            assert!(!r.examples().is_empty(), "{:?} has no examples", r);
        }
    }

    fn dev_base() -> ResolvedBase {
        ResolvedBase {
            name: Some("dev".into()),
            source: crate::config::BaseSource::Config,
        }
    }

    #[test]
    fn rule_report_r5_names_unmerged_branch_and_lists_state() {
        // On dev (integration) with a local unmerged feature -> FAIL, naming it.
        let g = on_integration("dev", "dev\nfeature-x", "dev");
        let rep = rule_report(&g, d(), &dev_base(), false, false, RuleId::CorrectBranch);
        assert!(!rep.passed);
        assert!(
            rep.verdict.contains("feature-x"),
            "verdict: {}",
            rep.verdict
        );
        // "This repo now" surfaces branch, base, rule, and the local branches.
        let facts: std::collections::HashMap<_, _> = rep.facts.iter().cloned().collect();
        assert_eq!(facts.get("branch").map(String::as_str), Some("dev"));
        assert!(facts.contains_key("base"));
        assert!(facts.get("local branches").unwrap().contains("feature-x"));
    }

    #[test]
    fn rule_report_r1_lists_dirty_files() {
        let g = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "dev")
            .ok("status -s", " M a.txt\n?? b.txt");
        let rep = rule_report(&g, d(), &dev_base(), false, false, RuleId::Committed);
        assert!(!rep.passed);
        let dirty: Vec<&str> = rep
            .facts
            .iter()
            .filter(|(l, _)| l == "dirty")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(dirty, vec!["M a.txt", "?? b.txt"]);
    }

    #[test]
    fn rule_report_r4_shows_behind_count() {
        let g = FakeGit::new()
            .ok("rev-parse --abbrev-ref HEAD", "dev")
            .ok("show-ref --quiet refs/remotes/origin/dev", "")
            .ok("rev-list --left-right --count origin/dev...dev", "3\t0");
        let rep = rule_report(&g, d(), &dev_base(), false, false, RuleId::NotBehindRemote);
        assert!(!rep.passed);
        let facts: std::collections::HashMap<_, _> = rep.facts.iter().cloned().collect();
        assert_eq!(facts.get("behind by").map(String::as_str), Some("3"));
        assert!(
            rep.verdict.contains("behind by 3"),
            "verdict: {}",
            rep.verdict
        );
    }

    // ---- R6 not-behind-base (base_sync_verdict) ----

    /// On feature branch `cur`, base `dev` resolves to `origin/dev` (no local
    /// `dev`), with the given `<behind>\t<ahead>` rev-list count vs base.
    fn on_feature_vs_base(cur: &str, counts: &str) -> FakeGit {
        FakeGit::new()
            .ok("symbolic-ref --short HEAD", cur)
            .ok("rev-parse --abbrev-ref HEAD", cur)
            .fail("show-ref --verify --quiet refs/heads/dev")
            .ok("show-ref --verify --quiet refs/remotes/origin/dev", "")
            .ok("rev-list --left-right --count origin/dev...HEAD", counts)
    }

    #[test]
    fn base_sync_diverged_fails() {
        // behind 2, ahead 1 -> diverged -> fail (allow_diverged off).
        let v = base_sync_verdict(&on_feature_vs_base("feature-x", "2\t1"), d(), "dev", false);
        assert!(!v.passed());
        assert!(matches!(
            v,
            BaseSyncVerdict::Behind {
                kind: BehindKind::Diverged,
                ahead: 1,
                behind: 2,
                allowed: false,
                ..
            }
        ));
        assert!(v.reason().contains("diverged from base 'dev'"));
    }

    #[test]
    fn base_sync_pure_ahead_passes() {
        // behind 0, ahead 3 -> on top of base -> pass.
        let v = base_sync_verdict(&on_feature_vs_base("feature-x", "0\t3"), d(), "dev", false);
        assert!(v.passed());
        assert_eq!(v, BaseSyncVerdict::Current);
    }

    #[test]
    fn base_sync_merged_stale_fails() {
        // behind 2, ahead 0 -> merged/stale -> fail.
        let v = base_sync_verdict(&on_feature_vs_base("feature-x", "2\t0"), d(), "dev", false);
        assert!(!v.passed());
        assert!(matches!(
            v,
            BaseSyncVerdict::Behind {
                kind: BehindKind::Stale,
                ..
            }
        ));
        assert!(v.reason().contains("behind base 'dev'"));
    }

    #[test]
    fn base_sync_even_passes() {
        let v = base_sync_verdict(&on_feature_vs_base("feature-x", "0\t0"), d(), "dev", false);
        assert!(v.passed());
    }

    #[test]
    fn base_sync_integration_branch_not_applicable() {
        // On dev itself -> not feature work -> NotApplicable (passes, no marker).
        let g = FakeGit::new()
            .ok("symbolic-ref --short HEAD", "dev")
            .ok("rev-parse --abbrev-ref HEAD", "dev");
        let v = base_sync_verdict(&g, d(), "dev", false);
        assert_eq!(v, BaseSyncVerdict::NotApplicable);
        assert!(v.passed());
        assert!(v.marker().is_none());
    }

    #[test]
    fn base_sync_detached_is_undeterminable() {
        let g = FakeGit::new().fail("symbolic-ref --short HEAD");
        let v = base_sync_verdict(&g, d(), "dev", false);
        assert!(!v.passed());
        assert!(matches!(v, BaseSyncVerdict::Undeterminable { .. }));
        assert!(v.reason().contains("detached"));
    }

    #[test]
    fn base_sync_absent_base_ref_is_undeterminable() {
        // Base 'dev' present neither locally nor on origin -> fail-closed.
        let g = FakeGit::new()
            .ok("symbolic-ref --short HEAD", "feature-x")
            .ok("rev-parse --abbrev-ref HEAD", "feature-x")
            .fail("show-ref --verify --quiet refs/heads/dev")
            .fail("show-ref --verify --quiet refs/remotes/origin/dev");
        let v = base_sync_verdict(&g, d(), "dev", false);
        assert!(!v.passed());
        assert!(v.reason().contains("not found"));
    }

    #[test]
    fn base_sync_allow_diverged_suppresses_to_marked_pass() {
        // Same diverged repo, but allow_diverged -> passes with a marker.
        let v = base_sync_verdict(&on_feature_vs_base("feature-x", "2\t1"), d(), "dev", true);
        assert!(v.passed());
        assert!(matches!(v, BaseSyncVerdict::Behind { allowed: true, .. }));
        let marker = v.marker().expect("allowed divergence has a marker");
        assert!(marker.contains("allowed by gkit.allowDiverged"));
        assert!(marker.contains("diverged"));
    }

    #[test]
    fn evaluate_unresolved_base_fails_r6_independently() {
        // No base -> R6 is Undeterminable (fail-closed), independent of R5.
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
        let st = evaluate(&g, d(), &ResolvedBase::unresolved(), false, false);
        assert!(!st.rule_passed(RuleId::NotBehindBase));
        assert!(st.failure_reason(RuleId::NotBehindBase).is_some());
    }
}
