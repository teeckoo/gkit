//! Fix submodule metadata over an existing tree — a generalized port of the zsh
//! `fixSubModuleMeta`. Applied recursively to every initialized submodule:
//!
//! 1. **Un-detach (branch reconcile):** `git submodule update --init` checks out the
//!    pinned commit in **detached HEAD** (the gitlink is a SHA, not a branch). `fixsub`
//!    switches a submodule onto its declared `.gitmodules` branch **only when it is in
//!    detached HEAD** — the genuine post-clone case. A submodule deliberately on a
//!    *named* branch (active feature work) is **left alone and the divergence is
//!    reported**, never silently switched. Every outcome is printed per submodule
//!    (no swallowed `git switch` — gkit's "every side effect is visible" rule). There is
//!    deliberately **no `--force`/`--switch-all`**: bulk-yanking feature branches is the
//!    footgun we removed from `stmb`; to move one submodule by hand, `git switch` in it.
//! 2. **Identity inherit (set-if-unset):** a submodule added after `gkit clone` misses
//!    the identity stamp. `fixsub` copies the **root** repo's local `user.name`/`user.email`
//!    into each submodule that has **no local identity** — never clobbering a deliberately-
//!    different one.
//!
//! (Optionally `direnv allow` each submodule with an `.envrc`, after the branch reconcile
//! re-points the working tree.)
//!
//! **Project-specific** config (e.g. `core.hooksPath`) is intentionally NOT here — that
//! belongs in the conf's `post-clone`, re-applied by `gkit stamp`. `fixsub` only does
//! universal git/submodule hygiene. (Note: `clone` still uses `clone::SUBMODULE_SWITCH`
//! to un-detach right after `submodule update --init`, where everything is detached and a
//! switch is unambiguously correct — that path is unchanged.)

use crate::clone::sh_squote;
use crate::config::current_branch_opt;
use crate::git::Git;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Ran the submodule fixes (the tree had submodules).
    Fixed,
    /// Nothing to do (no initialized submodules).
    Skipped,
    /// Not a git repo, a `submodule foreach` failed, or a submodule couldn't be
    /// un-detached onto its `.gitmodules` branch.
    Failed(String),
}

#[derive(Debug)]
pub struct FixsubReport {
    pub root: PathBuf,
    pub outcome: Outcome,
}

/// What to do with one submodule's checkout, decided purely from its current branch
/// (`None` = detached HEAD) and its declared `.gitmodules` branch. This is the whole
/// "un-detach only, never yank a named branch" policy in one testable function.
#[derive(Debug, PartialEq, Eq)]
pub enum SwitchPlan {
    /// Detached HEAD → switch onto the configured branch.
    Switch { to: String },
    /// Already on the configured branch → nothing to do.
    Keep { branch: String },
    /// On a *different* named branch → report the divergence, do NOT switch.
    Diverged { on: String, configured: String },
}

/// Decide the branch action for a submodule. **Only** an un-detach (detached →
/// configured) ever mutates; a named branch is kept (if it matches) or reported as
/// diverged (if it doesn't) — fixsub never moves a named branch.
pub fn decide_switch(current: Option<&str>, configured: &str) -> SwitchPlan {
    match current {
        None => SwitchPlan::Switch {
            to: configured.to_string(),
        },
        Some(b) if b == configured => SwitchPlan::Keep {
            branch: b.to_string(),
        },
        Some(b) => SwitchPlan::Diverged {
            on: b.to_string(),
            configured: configured.to_string(),
        },
    }
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

/// One initialized submodule, with the facts needed to resolve its `.gitmodules`
/// branch: `displaypath` (relative to `root`, for locating it) + `name` + `toplevel`
/// (its immediate parent superproject, whose `.gitmodules` declares the branch).
struct SubInfo {
    displaypath: String,
    name: String,
    toplevel: String,
}

/// Enumerate every initialized submodule recursively, with name + parent toplevel.
/// One `submodule foreach --recursive` that just `printf`s tab-separated fields; git's
/// own `Entering '…'` lines (no tabs) are naturally filtered out by the field split, so
/// this is robust without relying on `--quiet`.
fn submodule_infos(git: &dyn Git, root: &Path) -> Result<Vec<SubInfo>, String> {
    let out = git.run(
        root,
        &[
            "submodule",
            "foreach",
            "--recursive",
            r#"printf '%s\t%s\t%s\n' "$displaypath" "$name" "$toplevel""#,
        ],
    );
    if !out.success {
        return Err(format!(
            "submodule foreach (enumerate) failed: {}",
            out.stderr.trim()
        ));
    }
    Ok(out
        .stdout
        .lines()
        .filter_map(|line| {
            let mut it = line.splitn(3, '\t');
            let displaypath = it.next()?.trim();
            let name = it.next()?;
            let toplevel = it.next()?;
            (!displaypath.is_empty()).then(|| SubInfo {
                displaypath: displaypath.to_string(),
                name: name.to_string(),
                toplevel: toplevel.to_string(),
            })
        })
        .collect())
}

/// The branch a submodule declares in its parent's `.gitmodules`, defaulting to `main`
/// (git's own default for an unspecified submodule branch).
fn configured_branch(git: &dyn Git, root: &Path, toplevel: &str, name: &str) -> String {
    let gitmodules = format!("{toplevel}/.gitmodules");
    let key = format!("submodule.{name}.branch");
    let o = git.run(root, &["config", "-f", &gitmodules, "--get", &key]);
    let v = o.trimmed();
    if o.success && !v.is_empty() {
        v.to_string()
    } else {
        "main".to_string()
    }
}

/// Un-detach + identity-inherit (+ optional `direnv allow`) over the submodule tree
/// rooted at `root`. Prints a per-submodule outcome for the branch reconcile and the
/// git commands it runs; idempotent. `dry_run` prints the plan and mutates nothing.
pub fn fixsub<G: Git>(git: &G, root: &Path, dry_run: bool, direnv: bool) -> FixsubReport {
    let mk = |outcome| FixsubReport {
        root: root.to_path_buf(),
        outcome,
    };
    if !is_git_repo(git, root) {
        return mk(Outcome::Failed("not a git repository".into()));
    }

    let subs = match submodule_infos(git, root) {
        Ok(s) => s,
        Err(e) => return mk(Outcome::Failed(e)),
    };
    if subs.is_empty() {
        println!("  no initialized submodules — nothing to do");
        return mk(Outcome::Skipped);
    }

    // Phase 1 — branch reconcile: un-detach detached heads onto their `.gitmodules`
    // branch; keep / report (never yank) named branches. Every outcome printed.
    let mut switch_failures = 0u32;
    for s in &subs {
        let dir = root.join(&s.displaypath);
        let configured = configured_branch(git, root, &s.toplevel, &s.name);
        let current = current_branch_opt(git, &dir);
        match decide_switch(current.as_deref(), &configured) {
            SwitchPlan::Switch { to } => {
                if dry_run {
                    println!(
                        "  {}: detached HEAD → would switch to '{to}'",
                        s.displaypath
                    );
                } else {
                    println!("  {}: detached HEAD → switching to '{to}'", s.displaypath);
                    println!("    + git switch {to}");
                    let o = git.run(&dir, &["switch", &to]);
                    if !o.success {
                        println!("    ! switch to '{to}' FAILED: {}", o.stderr.trim());
                        switch_failures += 1;
                    }
                }
            }
            SwitchPlan::Keep { branch } => {
                println!(
                    "  {}: on '{branch}' (matches .gitmodules) — kept",
                    s.displaypath
                );
            }
            SwitchPlan::Diverged { on, configured } => {
                println!(
                    "  {}: on '{on}'; .gitmodules tracks '{configured}' — left as-is \
                     (merge it into '{configured}', or update .gitmodules)",
                    s.displaypath
                );
            }
        }
    }

    // Phase 2 — identity-inherit (set-if-unset) + optional direnv, over the tree. Runs
    // after the reconcile so `direnv allow` sees the (possibly) re-pointed working tree.
    let name = local_config(git, root, "user.name");
    let email = local_config(git, root, "user.email");
    let mut parts: Vec<String> = Vec::new();
    if let Some(id) = inherit_identity_cmd(name.as_deref(), email.as_deref()) {
        parts.push(id);
    }
    if direnv {
        parts.push("[ -f .envrc ] && direnv allow . 2>/dev/null || true".to_string());
    }
    if !parts.is_empty() {
        let body = parts.join("; ");
        println!("+ git submodule foreach --recursive {body}");
        if !dry_run {
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
        }
    }

    if switch_failures > 0 {
        return mk(Outcome::Failed(format!(
            "{switch_failures} submodule(s) could not be switched onto their .gitmodules branch (see output above)"
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
    fn decide_switch_un_detaches_only_and_reports_divergence() {
        // detached → switch to configured
        assert_eq!(
            decide_switch(None, "main"),
            SwitchPlan::Switch { to: "main".into() }
        );
        // on the configured branch → keep
        assert_eq!(
            decide_switch(Some("main"), "main"),
            SwitchPlan::Keep {
                branch: "main".into()
            }
        );
        // on a different named branch → diverged, NOT switched
        assert_eq!(
            decide_switch(Some("feature-x"), "main"),
            SwitchPlan::Diverged {
                on: "feature-x".into(),
                configured: "main".into()
            }
        );
    }

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
        // is_git_repo true + no submodules → fixsub queries (enumerate) but must NOT
        // call `git switch` / the mutating foreach. Returns Skipped.
        let git = FakeGit::new()
            .ok("rev-parse --is-inside-work-tree", "true")
            .ok(
                r#"submodule foreach --recursive printf '%s\t%s\t%s\n' "$displaypath" "$name" "$toplevel""#,
                "",
            );
        let r = fixsub(&git, Path::new("/r"), true, true);
        assert_eq!(r.outcome, Outcome::Skipped);
    }

    #[test]
    fn non_git_root_fails() {
        let git = FakeGit::new().fail("rev-parse --is-inside-work-tree");
        let r = fixsub(&git, Path::new("/nope"), false, true);
        assert!(matches!(r.outcome, Outcome::Failed(_)));
    }
}
