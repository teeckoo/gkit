//! Shared helpers for the integration tests — std-only (no dev-deps).
//!
//! Everything runs in a hermetic, network-free environment: a throwaway global
//! gitconfig (committer identity, `init.defaultBranch=main`, and
//! `protocol.file.allow=always` so gkit's own child gits can traverse `file://`
//! submodules), a non-existent system config, and `GIT_TERMINAL_PROMPT=0`. The
//! built binary is invoked via `env!("CARGO_BIN_EXE_gkit")`. Fixtures live under
//! `std::env::temp_dir()` (honors `$TMPDIR`; never `/tmp`) with unique names so
//! Cargo's parallel test threads don't collide.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

static COUNTER: AtomicU32 = AtomicU32::new(0);
static GLOBAL_CFG: OnceLock<PathBuf> = OnceLock::new();

/// Per-process root holding the throwaway gitconfig + all fixtures.
fn session_root() -> PathBuf {
    std::env::temp_dir().join(format!("gkit-it-{}", std::process::id()))
}

/// Path to the throwaway global gitconfig (written once).
fn global_gitconfig() -> &'static Path {
    GLOBAL_CFG.get_or_init(|| {
        let root = session_root();
        std::fs::create_dir_all(&root).unwrap();
        let cfg = root.join("gitconfig");
        std::fs::write(
            &cfg,
            "[user]\n\tname = gkit test\n\temail = gkit-test@example.com\n\
             [init]\n\tdefaultBranch = main\n\
             [protocol \"file\"]\n\tallow = always\n\
             [advice]\n\tdetachedHead = false\n",
        )
        .unwrap();
        cfg
    })
}

fn apply_env(c: &mut Command) {
    c.env("GIT_CONFIG_GLOBAL", global_gitconfig());
    c.env(
        "GIT_CONFIG_SYSTEM",
        session_root().join("no-such-system-config"),
    );
    c.env("GIT_TERMINAL_PROMPT", "0");
    c.env_remove("GIT_DIR");
    c.env_remove("GIT_WORK_TREE");
    // Tests never feed stdin; null it so `init`'s is_terminal() prompt never fires
    // (and a stray prompt can't block the suite when run from a real terminal).
    c.stdin(Stdio::null());
}

pub struct Out {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
    pub ok: bool,
}

impl Out {
    /// stdout + stderr concatenated (for substring assertions).
    pub fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn run(mut c: Command) -> Out {
    let o = c.output().expect("failed to spawn process");
    Out {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        code: o.status.code().unwrap_or(-1),
        ok: o.status.success(),
    }
}

/// `git -C <dir> <args…>` in the hermetic env.
pub fn git(dir: &Path, args: &[&str]) -> Out {
    let mut c = Command::new("git");
    c.arg("-C").arg(dir).args(args);
    apply_env(&mut c);
    run(c)
}

/// Like [`git`] but panics on failure (for fixture setup — any failure is a bug).
pub fn git_ok(dir: &Path, args: &[&str]) {
    let o = git(dir, args);
    assert!(
        o.ok,
        "git {args:?} failed in {}:\n{}",
        dir.display(),
        o.all()
    );
}

/// Run the built `gkit` binary with `cwd`, capturing output + exit code.
pub fn gkit(cwd: &Path, args: &[&str]) -> Out {
    let mut c = Command::new(env!("CARGO_BIN_EXE_gkit"));
    c.current_dir(cwd).args(args);
    apply_env(&mut c); // gkit's child gits inherit GIT_CONFIG_GLOBAL (identity + file://)
    run(c)
}

/// Unique fixture dir for one test (created). `tag` must be filesystem-safe.
pub fn temp_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = session_root().join(format!("{tag}-{n}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Windows-safe `file://` URL from an absolute path: `/a/b` → `file:///a/b`,
/// `C:\a\b` → `file:///C:/a/b`.
pub fn file_url(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

pub struct Repo {
    pub work: PathBuf,
    pub bare: PathBuf,
}

/// A bare "remote" plus a working clone with one commit on `default_branch`,
/// pushed with upstream set. Clean + fully pushed + branches-have-remote true.
pub fn repo_with_remote(tag: &str, default_branch: &str) -> Repo {
    let base = temp_dir(tag);
    let bare = base.join("remote.git");
    let work = base.join("work");
    git_ok(
        &base,
        &[
            "init",
            "--bare",
            "-b",
            default_branch,
            bare.to_str().unwrap(),
        ],
    );
    git_ok(&base, &["clone", &file_url(&bare), work.to_str().unwrap()]);
    std::fs::write(work.join("README.md"), "init\n").unwrap();
    git_ok(&work, &["add", "."]);
    git_ok(&work, &["commit", "-m", "init"]);
    git_ok(&work, &["push", "-u", "origin", default_branch]);
    Repo { work, bare }
}

/// Add a real submodule to `super_work` from a fresh second bare repo, commit the
/// gitlink + .gitmodules, push, and ensure the submodule is on its `main` branch
/// (not detached). Returns the submodule's bare path.
pub fn add_submodule(super_work: &Path, sub_tag: &str, path_in_super: &str) -> PathBuf {
    let sub = repo_with_remote(sub_tag, "main");
    git_ok(
        super_work,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            &file_url(&sub.bare),
            path_in_super,
        ],
    );
    // A fresh `submodule add` may leave the submodule detached; put it on main.
    git_ok(&super_work.join(path_in_super), &["checkout", "main"]);
    git_ok(super_work, &["commit", "-m", "add submodule"]);
    git_ok(super_work, &["push", "origin", "HEAD"]);
    sub.bare
}

/// Build a depth-2 submodule chain `super_work` → `mid_path` → `leaf_path`: the
/// mid-level repo itself contains the leaf as a submodule, so the leaf is a
/// submodule-of-a-submodule. Returns the leaf's working path inside the superproject
/// (`super_work/mid_path/leaf_path`).
pub fn add_nested_submodule(
    super_work: &Path,
    tag: &str,
    mid_path: &str,
    leaf_path: &str,
) -> PathBuf {
    let leaf = repo_with_remote(&format!("{tag}-leaf"), "main");
    let mid = repo_with_remote(&format!("{tag}-mid"), "main");
    let leaf_url = file_url(&leaf.bare);
    let mid_url = file_url(&mid.bare);
    let af = "protocol.file.allow=always";

    // mid contains leaf
    git_ok(
        &mid.work,
        &["-c", af, "submodule", "add", &leaf_url, leaf_path],
    );
    git_ok(&mid.work.join(leaf_path), &["checkout", "main"]);
    git_ok(&mid.work, &["commit", "-m", "add leaf"]);
    git_ok(&mid.work, &["push", "origin", "HEAD"]);

    // super contains mid (and, recursively, leaf)
    git_ok(
        super_work,
        &["-c", af, "submodule", "add", &mid_url, mid_path],
    );
    git_ok(
        super_work,
        &["-c", af, "submodule", "update", "--init", "--recursive"],
    );
    git_ok(&super_work.join(mid_path), &["checkout", "main"]);
    git_ok(super_work, &["commit", "-m", "add mid"]);
    git_ok(super_work, &["push", "origin", "HEAD"]);

    super_work.join(mid_path).join(leaf_path)
}

/// Put an initialized submodule into **detached HEAD** at its current commit —
/// simulating what `git submodule update --init` leaves behind (checks out the
/// pinned SHA, not a branch).
pub fn detach_submodule(sub_work: &Path) {
    git_ok(sub_work, &["checkout", "--detach"]);
}

/// True if any line of `out` is `<path>\t<check>\t<value>` for `repo` — matched by
/// the path's **last component** (robust to OS path rendering / canonicalization).
pub fn has_check(out: &str, repo: &Path, check: &str, value: &str) -> bool {
    let want = repo.file_name().unwrap().to_string_lossy();
    out.lines().any(|l| {
        let cols: Vec<&str> = l.split('\t').collect();
        cols.len() >= 3
            && cols[1] == check
            && cols[2] == value
            && cols[0]
                .replace('\\', "/")
                .rsplit('/')
                .next()
                .map(|last| last == want)
                .unwrap_or(false)
    })
}

/// Convenience: assert a check line is present (panics with the full output).
pub fn assert_check(out: &str, repo: &Path, check: &str, value: &str) {
    assert!(
        has_check(out, repo, check, value),
        "expected `{}\\t{check}\\t{value}` in:\n{out}",
        repo.file_name().unwrap().to_string_lossy()
    );
}

/// Assert `needle` appears in `hay` (panics with the full text).
pub fn assert_contains(hay: &str, needle: &str) {
    assert!(
        hay.contains(needle),
        "expected to contain `{needle}` in:\n{hay}"
    );
}
