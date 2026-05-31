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
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
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
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
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
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
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
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert!(
        o.stdout
            .lines()
            .any(|l| l.contains("base-branch") && l.contains("UNRESOLVED")),
        "expected UNRESOLVED base-branch:\n{}",
        o.stdout
    );
    assert_check(&o.stdout, &r.work, "correct-branch", "false");
    assert_eq!(o.code, 1);
}

#[test]
fn base_branch_flag_override() {
    let r = repo_with_remote("baseflag", "main");
    let o = gkit(
        &r.work,
        &[
            "logoff",
            "-v",
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
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    // Verbose explains the active (non-default) rule on its own line.
    assert_check(
        &o.stdout,
        &r.work,
        "branch-rule",
        "solo (gkit.solo on) — flags any feature branch on the remote",
    );
    assert_check(&o.stdout, &r.work, "correct-branch", "false");
    assert_eq!(o.code, 1);
}

#[test]
fn solo_passes_when_remote_is_integration_only() {
    let r = repo_with_remote("solo-clean", "main");
    git_ok(&r.work, &["config", "gkit.solo", "true"]);
    let o = gkit(
        &r.work,
        &["logoff", "-v", "--no-fetch", r.work.to_str().unwrap()],
    );
    assert_check(
        &o.stdout,
        &r.work,
        "branch-rule",
        "solo (gkit.solo on) — flags any feature branch on the remote",
    );
    assert_check(&o.stdout, &r.work, "correct-branch", "true");
    assert_eq!(
        o.code,
        0,
        "solo + integration-only remote should pass:\n{}",
        o.all()
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
    let result_idx = |name: &str| {
        o.stdout
            .lines()
            .position(|l| l.contains("\tRESULT\t") && l.split('\t').next().unwrap().ends_with(name))
    };
    let sub_i = result_idx("/sub").expect("submodule RESULT line");
    let sup_i = result_idx("/work").expect("superproject RESULT line");
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

// ---------------------------------------------------------------- init

#[test]
fn init_creates_team_conf_by_default() {
    let d = temp_dir("init-team");
    let o = gkit(&d, &["init", "repos.toml"]); // stdin is null -> non-interactive -> team
    assert_contains(&o.stdout, "created repos.toml");
    let text = std::fs::read_to_string(d.join("repos.toml")).unwrap();
    assert_contains(&text, "host");
    assert_contains(&text, "[[repo]]");
    assert_contains(&text, "solo = false");
    assert_eq!(o.code, 0);
}

#[test]
fn init_solo_flag_writes_solo_true() {
    let d = temp_dir("init-solo");
    let o = gkit(&d, &["init", "--solo", "repos.toml"]);
    assert_eq!(o.code, 0);
    let text = std::fs::read_to_string(d.join("repos.toml")).unwrap();
    assert_contains(&text, "solo = true");
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
