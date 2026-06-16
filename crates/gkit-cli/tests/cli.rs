//! Hermetic, always-on integration tests for the `gkit` CLI.
//!
//! No network, no ssh: fixtures are local `file://` bare repos + clones. Runs on
//! the CI matrix via `cargo test`. See `common/mod.rs` for the hermetic env.

mod common;
use common::*;

// ---------------------------------------------------------------- the 4 checks

#[test]
fn logoff_clean_repo_passes() {
    let r = repo_with_remote("clean", "main");
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "committed", "true");
    assert_check(&o.stdout, &r.work, "all-commits-pushed", "true");
    assert_check(&o.stdout, &r.work, "branches-have-remote", "true");
    assert_check(&o.stdout, &r.work, "not-behind-remote", "true");
    assert_check(&o.stdout, &r.work, "correct-branch", "true");
    // Default (team) rule is silent — no `branch-rule` noise.
    assert!(
        !o.stdout.contains("branch-rule"),
        "team default should not print a branch-rule line:\n{}",
        o.stdout
    );
    assert_eq!(o.code, 0, "clean repo should pass:\n{}", o.all());
}

#[test]
fn logoff_dirty_repo_fails() {
    let r = repo_with_remote("dirty", "main");
    std::fs::write(r.work.join("README.md"), "changed\n").unwrap();
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "committed", "false");
    assert_eq!(o.code, 1);
}

#[test]
fn logoff_unpushed_commit_fails() {
    let r = repo_with_remote("unpushed", "main");
    std::fs::write(r.work.join("new.txt"), "x\n").unwrap();
    git_ok(&r.work, &["add", "."]);
    git_ok(&r.work, &["commit", "-m", "local only"]);
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "all-commits-pushed", "false");
    assert_eq!(o.code, 1);
}

#[test]
fn logoff_local_branch_without_remote_fails() {
    let r = repo_with_remote("noremote", "main");
    git_ok(&r.work, &["checkout", "-b", "local-only"]);
    std::fs::write(r.work.join("f.txt"), "x\n").unwrap();
    git_ok(&r.work, &["add", "."]);
    git_ok(&r.work, &["commit", "-m", "wip"]);
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "branches-have-remote", "false");
    assert_eq!(o.code, 1);
}

#[test]
fn logoff_non_git_dir_fails() {
    let d = temp_dir("notgit");
    let o = gkit(&d, &["logoff", "--no-fetch", d.to_str().unwrap()]);
    assert_contains(&o.stdout, "not a git repository");
    assert_eq!(o.code, 1);
}

#[test]
fn logoff_missing_dir_fails() {
    let d = temp_dir("missingparent");
    let missing = d.join("does-not-exist");
    let o = gkit(&d, &["logoff", "--no-fetch", missing.to_str().unwrap()]);
    assert_contains(&o.stdout, "no such directory");
    assert_eq!(o.code, 1);
}

// ---------------------------------------------------------------- base branch

#[test]
fn base_from_git_config() {
    let r = repo_with_remote("baseconfig", "main");
    git_ok(&r.work, &["config", "gkit.baseBranch", "dev"]);
    // base-branch metadata is a -vv line now (-v is a pure pass/fail scan).
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(
        &o.stdout,
        &r.work,
        "base-branch",
        "dev (from git config gkit.baseBranch)",
    );
}

#[test]
fn base_from_remote_main() {
    let r = repo_with_remote("basemain", "main");
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(
        &o.stdout,
        &r.work,
        "base-branch",
        "main (derived from remote origin/main)",
    );
}

#[test]
fn base_from_remote_master() {
    let r = repo_with_remote("basemaster", "master");
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(
        &o.stdout,
        &r.work,
        "base-branch",
        "master (derived from remote origin/master)",
    );
}

#[test]
fn base_unresolved_fails() {
    // remote default branch is neither main nor master, and no gkit.baseBranch.
    let r = repo_with_remote("basenone", "trunk");
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert!(
        o.stdout
            .lines()
            .any(|l| l.contains("base-branch") && l.contains("UNRESOLVED")),
        "expected UNRESOLVED base-branch:\n{}",
        o.stdout
    );
    assert_check(&o.stdout, &r.work, "R5 correct-branch", "false");
    assert_eq!(o.code, 1);
}

#[test]
fn base_branch_flag_override() {
    let r = repo_with_remote("baseflag", "main");
    let o = gkit(
        &r.work,
        &[
            "logoff",
            "-vv",
            "--no-fetch",
            "--base-branch",
            "custombase",
            r.work.to_str().unwrap(),
        ],
    );
    assert_check(
        &o.stdout,
        &r.work,
        "base-branch",
        "custombase (from --base-branch)",
    );
}

// ---------------------------------------------------------------- correct-branch (redesign)

/// Push a feature branch to the remote, then delete it locally — so it exists
/// only as a remote-tracking ref (other people's work).
fn push_then_drop_local_feature(work: &std::path::Path, branch: &str) {
    git_ok(work, &["checkout", "-b", branch]);
    std::fs::write(work.join(format!("{branch}.txt")), "x\n").unwrap();
    git_ok(work, &["add", "."]);
    git_ok(work, &["commit", "-m", "feature work"]);
    git_ok(work, &["push", "-u", "origin", branch]);
    git_ok(work, &["checkout", "main"]);
    git_ok(work, &["branch", "-D", branch]);
}

#[test]
fn correct_branch_default_ignores_others_remote_branches() {
    // On main, only `main` locally; a feature branch lives only on the remote.
    let r = repo_with_remote("others", "main");
    push_then_drop_local_feature(&r.work, "alice-x");
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "correct-branch", "true");
    assert_eq!(
        o.code,
        0,
        "ideal logged-off state should pass:\n{}",
        o.all()
    );
}

#[test]
fn correct_branch_flags_local_unmerged_feature() {
    // On main with a LOCAL feature branch (pushed) that isn't merged into main.
    let r = repo_with_remote("localunmerged", "main");
    git_ok(&r.work, &["checkout", "-b", "feature-y"]);
    std::fs::write(r.work.join("y.txt"), "x\n").unwrap();
    git_ok(&r.work, &["add", "."]);
    git_ok(&r.work, &["commit", "-m", "unmerged"]);
    git_ok(&r.work, &["push", "-u", "origin", "feature-y"]);
    git_ok(&r.work, &["checkout", "main"]);
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "correct-branch", "false");
    assert_eq!(o.code, 1);
}

#[test]
fn correct_branch_allows_local_merged_feature() {
    // A local feature branch at main's HEAD (already "merged"), just not deleted.
    let r = repo_with_remote("localmerged", "main");
    git_ok(&r.work, &["branch", "feature-z"]); // points at main HEAD
    git_ok(&r.work, &["push", "-u", "origin", "feature-z"]);
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "correct-branch", "true");
    assert_eq!(o.code, 0, "merged local branch should pass:\n{}", o.all());
}

#[test]
fn correct_branch_detached_head_fails() {
    let r = repo_with_remote("detached", "main");
    git_ok(&r.work, &["checkout", "--detach", "HEAD"]);
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "correct-branch", "false");
    assert_eq!(o.code, 1);
}

#[test]
fn solo_flags_remote_feature_branch() {
    // solo=true: on main, clean locally, but a feature branch exists on the remote.
    let r = repo_with_remote("solo-on", "main");
    push_then_drop_local_feature(&r.work, "bob-y");
    git_ok(&r.work, &["config", "gkit.solo", "true"]);
    // branch-rule is a -vv metadata line now; the gate still fails at any level.
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(
        &o.stdout,
        &r.work,
        "branch-rule",
        "solo (gkit.solo on) — flags any feature branch on the remote",
    );
    assert_check(&o.stdout, &r.work, "R5 correct-branch", "false");
    assert_eq!(o.code, 1);
}

#[test]
fn solo_passes_when_remote_is_integration_only() {
    let r = repo_with_remote("solo-clean", "main");
    git_ok(&r.work, &["config", "gkit.solo", "true"]);
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(
        &o.stdout,
        &r.work,
        "branch-rule",
        "solo (gkit.solo on) — flags any feature branch on the remote",
    );
    assert_check(&o.stdout, &r.work, "R5 correct-branch", "true");
    assert_eq!(
        o.code,
        0,
        "solo + integration-only remote should pass:\n{}",
        o.all()
    );
}

// ---------------------------------------------------------------- -vv reasons + -e

#[test]
fn vv_explains_why_correct_branch_failed() {
    // On main with a LOCAL feature branch unmerged into main (team rule fail).
    // -vv must name the offending branch in an `R5 reason` line.
    let r = repo_with_remote("vv-localunmerged", "main");
    git_ok(&r.work, &["checkout", "-b", "feature-y"]);
    std::fs::write(r.work.join("y.txt"), "x\n").unwrap();
    git_ok(&r.work, &["add", "."]);
    git_ok(&r.work, &["commit", "-m", "unmerged"]);
    git_ok(&r.work, &["push", "-u", "origin", "feature-y"]);
    git_ok(&r.work, &["checkout", "main"]);
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    // The check line carries the R5 rule id...
    assert_check(&o.stdout, &r.work, "R5 correct-branch", "false");
    // ...and a reason line names the branch.
    assert_contains(&o.stdout, "R5 reason");
    assert_contains(&o.stdout, "feature-y");
    assert_eq!(o.code, 1);
}

#[test]
fn vv_clean_repo_has_no_reason_lines() {
    // -vv on a passing repo: every check gets an R<n> prefix, but no reason lines
    // (reasons appear only for failures).
    let r = repo_with_remote("vv-clean", "main");
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "R1 committed", "true");
    assert_check(&o.stdout, &r.work, "R5 correct-branch", "true");
    assert!(
        !o.stdout.contains("reason"),
        "a clean repo should have no reason lines:\n{}",
        o.stdout
    );
    assert_eq!(o.code, 0, "clean repo should pass:\n{}", o.all());
}

#[test]
fn v_single_is_a_pure_scan() {
    // -v is the bare pass/fail scan: no R<n> prefixes, no reasons, and no
    // contextual metadata (base-branch / branch-rule moved to -vv).
    let r = repo_with_remote("v-plain", "main");
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "committed", "true");
    assert!(
        !o.stdout.contains("R1 ")
            && !o.stdout.contains("reason")
            && !o.stdout.contains("base-branch")
            && !o.stdout.contains("branch-rule"),
        "-v must be a pure scan (no ids, reasons, or metadata):\n{}",
        o.stdout
    );
}

#[test]
fn explain_lists_all_rules() {
    let d = temp_dir("explain-all");
    let o = gkit(&d, &["logoff", "-e"]);
    for tag in ["R1", "R2", "R3", "R4", "R5", "R6"] {
        assert_contains(&o.stdout, tag);
    }
    assert_contains(&o.stdout, "correct-branch");
    assert_contains(&o.stdout, "not-behind-base");
    assert_eq!(o.code, 0);
}

#[test]
fn explain_rule_deep_dive_reads_the_repo() {
    // `-e 5` on a repo on main with a local unmerged feature: the deep dive shows
    // the rule, this repo's live state (naming the branch), examples, and the
    // FAIL verdict — but exits 0 (informational, not the gate).
    let r = repo_with_remote("explain-deep", "main");
    git_ok(&r.work, &["checkout", "-b", "feature-y"]);
    std::fs::write(r.work.join("y.txt"), "x\n").unwrap();
    git_ok(&r.work, &["add", "."]);
    git_ok(&r.work, &["commit", "-m", "unmerged"]);
    git_ok(&r.work, &["push", "-u", "origin", "feature-y"]);
    git_ok(&r.work, &["checkout", "main"]);
    let o = gkit(&r.work, &["logoff", "-e", "5", r.work.to_str().unwrap()]);
    assert_contains(&o.stdout, "R5");
    assert_contains(&o.stdout, "correct-branch");
    assert_contains(&o.stdout, "[this repo: FAIL]");
    assert_contains(&o.stdout, "This repo now");
    assert_contains(&o.stdout, "feature-y"); // live branch value
    assert_contains(&o.stdout, "Examples");
    assert_eq!(
        o.code,
        0,
        "explain is informational, not a gate:\n{}",
        o.all()
    );
}

#[test]
fn explain_invalid_rule_errors() {
    let d = temp_dir("explain-bad");
    let o = gkit(&d, &["logoff", "-e", "9"]);
    assert_contains(&o.stderr, "no such rule 9");
    assert_ne!(o.code, 0);
}

// ---------------------------------------------------------------- R6 not-behind-base

/// Leave HEAD on feature `branch` with a unique commit (pushed) while LOCAL `main`
/// has advanced one commit (pushed) — so `branch` is 1 ahead / 1 behind base.
fn make_diverged(work: &std::path::Path, branch: &str) {
    git_ok(work, &["checkout", "-b", branch]);
    std::fs::write(work.join(format!("{branch}.txt")), "x\n").unwrap();
    git_ok(work, &["add", "."]);
    git_ok(work, &["commit", "-m", "feature work"]);
    git_ok(work, &["push", "-u", "origin", branch]);
    git_ok(work, &["checkout", "main"]);
    std::fs::write(work.join("main2.txt"), "y\n").unwrap();
    git_ok(work, &["add", "."]);
    git_ok(work, &["commit", "-m", "advance main"]);
    git_ok(work, &["push", "origin", "main"]);
    git_ok(work, &["checkout", branch]);
}

#[test]
fn r6_diverged_feature_fails() {
    let r = repo_with_remote("r6-diverged", "main");
    make_diverged(&r.work, "SCB-283");
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "R6 not-behind-base", "false");
    assert_contains(&o.stdout, "diverged from base 'main'");
    assert_eq!(
        o.code,
        1,
        "diverged feature branch should fail:\n{}",
        o.all()
    );
}

#[test]
fn r6_merged_stale_feature_fails() {
    // Feature branch at the OLD base tip (no unique commits), base advances -> stale.
    let r = repo_with_remote("r6-stale", "main");
    git_ok(&r.work, &["checkout", "-b", "feature-z"]); // at main HEAD
    git_ok(&r.work, &["push", "-u", "origin", "feature-z"]);
    git_ok(&r.work, &["checkout", "main"]);
    std::fs::write(r.work.join("main2.txt"), "y\n").unwrap();
    git_ok(&r.work, &["add", "."]);
    git_ok(&r.work, &["commit", "-m", "advance main"]);
    git_ok(&r.work, &["push", "origin", "main"]);
    git_ok(&r.work, &["checkout", "feature-z"]);
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "R6 not-behind-base", "false");
    assert_contains(&o.stdout, "behind base 'main'");
    assert_eq!(o.code, 1);
}

#[test]
fn r6_allow_diverged_passes_with_default_marker() {
    // Same diverged repo + gkit.allowDiverged -> passes, default line is marked.
    let r = repo_with_remote("r6-allowed", "main");
    make_diverged(&r.work, "SCB-283");
    git_ok(&r.work, &["config", "gkit.allowDiverged", "true"]);
    // Default output (no -v/-vv): the suppression marker rides the default line.
    let o = gkit(&r.work, &["logoff", "--no-fetch", r.work.to_str().unwrap()]);
    assert_contains(&o.stdout, "allowed by gkit.allowDiverged");
    assert_eq!(o.code, 0, "allowDiverged should pass:\n{}", o.all());
}

#[test]
fn r6_pure_ahead_feature_passes() {
    // Ahead of base but NOT behind -> on top of base -> R6 passes.
    let r = repo_with_remote("r6-ahead", "main");
    git_ok(&r.work, &["checkout", "-b", "feature-x"]);
    std::fs::write(r.work.join("f.txt"), "x\n").unwrap();
    git_ok(&r.work, &["add", "."]);
    git_ok(&r.work, &["commit", "-m", "feature"]);
    git_ok(&r.work, &["push", "-u", "origin", "feature-x"]);
    let o = gkit(
        &r.work,
        &["logoff", "-vv", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&o.stdout, &r.work, "R6 not-behind-base", "true");
    assert_eq!(o.code, 0, "pure-ahead feature should pass:\n{}", o.all());
}

// ---------------------------------------------------------------- root fetch

/// gkit logoff fetches the ROOT repo (not just submodules) before the behind
/// checks. The remote advances out-of-band while the work clone's `origin/main`
/// stays stale: with `--no-fetch` R4 passes vacuously against the stale ref;
/// the default run fetches the root, sees the branch is 1 behind, and R4 fails.
/// Regression for the root never being fetched (stale-ref false green).
#[test]
fn logoff_fetches_root_before_behind_check() {
    let r = repo_with_remote("rootfetch", "main");
    // A second clone advances the remote's main by one pushed commit, so the
    // first work clone's origin/main is now stale (still at the old tip).
    let other = temp_dir("rootfetch-other");
    git_ok(&other, &["clone", &file_url(&r.bare), "."]);
    std::fs::write(other.join("more.txt"), "x\n").unwrap();
    git_ok(&other, &["add", "."]);
    git_ok(&other, &["commit", "-m", "advance remote"]);
    git_ok(&other, &["push", "origin", "main"]);

    // --no-fetch: stale origin/main == HEAD, so R4 passes vacuously.
    let stale = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(&stale.stdout, &r.work, "not-behind-remote", "true");

    // default (fetch): gkit fetches the root, origin/main advances, R4 fails.
    let fresh = gkit(&r.work, &["logoff", "-v", r.work.to_str().unwrap()]);
    assert_check(&fresh.stdout, &r.work, "not-behind-remote", "false");
    assert_eq!(
        fresh.code,
        1,
        "behind remote after a root fetch should fail:\n{}",
        fresh.all()
    );
}

// ---------------------------------------------------------------- submodule recursion

#[test]
fn logoff_recurses_submodule_postorder() {
    let sup = repo_with_remote("super", "main");
    add_submodule(&sup.work, "submod", "sub");
    let sub_path = sup.work.join("sub");

    let o = gkit(
        &sup.work,
        &["logoff", "-v", "--no-fetch", sup.work.to_str().unwrap()],
    );
    // Two repos evaluated; both clean.
    assert_check(&o.stdout, &sub_path, "correct-branch", "true");
    assert_check(&o.stdout, &sup.work, "correct-branch", "true");
    assert_eq!(o.code, 0, "fresh super+submodule should pass:\n{}", o.all());

    // Post-order: the submodule RESULT line comes before the superproject's.
    // Match the path's last component (normalize `\` so it works on Windows too).
    let result_idx = |last: &str| {
        o.stdout.lines().position(|l| {
            l.contains("\tRESULT\t")
                && l.split('\t')
                    .next()
                    .unwrap()
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    == Some(last)
        })
    };
    let sub_i = result_idx("sub").expect("submodule RESULT line");
    let sup_i = result_idx("work").expect("superproject RESULT line");
    assert!(
        sub_i < sup_i,
        "submodule should be reported before superproject:\n{}",
        o.stdout
    );
}

// ---------------------------------------------------------------- stmb

#[test]
fn stmb_dry_run_is_noop() {
    let r = repo_with_remote("stmb-dry", "main");
    git_ok(&r.work, &["checkout", "-b", "feat-x"]);
    let o = gkit(
        &r.work,
        &[
            "stmb",
            "--base",
            "main",
            "--dry-run",
            r.work.to_str().unwrap(),
        ],
    );
    assert_contains(&o.stdout, "stmb plan (1 repo(s)):");
    assert_contains(&o.stdout, "switch to 'main'");
    assert_contains(&o.stdout, "delete 'feat-x'");
    assert_eq!(o.code, 0);
    // branch survived (no-op)
    let b = git(&r.work, &["branch", "--list", "feat-x"]);
    assert!(
        !b.stdout.trim().is_empty(),
        "feat-x should still exist after --dry-run"
    );
}

#[test]
fn stmb_executes_switch_and_delete() {
    let r = repo_with_remote("stmb-go", "main");
    git_ok(&r.work, &["checkout", "-b", "feat-x"]); // at main HEAD -> merged
    let o = gkit(
        &r.work,
        &["stmb", "--yes", "--base", "main", r.work.to_str().unwrap()],
    );
    assert_contains(&o.stdout, "+ git checkout main");
    assert_contains(&o.stdout, "+ git branch -d feat-x");
    assert_contains(&o.stdout, "--- logoff ---");
    // branch deleted
    let b = git(&r.work, &["branch", "--list", "feat-x"]);
    assert!(
        b.stdout.trim().is_empty(),
        "feat-x should be deleted:\n{}",
        o.all()
    );
}

#[test]
fn stmb_skips_dirty_repo() {
    let r = repo_with_remote("stmb-dirty", "main");
    git_ok(&r.work, &["checkout", "-b", "feat-d"]);
    std::fs::write(r.work.join("README.md"), "dirty\n").unwrap();
    let o = gkit(
        &r.work,
        &[
            "stmb",
            "--base",
            "main",
            "--dry-run",
            r.work.to_str().unwrap(),
        ],
    );
    assert_contains(&o.stdout, "skip:");
    assert_contains(&o.stdout, "uncommitted changes");
}

#[test]
fn stmb_on_base_branch_switches_without_delete() {
    // Already on base -> plan switches/pulls but deletes nothing.
    let r = repo_with_remote("stmb-onbase", "main");
    let o = gkit(
        &r.work,
        &[
            "stmb",
            "--base",
            "main",
            "--dry-run",
            r.work.to_str().unwrap(),
        ],
    );
    assert_contains(&o.stdout, "switch to 'main'");
    assert!(
        !o.stdout.contains("delete '"),
        "on base, nothing should be deleted:\n{}",
        o.stdout
    );
}

#[test]
fn stmb_skips_detached_head() {
    let r = repo_with_remote("stmb-detached", "main");
    git_ok(&r.work, &["checkout", "--detach", "HEAD"]);
    let o = gkit(
        &r.work,
        &[
            "stmb",
            "--base",
            "main",
            "--dry-run",
            r.work.to_str().unwrap(),
        ],
    );
    assert_contains(&o.stdout, "skip:");
    assert_contains(&o.stdout, "detached HEAD");
}

#[test]
fn stmb_resolves_base_from_config() {
    // No --base: stmb resolves gkit.baseBranch (then origin/HEAD).
    let r = repo_with_remote("stmb-cfgbase", "main");
    git_ok(&r.work, &["config", "gkit.baseBranch", "main"]);
    git_ok(&r.work, &["checkout", "-b", "feat-c"]); // at main HEAD -> merged
    let o = gkit(&r.work, &["stmb", "--yes", r.work.to_str().unwrap()]);
    assert_contains(&o.stdout, "+ git checkout main");
    let b = git(&r.work, &["branch", "--list", "feat-c"]);
    assert!(
        b.stdout.trim().is_empty(),
        "feat-c should be deleted (base from config):\n{}",
        o.all()
    );
}

#[test]
fn stmb_refuses_unmerged_then_force_deletes() {
    let r = repo_with_remote("stmb-unmerged", "main");
    git_ok(&r.work, &["checkout", "-b", "feat-u"]);
    std::fs::write(r.work.join("u.txt"), "x\n").unwrap();
    git_ok(&r.work, &["add", "."]);
    git_ok(&r.work, &["commit", "-m", "unmerged work"]); // not merged into main

    // On feat-u, no --force: safe-delete refuses; the branch survives.
    let o = gkit(
        &r.work,
        &["stmb", "--yes", "--base", "main", r.work.to_str().unwrap()],
    );
    assert_contains(&o.all(), "not fully merged");
    assert_eq!(o.code, 1);
    assert!(
        !git(&r.work, &["branch", "--list", "feat-u"])
            .stdout
            .trim()
            .is_empty(),
        "feat-u must survive a refused delete"
    );

    // Back on feat-u, with --force: deleted.
    git_ok(&r.work, &["checkout", "feat-u"]);
    let o = gkit(
        &r.work,
        &[
            "stmb",
            "--yes",
            "--force",
            "--base",
            "main",
            r.work.to_str().unwrap(),
        ],
    );
    assert_contains(&o.stdout, "+ git branch -D feat-u");
    assert!(
        git(&r.work, &["branch", "--list", "feat-u"])
            .stdout
            .trim()
            .is_empty(),
        "feat-u should be force-deleted:\n{}",
        o.all()
    );
}

#[test]
fn stmb_recurses_into_submodule() {
    let sup = repo_with_remote("stmb-rsuper", "main");
    add_submodule(&sup.work, "stmb-rsub", "sub");
    let sub = sup.work.join("sub");
    // A finished feature branch in BOTH the superproject and the submodule.
    git_ok(&sup.work, &["checkout", "-b", "feat-top"]);
    git_ok(&sub, &["checkout", "-b", "feat-sub"]);

    let o = gkit(
        &sup.work,
        &[
            "stmb",
            "--yes",
            "--base",
            "main",
            sup.work.to_str().unwrap(),
        ],
    );
    assert_contains(&o.stdout, "stmb plan (2 repo(s)):");
    // Both repos switched back to main and their feature branch deleted.
    assert!(
        git(&sup.work, &["branch", "--list", "feat-top"])
            .stdout
            .trim()
            .is_empty(),
        "superproject feat-top should be deleted:\n{}",
        o.all()
    );
    assert!(
        git(&sub, &["branch", "--list", "feat-sub"])
            .stdout
            .trim()
            .is_empty(),
        "submodule feat-sub should be deleted:\n{}",
        o.all()
    );
}

#[test]
fn stmb_no_recursive_limits_to_top() {
    let sup = repo_with_remote("stmb-nrsuper", "main");
    add_submodule(&sup.work, "stmb-nrsub", "sub");
    git_ok(&sup.work, &["checkout", "-b", "feat-top"]);
    let o = gkit(
        &sup.work,
        &[
            "stmb",
            "--no-recursive",
            "--base",
            "main",
            "--dry-run",
            sup.work.to_str().unwrap(),
        ],
    );
    // Only the top repo is planned (submodule excluded).
    assert_contains(&o.stdout, "stmb plan (1 repo(s)):");
}

// ---------------------------------------------------------------- clone arg-errors

#[test]
fn clone_bare_errors() {
    let d = temp_dir("clone-bare");
    let o = gkit(&d, &["clone"]);
    assert_contains(&o.stderr, "need at least one conf file");
    assert_eq!(o.code, 2);
}

#[test]
fn clone_directory_arg_errors() {
    let d = temp_dir("clone-dir");
    let o = gkit(&d, &["clone", d.to_str().unwrap()]);
    assert_contains(&o.stderr, "is a directory");
    assert_eq!(o.code, 2);
}

// ---------------------------------------------------------------- clone insteadOf routing

/// Set up a temp HOME with a git_users alias + an already-cloned [[repo]] (so the
/// clone is skipped, no network), returns (home, conf-path).
fn insteadof_fixture(
    tag: &str,
    alias: &str,
    hostname: &str,
    ns: &str,
) -> (std::path::PathBuf, String) {
    let home = temp_dir(tag);
    write_git_users(&home, alias, hostname);
    let repodir = home.join("work/myrepo");
    std::fs::create_dir_all(&repodir).unwrap();
    git_ok(&repodir, &["init", "-q"]);
    let conf = format!(
        "host = \"{alias}\"\nnamespace = \"{ns}\"\n[[repo]]\ndir = '{}'\n",
        repodir.display()
    );
    let cf = write_conf(&home, "io.toml", &conf);
    (home, cf)
}

#[test]
fn clone_writes_namespace_scoped_insteadof_and_include() {
    let (home, cf) = insteadof_fixture("io-home", "myalias", "example.com", "myns");
    let o = gkit_home(&home, &["clone", "--no-direnv", &cf]);
    assert_eq!(
        o.code,
        0,
        "clone (skipping existing repo) should succeed:\n{}",
        o.all()
    );
    // The gkit-owned routing file got the namespace-scoped rule.
    let routing = std::fs::read_to_string(home.join(".gitconfig-gkit")).unwrap_or_default();
    assert_contains(&routing, "myalias:myns/"); // the [url "myalias:myns/"] section
    assert_contains(&routing, "git@example.com:myns/"); // the insteadOf value
                                                        // ~/.gitconfig (isolated) now includes the routing file.
    let global = std::fs::read_to_string(home.join("gitconfig")).unwrap_or_default();
    assert_contains(&global, ".gitconfig-gkit");
}

#[test]
fn clone_no_insteadof_skips_routing() {
    let (home, cf) = insteadof_fixture("io-skip", "myalias", "example.com", "myns");
    let o = gkit_home(&home, &["clone", "--no-direnv", "--no-insteadof", &cf]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert!(
        !home.join(".gitconfig-gkit").exists(),
        "--no-insteadof must not write the routing file"
    );
    let global = std::fs::read_to_string(home.join("gitconfig")).unwrap_or_default();
    assert!(
        !global.contains(".gitconfig-gkit"),
        "no include should be added"
    );
}

// ---------------------------------------------------------------- init

#[test]
fn init_creates_team_conf_by_default() {
    let d = temp_dir("init-team");
    let o = gkit(&d, &["init", "repos.toml"]); // stdin is null -> non-interactive -> team
    assert_contains(&o.stdout, "created repos.toml");
    let text = std::fs::read_to_string(d.join("repos.toml")).unwrap();
    assert_contains(&text, "host");
    assert_contains(&text, "[[repo]]");
    assert_eq!(o.code, 0);
}

#[test]
fn init_refuses_existing_without_force() {
    let d = temp_dir("init-force");
    assert_eq!(gkit(&d, &["init", "repos.toml"]).code, 0);
    let again = gkit(&d, &["init", "repos.toml"]);
    assert_contains(&again.stderr, "already exists");
    assert_eq!(again.code, 2);
    assert_eq!(gkit(&d, &["init", "--force", "repos.toml"]).code, 0);
}

// ---------------------------------------------------------------- stamp

/// Write a conf file under `dir` and return its path (as a CLI arg string). Dirs
/// are embedded as TOML **literal strings** (single-quoted) so a Windows path's
/// backslashes aren't treated as escapes.
fn write_conf(dir: &std::path::Path, name: &str, text: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p.to_str().unwrap().to_string()
}

#[test]
fn stamp_runs_post_clone_config() {
    let r = repo_with_remote("stamp-basic", "main");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n\
         post-clone = [\"git config gkit.baseBranch dev\", \"git config gkit.solo true\"]\n\
         [[repo]]\ndir = '{}'\n",
        r.work.display()
    );
    let cf = write_conf(&r.work, "stamp.toml", &conf);
    let o = gkit(&r.work, &["stamp", "--conf", &cf, "-y"]);
    assert_eq!(o.code, 0, "stamp should succeed:\n{}", o.all());
    assert_contains(&o.stdout, "stamped");
    // The repo now carries the stamped git config the gate reads.
    assert_eq!(
        git(&r.work, &["config", "--local", "gkit.baseBranch"])
            .stdout
            .trim(),
        "dev"
    );
    assert_eq!(
        git(&r.work, &["config", "--local", "gkit.solo"])
            .stdout
            .trim(),
        "true"
    );
}

#[test]
fn stamp_recurses_into_submodules() {
    // The motivating case: a submodule that lacks gkit config (like a chapter added
    // on a feature branch after the initial clone) gets stamped via the conf's
    // `git submodule foreach --recursive` post-clone hook.
    let sup = repo_with_remote("stamp-sup", "main");
    add_submodule(&sup.work, "stamp-sub", "child");
    let child = sup.work.join("child");
    assert!(
        !git(&child, &["config", "--local", "gkit.baseBranch"]).ok,
        "submodule should start with no gkit.baseBranch"
    );

    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n\
         post-clone = [\"git config gkit.baseBranch dev\", \
         \"git submodule foreach --recursive 'git config gkit.baseBranch dev'\"]\n\
         [[repo]]\ndir = '{}'\n",
        sup.work.display()
    );
    let cf = write_conf(&sup.work, "stamp.toml", &conf);
    let o = gkit(&sup.work, &["stamp", "--conf", &cf, "-y"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        git(&sup.work, &["config", "--local", "gkit.baseBranch"])
            .stdout
            .trim(),
        "dev"
    );
    assert_eq!(
        git(&child, &["config", "--local", "gkit.baseBranch"])
            .stdout
            .trim(),
        "dev",
        "the late-added submodule must be stamped too"
    );
}

#[test]
fn stamp_dry_run_changes_nothing() {
    let r = repo_with_remote("stamp-dry", "main");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n\
         post-clone = [\"git config gkit.baseBranch dev\"]\n[[repo]]\ndir = '{}'\n",
        r.work.display()
    );
    let cf = write_conf(&r.work, "stamp.toml", &conf);
    let o = gkit(&r.work, &["stamp", "--conf", &cf, "--dry-run"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_contains(&o.stdout, "stamp plan (conf mode)");
    assert_contains(&o.stdout, "git config gkit.baseBranch dev");
    assert!(
        !git(&r.work, &["config", "--local", "gkit.baseBranch"]).ok,
        "dry-run must not write any config"
    );
}

#[test]
fn stamp_missing_dir_fails() {
    let d = temp_dir("stamp-missing");
    let missing = d.join("nope");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n\
         post-clone = [\"git config gkit.baseBranch dev\"]\n[[repo]]\ndir = '{}'\n",
        missing.display()
    );
    let cf = write_conf(&d, "stamp.toml", &conf);
    let o = gkit(&d, &["stamp", "--conf", &cf, "-y"]);
    assert_eq!(
        o.code,
        1,
        "a missing repo dir must fail the run:\n{}",
        o.all()
    );
    assert_contains(&o.all(), "no such directory");
}

#[test]
fn stamp_no_post_clone_skips() {
    let r = repo_with_remote("stamp-skip", "main");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n[[repo]]\ndir = '{}'\n",
        r.work.display()
    );
    let cf = write_conf(&r.work, "stamp.toml", &conf);
    let o = gkit(&r.work, &["stamp", "--conf", &cf, "-y"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_contains(&o.stdout, "no post-clone hooks");
}

#[test]
fn stamp_conf_no_paths_errors() {
    // conf-mode still requires explicit conf file(s), like `logoff --conf`.
    let d = temp_dir("stamp-conf-bare");
    let o = gkit(&d, &["stamp", "--conf"]);
    assert_contains(&o.stderr, "need at least one conf file");
    assert_eq!(o.code, 2);
}

#[test]
fn stamp_no_arg_in_non_git_dir_fails() {
    // No arg = repo-mode on cwd; a non-git dir fails clearly (not a silent pass).
    // `-y` skips the confirm so the authoritative run executes.
    let d = temp_dir("stamp-bare");
    let o = gkit(&d, &["stamp", "-y"]);
    assert_contains(&o.all(), "not a git repository");
    assert_eq!(o.code, 1);
}

// ---------------------------------------------------------------- stamp repo-mode + back-fill

#[test]
fn stamp_repo_mode_uses_repo_conf() {
    // gkit.conf points stamp at the conf; no-arg run inside the repo re-applies it.
    let r = repo_with_remote("stamp-repo", "main");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n\
         post-clone = [\"git config gkit.baseBranch dev\"]\n[[repo]]\ndir = '{}'\n",
        r.work.display()
    );
    let cf = write_conf(&r.work, "stamp.toml", &conf);
    let abs = std::fs::canonicalize(&cf).unwrap();
    git_ok(&r.work, &["config", "gkit.conf", abs.to_str().unwrap()]);
    let o = gkit(&r.work, &["stamp", "-y"]); // no path arg → cwd repo-mode
    assert_eq!(o.code, 0, "repo-mode should succeed:\n{}", o.all());
    assert_contains(&o.stdout, "stamped");
    assert_eq!(
        git(&r.work, &["config", "--local", "gkit.baseBranch"])
            .stdout
            .trim(),
        "dev"
    );
}

#[test]
fn stamp_repo_mode_dir_arg() {
    // A directory arg (rejected in v0.8.0) is now a valid repo path.
    let r = repo_with_remote("stamp-repo-dir", "main");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n\
         post-clone = [\"git config gkit.baseBranch dev\"]\n[[repo]]\ndir = '{}'\n",
        r.work.display()
    );
    let cf = write_conf(&r.work, "stamp.toml", &conf);
    let abs = std::fs::canonicalize(&cf).unwrap();
    git_ok(&r.work, &["config", "gkit.conf", abs.to_str().unwrap()]);
    let d = temp_dir("stamp-elsewhere");
    let o = gkit(&d, &["stamp", "-y", r.work.to_str().unwrap()]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        git(&r.work, &["config", "--local", "gkit.baseBranch"])
            .stdout
            .trim(),
        "dev"
    );
}

#[test]
fn stamp_repo_mode_unset_conf_fails() {
    // No gkit.conf set → repo-mode fails with an actionable hint.
    let r = repo_with_remote("stamp-noconf", "main");
    let o = gkit(&r.work, &["stamp", "-y"]);
    assert_eq!(o.code, 1, "unset gkit.conf must fail:\n{}", o.all());
    assert_contains(&o.all(), "gkit.conf not set");
    assert_contains(&o.all(), "gkit stamp --conf");
}

#[test]
fn stamp_repo_mode_dry_run_changes_nothing() {
    let r = repo_with_remote("stamp-repo-dry", "main");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n\
         post-clone = [\"git config gkit.baseBranch dev\"]\n[[repo]]\ndir = '{}'\n",
        r.work.display()
    );
    let cf = write_conf(&r.work, "stamp.toml", &conf);
    let abs = std::fs::canonicalize(&cf).unwrap();
    git_ok(&r.work, &["config", "gkit.conf", abs.to_str().unwrap()]);
    let o = gkit(&r.work, &["stamp", "--dry-run"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_contains(&o.stdout, "stamp plan (repo mode)");
    assert!(
        !git(&r.work, &["config", "--local", "gkit.baseBranch"]).ok,
        "dry-run must not write config"
    );
}

#[test]
fn stamp_repo_mode_recurses_via_hook() {
    // Repo-mode honors the conf's `submodule foreach --recursive` post-clone hook.
    let sup = repo_with_remote("stamp-repo-sup", "main");
    add_submodule(&sup.work, "stamp-repo-sub", "child");
    let child = sup.work.join("child");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n\
         post-clone = [\"git config gkit.baseBranch dev\", \
         \"git submodule foreach --recursive 'git config gkit.baseBranch dev'\"]\n\
         [[repo]]\ndir = '{}'\n",
        sup.work.display()
    );
    let cf = write_conf(&sup.work, "stamp.toml", &conf);
    let abs = std::fs::canonicalize(&cf).unwrap();
    git_ok(&sup.work, &["config", "gkit.conf", abs.to_str().unwrap()]);
    let o = gkit(&sup.work, &["stamp", "-y"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        git(&child, &["config", "--local", "gkit.baseBranch"])
            .stdout
            .trim(),
        "dev"
    );
}

#[test]
fn stamp_conf_mode_backfills_gkit_conf() {
    // conf-mode sets gkit.conf (absolute) where missing, never overwriting.
    let r = repo_with_remote("stamp-backfill", "main");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n[[repo]]\ndir = '{}'\n",
        r.work.display()
    );
    let cf = write_conf(&r.work, "stamp.toml", &conf);
    assert!(
        !git(&r.work, &["config", "--local", "gkit.conf"]).ok,
        "starts with no gkit.conf"
    );
    let o = gkit(&r.work, &["stamp", "--conf", &cf, "-y"]);
    assert_eq!(o.code, 0, "{}", o.all());
    let abs = std::fs::canonicalize(&cf).unwrap();
    assert_eq!(
        git(&r.work, &["config", "--local", "gkit.conf"])
            .stdout
            .trim(),
        abs.to_str().unwrap(),
        "gkit.conf back-filled to the absolute conf path"
    );
    // A second run does not change it (idempotent / never overwrites).
    let _ = gkit(&r.work, &["stamp", "--conf", &cf, "-y"]);
    assert_eq!(
        git(&r.work, &["config", "--local", "gkit.conf"])
            .stdout
            .trim(),
        abs.to_str().unwrap()
    );
}

// ---------------------------------------------------------------- fixsub

#[test]
fn fixsub_switches_detached_submodule_to_branch() {
    // A detached submodule (à la `submodule update --init`) is switched back onto
    // its branch (.gitmodules has no `branch=`, so SUBMODULE_SWITCH falls back to main).
    let sup = repo_with_remote("fixsub-sup", "main");
    add_submodule(&sup.work, "fixsub-sub", "child");
    let child = sup.work.join("child");
    detach_submodule(&child);
    assert!(
        !git(&child, &["symbolic-ref", "--short", "HEAD"]).ok,
        "precondition: submodule is detached"
    );
    let o = gkit(&sup.work, &["fixsub", "-y"]);
    assert_eq!(o.code, 0, "fixsub should succeed:\n{}", o.all());
    assert_eq!(
        git(&child, &["symbolic-ref", "--short", "HEAD"])
            .stdout
            .trim(),
        "main",
        "submodule is back on its branch"
    );
}

#[test]
fn fixsub_inherits_identity_when_submodule_lacks_one() {
    let sup = repo_with_remote("fixsub-id", "main");
    add_submodule(&sup.work, "fixsub-id-sub", "child");
    let child = sup.work.join("child");
    git_ok(&sup.work, &["config", "user.name", "Root Dev"]);
    git_ok(&sup.work, &["config", "user.email", "root@example.com"]);
    assert!(
        !git(&child, &["config", "--local", "user.name"]).ok,
        "precondition: submodule has no local identity"
    );
    let o = gkit(&sup.work, &["fixsub", "-y"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        git(&child, &["config", "--local", "user.name"])
            .stdout
            .trim(),
        "Root Dev"
    );
    assert_eq!(
        git(&child, &["config", "--local", "user.email"])
            .stdout
            .trim(),
        "root@example.com"
    );
}

#[test]
fn fixsub_does_not_clobber_existing_submodule_identity() {
    // Key safety test: a deliberately-different submodule identity survives.
    let sup = repo_with_remote("fixsub-noclobber", "main");
    add_submodule(&sup.work, "fixsub-noclobber-sub", "child");
    let child = sup.work.join("child");
    git_ok(&sup.work, &["config", "user.name", "Root Dev"]);
    git_ok(&sup.work, &["config", "user.email", "root@example.com"]);
    git_ok(&child, &["config", "user.name", "Sub Owner"]);
    git_ok(&child, &["config", "user.email", "sub@example.com"]);
    let o = gkit(&sup.work, &["fixsub", "-y"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        git(&child, &["config", "--local", "user.name"])
            .stdout
            .trim(),
        "Sub Owner",
        "submodule's own identity is not clobbered"
    );
}

#[test]
fn fixsub_dry_run_changes_nothing() {
    let sup = repo_with_remote("fixsub-dry", "main");
    add_submodule(&sup.work, "fixsub-dry-sub", "child");
    let child = sup.work.join("child");
    detach_submodule(&child);
    let o = gkit(&sup.work, &["fixsub", "--dry-run"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_contains(&o.stdout, "fixsub plan:");
    assert_contains(&o.stdout, "submodule foreach --recursive");
    assert!(
        !git(&child, &["symbolic-ref", "--short", "HEAD"]).ok,
        "dry-run must not re-attach the submodule"
    );
}

#[test]
fn fixsub_idempotent() {
    let sup = repo_with_remote("fixsub-idem", "main");
    add_submodule(&sup.work, "fixsub-idem-sub", "child");
    let child = sup.work.join("child");
    detach_submodule(&child);
    assert_eq!(gkit(&sup.work, &["fixsub", "-y"]).code, 0);
    let o = gkit(&sup.work, &["fixsub", "-y"]); // second run
    assert_eq!(o.code, 0, "idempotent re-run:\n{}", o.all());
    assert_eq!(
        git(&child, &["symbolic-ref", "--short", "HEAD"])
            .stdout
            .trim(),
        "main"
    );
}

#[test]
fn fixsub_non_git_dir_fails() {
    let d = temp_dir("fixsub-nongit");
    let o = gkit(&d, &["fixsub", "-y", d.to_str().unwrap()]);
    assert_contains(&o.all(), "not a git repository");
    assert_eq!(o.code, 1);
}

// ---------------------------------------------------------------- nested submodules

#[test]
fn fixsub_recurses_into_nested_submodules() {
    // super → mid → leaf. Detach the DEPTH-2 leaf; `foreach --recursive` must reach it.
    let sup = repo_with_remote("fixsub-deep", "main");
    let leaf = add_nested_submodule(&sup.work, "fixsub-deep", "mid", "leaf");
    detach_submodule(&leaf);
    assert!(
        !git(&leaf, &["symbolic-ref", "--short", "HEAD"]).ok,
        "precondition: nested leaf is detached"
    );
    let o = gkit(&sup.work, &["fixsub", "-y"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        git(&leaf, &["symbolic-ref", "--short", "HEAD"])
            .stdout
            .trim(),
        "main",
        "depth-2 nested submodule switched back to its branch"
    );
}

#[test]
fn stamp_repo_mode_recurses_into_nested_submodules() {
    // The conf's `foreach --recursive` post-clone hook must reach the depth-2 leaf.
    let sup = repo_with_remote("stamp-deep", "main");
    let leaf = add_nested_submodule(&sup.work, "stamp-deep", "mid", "leaf");
    let conf = format!(
        "host = \"h\"\nnamespace = \"n\"\n\
         post-clone = [\"git submodule foreach --recursive 'git config gkit.baseBranch dev'\"]\n\
         [[repo]]\ndir = '{}'\n",
        sup.work.display()
    );
    let cf = write_conf(&sup.work, "stamp.toml", &conf);
    let abs = std::fs::canonicalize(&cf).unwrap();
    git_ok(&sup.work, &["config", "gkit.conf", abs.to_str().unwrap()]);
    let o = gkit(&sup.work, &["stamp", "-y"]);
    assert_eq!(o.code, 0, "{}", o.all());
    assert_eq!(
        git(&leaf, &["config", "--local", "gkit.baseBranch"])
            .stdout
            .trim(),
        "dev",
        "depth-2 nested submodule got config via foreach --recursive"
    );
}
