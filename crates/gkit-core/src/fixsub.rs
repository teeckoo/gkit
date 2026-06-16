//! Fix submodule metadata over an existing tree — a generalized port of the zsh
//! `fixSubModuleMeta`. Two universal, idempotent fixes applied recursively to every
//! initialized submodule:
//!
//! 1. **Branch-switch (un-detach):** `git submodule update --init` checks out the
//!    pinned commit in **detached HEAD** (the gitlink is a SHA, not a branch; it
//!    ignores `.gitmodules branch=`). `fixsub` switches each submodule onto its
//!    declared `.gitmodules` branch — reusing clone's `SUBMODULE_SWITCH`.
//! 2. **Identity inherit (set-if-unset):** a submodule added after `gkit clone`
//!    misses the identity stamp. `fixsub` copies the **root** repo's local
//!    `user.name`/`user.email` into each submodule that has **no local identity** —
//!    never clobbering a deliberately-different one.
//!
//! (Optionally `direnv allow` each submodule with an `.envrc`, mirroring clone's
//! direnv built-in, after the branch flip re-points the working tree.)
//!
//! **Project-specific** config (e.g. `core.hooksPath`) is intentionally NOT here —
//! that belongs in the conf's `post-clone`, re-applied by `gkit stamp`. `fixsub` only
//! does universal git/submodule hygiene.

use crate::clone::{sh_squote, SUBMODULE_SWITCH};
use crate::git::Git;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Ran the submodule fixes (the tree had submodules).
    Fixed,
    /// Nothing to do (no submodules / no recursion target).
    Skipped,
    /// Not a git repo, or a `submodule foreach` failed.
    Failed(String),
}

#[derive(Debug)]
pub struct FixsubReport {
    pub root: PathBuf,
    pub outcome: Outcome,
}

/// The `submodule foreach` body that inherits the root's identity **only where the
/// submodule lacks its own** — for each of `user.name`/`user.email` that the root
/// has, emit `git config --local user.X >/dev/null 2>&1 || git config user.X '<val>'`
/// (values single-quoted via [`sh_squote`]). `None` when the root has no identity to
/// inherit (nothing to do).
pub fn inherit_identity_cmd(root_name: Option<&str>, root_email: Option<&str>) -> Option<String> {
    let parts: Vec<String> = [("user.name", root_name), ("user.email", root_email)]
        .into_iter()
        .filter_map(|(k, v)| {
            v.map(|v| {
                format!(
                    "git config --local {k} >/dev/null 2>&1 || git config {k} {}",
                    sh_squote(v)
                )
            })
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Is `dir` inside a git work tree? (Same probe `stamp`/the gate use.)
fn is_git_repo(git: &dyn Git, dir: &Path) -> bool {
    let r = git.run(dir, &["rev-parse", "--is-inside-work-tree"]);
    r.success && r.trimmed() == "true"
}

/// A repo's local config value (`--local --get`), trimmed, or `None`.
fn local_config(git: &dyn Git, dir: &Path, key: &str) -> Option<String> {
    let o = git.run(dir, &["config", "--local", "--get", key]);
    let v = o.trimmed();
    (o.success && !v.is_empty()).then(|| v.to_string())
}

/// Branch-switch + identity-inherit (+ optional `direnv allow`) over the submodule
/// tree rooted at `root`. Prints every git command; idempotent. `dry_run` prints the
/// plan and runs nothing.
pub fn fixsub<G: Git>(git: &G, root: &Path, dry_run: bool, direnv: bool) -> FixsubReport {
    let mk = |outcome| FixsubReport {
        root: root.to_path_buf(),
        outcome,
    };
    if !is_git_repo(git, root) {
        return mk(Outcome::Failed("not a git repository".into()));
    }

    // Build one `submodule foreach --recursive` body: un-detach, then inherit
    // identity where unset, then (optionally) re-trust .envrc. `foreach` visits only
    // initialized submodules and recurses — matching the zsh skip + recursion.
    let name = local_config(git, root, "user.name");
    let email = local_config(git, root, "user.email");
    let mut body = SUBMODULE_SWITCH.to_string();
    if let Some(id) = inherit_identity_cmd(name.as_deref(), email.as_deref()) {
        body.push_str("; ");
        body.push_str(&id);
    }
    if direnv {
        body.push_str("; [ -f .envrc ] && direnv allow . 2>/dev/null || true");
    }

    println!("+ git submodule foreach --recursive {body}");
    if dry_run {
        return mk(Outcome::Fixed);
    }
    let out = git.run(
        root,
        &["submodule", "foreach", "--recursive", body.as_str()],
    );
    if !out.success {
        return mk(Outcome::Failed(format!(
            "submodule foreach failed: {}",
            out.stderr.trim()
        )));
    }
    mk(Outcome::Fixed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::FakeGit;
    use std::path::Path;

    #[test]
    fn inherit_identity_cmd_set_if_unset_and_quotes() {
        // both fields → two guarded, single-quoted clauses joined with `; `
        assert_eq!(
            inherit_identity_cmd(Some("Jane Dev"), Some("jane@acme.com")).as_deref(),
            Some(
                "git config --local user.name >/dev/null 2>&1 || git config user.name 'Jane Dev'; \
                 git config --local user.email >/dev/null 2>&1 || git config user.email 'jane@acme.com'"
            )
        );
        // only one field → just that clause
        assert_eq!(
            inherit_identity_cmd(Some("Jane"), None).as_deref(),
            Some("git config --local user.name >/dev/null 2>&1 || git config user.name 'Jane'")
        );
        // neither → None (caller skips identity)
        assert_eq!(inherit_identity_cmd(None, None), None);
        // embedded single quote is escaped so `sh` can't break out
        assert_eq!(
            inherit_identity_cmd(Some("O'Brien"), None).as_deref(),
            Some(
                r"git config --local user.name >/dev/null 2>&1 || git config user.name 'O'\''Brien'"
            )
        );
    }

    #[test]
    fn dry_run_runs_no_git_mutations() {
        // is_git_repo true, but dry_run → fixsub must NOT call `submodule foreach`
        // (a default FakeGit would fail that unknown call). Returns Fixed.
        let git = FakeGit::new().ok("rev-parse --is-inside-work-tree", "true");
        let r = fixsub(&git, Path::new("/r"), true, true);
        assert_eq!(r.outcome, Outcome::Fixed);
    }

    #[test]
    fn non_git_root_fails() {
        let git = FakeGit::new().fail("rev-parse --is-inside-work-tree");
        let r = fixsub(&git, Path::new("/nope"), false, true);
        assert!(matches!(r.outcome, Outcome::Failed(_)));
    }
}
