//! Network integration tests — **opt-in**, run only with `GKIT_NET_TESTS=1`.
//!
//! These prove the ssh-free real-world path: clone PUBLIC GitHub repos over
//! **HTTPS** (no ssh key — works on CI runners unauthenticated) and run `gkit
//! logoff` against them. `gkit clone` itself can't be exercised here (it emits
//! ssh-alias `host:ns/repo.git` URLs), so we clone with plain `git`.
//!
//! Strategy: clone a public repo once, then drive every logoff scenario by
//! **mutating the local clone** (dirty / unpushed / detached / local feature /
//! base config / `gkit.solo`), resetting to pristine between phases. This is
//! deterministic regardless of how the upstream repo's content moves.
//!
//! Pinned public repos:
//! - `octocat/Hello-World` — no submodules; its remote has feature branches
//!   (`test`, `octocat-patch-1`), which makes it a real test for the solo rule.
//! - `git-fixtures/submodule` — a stable fixture with an HTTPS submodule
//!   (`basic`); used to prove submodule recursion over HTTPS.

mod common;
use common::*;
use std::path::{Path, PathBuf};

const HELLO: &str = "https://github.com/octocat/Hello-World.git";
const SUBMOD: &str = "https://github.com/git-fixtures/submodule.git";

fn gated() -> bool {
    if std::env::var("GKIT_NET_TESTS").as_deref() == Ok("1") {
        true
    } else {
        eprintln!("skipping network test (set GKIT_NET_TESTS=1 to run)");
        false
    }
}

fn clone_public(repo: &str, tag: &str) -> PathBuf {
    let base = temp_dir(tag);
    let work = base.join("repo");
    git_ok(&base, &["clone", repo, work.to_str().unwrap()]);
    work
}

/// Restore a Hello-World clone to a pristine `master` checkout between phases.
fn pristine(work: &Path) {
    git_ok(work, &["checkout", "-qf", "master"]);
    git_ok(work, &["reset", "--hard", "-q", "origin/master"]);
    let _ = git(work, &["clean", "-fdq"]);
    // Drop any stray local branch (e.g. from a previous phase).
    let heads = git(
        work,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/*"],
    )
    .stdout;
    for b in heads.lines().map(str::trim).filter(|b| !b.is_empty()) {
        if b != "master" {
            let _ = git(work, &["branch", "-D", b]);
        }
    }
    let _ = git(work, &["config", "--unset", "gkit.solo"]);
    let _ = git(work, &["config", "--unset", "gkit.baseBranch"]);
}

fn logoff_v(work: &Path) -> Out {
    gkit(
        work,
        &["logoff", "-v", "--no-fetch", work.to_str().unwrap()],
    )
}

#[test]
fn net_logoff_scenarios_no_submodules() {
    if !gated() {
        return;
    }
    let work = clone_public(HELLO, "net-hw");

    // 1. Pristine clone on master -> all checks pass (team rule; only `master`
    //    is local, others' feature branches are remote-tracking and ignored).
    pristine(&work);
    let o = logoff_v(&work);
    assert_check(&o.stdout, &work, "committed", "true");
    assert_check(&o.stdout, &work, "all-commits-pushed", "true");
    assert_check(&o.stdout, &work, "branches-have-remote", "true");
    assert_check(
        &o.stdout,
        &work,
        "base-branch",
        "master (derived from remote origin/master)",
    );
    assert_check(&o.stdout, &work, "correct-branch", "true");
    assert!(
        !o.stdout.contains("branch-rule"),
        "team rule is silent:\n{}",
        o.stdout
    );
    assert_eq!(o.code, 0, "pristine clone should pass:\n{}", o.all());

    // 2. Uncommitted change -> committed false.
    pristine(&work);
    std::fs::write(work.join("DIRTY.txt"), "x\n").unwrap();
    let o = logoff_v(&work);
    assert_check(&o.stdout, &work, "committed", "false");
    assert_eq!(o.code, 1);

    // 3. Local commit not on any remote -> all-commits-pushed false.
    pristine(&work);
    std::fs::write(work.join("local.txt"), "x\n").unwrap();
    git_ok(&work, &["add", "."]);
    git_ok(&work, &["commit", "-m", "local only"]);
    let o = logoff_v(&work);
    assert_check(&o.stdout, &work, "all-commits-pushed", "false");
    assert_eq!(o.code, 1);

    // 4. Local branch with no remote counterpart -> branches-have-remote false.
    pristine(&work);
    git_ok(&work, &["checkout", "-q", "-b", "local-only"]);
    std::fs::write(work.join("f.txt"), "x\n").unwrap();
    git_ok(&work, &["add", "."]);
    git_ok(&work, &["commit", "-m", "wip"]);
    let o = logoff_v(&work);
    assert_check(&o.stdout, &work, "branches-have-remote", "false");
    assert_eq!(o.code, 1);

    // 5. Detached HEAD -> correct-branch false.
    pristine(&work);
    git_ok(&work, &["checkout", "-q", "--detach", "HEAD"]);
    let o = logoff_v(&work);
    assert_check(&o.stdout, &work, "correct-branch", "false");
    assert_eq!(o.code, 1);

    // 6. Local feature branch unmerged into base (team rule) -> correct-branch false.
    pristine(&work);
    git_ok(&work, &["checkout", "-q", "-b", "feature-x"]);
    std::fs::write(work.join("feat.txt"), "x\n").unwrap();
    git_ok(&work, &["add", "."]);
    git_ok(&work, &["commit", "-m", "feature"]);
    git_ok(&work, &["checkout", "-q", "master"]);
    let o = logoff_v(&work);
    assert_check(&o.stdout, &work, "correct-branch", "false");
    assert_eq!(o.code, 1);

    // 7. base-branch from git config.
    pristine(&work);
    git_ok(&work, &["config", "gkit.baseBranch", "dev"]);
    let o = logoff_v(&work);
    assert_check(
        &o.stdout,
        &work,
        "base-branch",
        "dev (from git config gkit.baseBranch)",
    );

    // 8. SOLO rule on a real repo: the remote has feature branches (`test`,
    //    `octocat-patch-1`), so on `master` the solo rule FAILS where team passes.
    pristine(&work);
    git_ok(&work, &["config", "gkit.solo", "true"]);
    let o = logoff_v(&work);
    assert_check(
        &o.stdout,
        &work,
        "branch-rule",
        "solo (gkit.solo on) — flags any feature branch on the remote",
    );
    assert_check(&o.stdout, &work, "correct-branch", "false");
    assert_eq!(
        o.code,
        1,
        "solo rule should fail (remote has feature branches):\n{}",
        o.all()
    );

    // 8b. Same repo, solo off -> team rule passes (the contrast).
    pristine(&work);
    let o = logoff_v(&work);
    assert_check(&o.stdout, &work, "correct-branch", "true");
    assert_eq!(
        o.code,
        0,
        "team rule passes where solo failed:\n{}",
        o.all()
    );
}

#[test]
fn net_stmb_switch_and_delete() {
    if !gated() {
        return;
    }
    // stmb against a real public repo over HTTPS: it checks out base, `pull
    // --rebase origin <base>` (a read-only fetch — no push), deletes the merged
    // feature branch, then verifies with a logoff. No ssh needed.
    let work = clone_public(HELLO, "net-stmb");
    git_ok(&work, &["checkout", "-q", "-b", "feat-net"]); // at master HEAD -> merged

    let o = gkit(
        &work,
        &["stmb", "--yes", "--base", "master", work.to_str().unwrap()],
    );
    assert_contains(&o.stdout, "+ git checkout master");
    assert_contains(&o.stdout, "+ git branch -d feat-net");
    assert_contains(&o.stdout, "--- logoff ---");
    assert!(
        git(&work, &["branch", "--list", "feat-net"])
            .stdout
            .trim()
            .is_empty(),
        "feat-net should be deleted:\n{}",
        o.all()
    );
    assert_eq!(
        o.code,
        0,
        "stmb + verifying logoff should pass:\n{}",
        o.all()
    );
}

#[test]
fn net_logoff_submodule_recursion() {
    if !gated() {
        return;
    }
    // Clone the superproject (without --recurse), then init ONLY the `basic`
    // submodule over HTTPS (the fixture's other submodule `itself` is
    // self-referential — leave it uninitialized so recursion is bounded).
    let base = temp_dir("net-sub");
    let sup = base.join("super");
    git_ok(&base, &["clone", SUBMOD, sup.to_str().unwrap()]);
    git_ok(&sup, &["submodule", "update", "--init", "basic"]);
    let basic = sup.join("basic");

    // Phase 1: a freshly-updated submodule is at a DETACHED HEAD — gkit recurses
    // into it (post-order: submodule before superproject) and flags it.
    let o = logoff_v(&sup);
    assert_check(&o.stdout, &basic, "correct-branch", "false"); // detached
    assert!(
        result_index(&o.stdout, "/basic") < result_index(&o.stdout, "/super"),
        "submodule should be reported before superproject:\n{}",
        o.stdout
    );
    assert_eq!(
        o.code,
        1,
        "detached submodule should fail the gate:\n{}",
        o.all()
    );

    // Phase 2: put the submodule on its branch (as `gkit clone` would) -> clean
    // recursion pass over an HTTPS-cloned submodule.
    git_ok(&basic, &["checkout", "-q", "master"]);
    let o = logoff_v(&sup);
    assert_check(&o.stdout, &basic, "correct-branch", "true");
    assert_check(&o.stdout, &sup, "correct-branch", "true");
    assert_eq!(
        o.code,
        0,
        "super + on-branch submodule should pass:\n{}",
        o.all()
    );
}

/// Index of the `RESULT` line whose path ends with `suffix` (for post-order checks).
fn result_index(out: &str, suffix: &str) -> usize {
    out.lines()
        .position(|l| l.contains("\tRESULT\t") && l.split('\t').next().unwrap().ends_with(suffix))
        .unwrap_or_else(|| panic!("no RESULT line ending in {suffix}:\n{out}"))
}
