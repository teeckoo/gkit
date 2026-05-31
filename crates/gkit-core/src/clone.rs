//! Config-driven clone with explicit flag placement, built-in stateless steps, and
//! pre/post-clone hooks.
//!
//! Per repo, in order: global `pre-clone` → repo `pre-clone` → `git <PRE> clone
//! <POST> <url> <dir>` → built-ins (submodule branch-switch + `direnv allow`) →
//! global `post-clone` → repo `post-clone`.
//!
//! The `git clone` and built-ins are **captured** (clean status; an `.envrc` that
//! runs `glow …` can't distort output — `direnv allow` only records trust). User
//! hooks run via `sh -c` with their output **inherited** (explicit commands, shown
//! live) and `$GKIT_REPO`/`GKIT_DIR`/`GKIT_URL`/`GKIT_HOST`/`GKIT_NAMESPACE` set.

use crate::conf::{expand_path, CloneConf};
use crate::git::Git;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Cloned,
    Skipped,
    Failed(String),
}

#[derive(Debug)]
pub struct CloneReport {
    pub name: String,
    pub dir: PathBuf,
    pub outcome: Outcome,
    pub command: String,
}

pub struct Opts {
    pub submodule_branch: bool,
    pub direnv: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            submodule_branch: true,
            direnv: true,
        }
    }
}

const SUBMODULE_SWITCH: &str = "b=$(git config -f \"$toplevel/.gitmodules\" \"submodule.$name.branch\" 2>/dev/null || echo main); git switch \"$b\" 2>/dev/null || true";

/// Run hook commands via `sh -c` in `cwd` with `env` set; output inherited; each
/// printed `+ <cmd>`. Stops at the first non-zero exit.
fn run_hooks(cmds: &[String], cwd: &Path, env: &[(&str, &str)]) -> Result<(), String> {
    for cmd in cmds {
        println!("+ {cmd}");
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd).current_dir(cwd);
        for (k, v) in env {
            c.env(k, v);
        }
        match c.status() {
            Ok(s) if s.success() => {}
            Ok(s) => return Err(format!("hook `{cmd}` exited {}", s.code().unwrap_or(-1))),
            Err(e) => return Err(format!("hook `{cmd}` failed to start: {e}")),
        }
    }
    Ok(())
}

/// Clone every repo in `conf`, printing each step in order. Returns a report per
/// repo (for the aggregate exit code).
pub fn clone_all<G: Git>(git: &G, conf: &CloneConf, opts: &Opts) -> Vec<CloneReport> {
    conf.repo
        .iter()
        .map(|r| {
            let name = r.name();
            let dir_s = expand_path(&r.dir, |k| std::env::var(k).ok());
            let dir = PathBuf::from(&dir_s);
            // Per-repo namespace overrides the global one; `clone_cmd` validates this
            // up front, so `None` here is a defensive backstop, not a normal path.
            let ns = match conf.namespace_for(r) {
                Some(n) => n.to_string(),
                None => {
                    let e = format!("no namespace for {}", r.dir);
                    println!("FAILED   {name:<28} {e}");
                    return CloneReport {
                        name,
                        dir,
                        outcome: Outcome::Failed(e),
                        command: String::new(),
                    };
                }
            };
            let url = format!("{}:{}/{}.git", conf.host, ns, name);

            // git <git-flags> clone <depth/branch> --recurse-submodules <clone-flags> <repo flags> <url> <dir>
            let mut args: Vec<String> = Vec::new();
            args.extend(conf.git_flags.iter().cloned());
            args.push("clone".into());
            if let Some(d) = r.depth {
                args.push("--depth".into());
                args.push(d.to_string());
            }
            if let Some(b) = &r.branch {
                args.push("--branch".into());
                args.push(b.clone());
                args.push("--single-branch".into());
            }
            args.push("--recurse-submodules".into());
            args.extend(conf.clone_flags.iter().cloned());
            args.extend(r.clone_flags.iter().cloned());
            args.push(url.clone());
            args.push(dir_s.clone());
            let command = format!("git {}", args.join(" "));

            let mk = |outcome| CloneReport {
                name: name.clone(),
                dir: dir.clone(),
                outcome,
                command: command.clone(),
            };

            if dir.join(".git").exists() {
                println!("+ {command}");
                println!("skipped  {name:<28} {dir_s} (exists)");
                return mk(Outcome::Skipped);
            }

            let env = [
                ("GKIT_REPO", name.as_str()),
                ("GKIT_DIR", dir_s.as_str()),
                ("GKIT_URL", url.as_str()),
                ("GKIT_HOST", conf.host.as_str()),
                ("GKIT_NAMESPACE", ns.as_str()),
            ];

            // 1+2: pre-clone hooks (cwd = parent of target; create it first)
            let parent = dir.parent().unwrap_or(Path::new("."));
            let _ = std::fs::create_dir_all(parent);
            let pre: Vec<String> = conf
                .pre_clone
                .0
                .iter()
                .chain(r.pre_clone.0.iter())
                .cloned()
                .collect();
            if let Err(e) = run_hooks(&pre, parent, &env) {
                println!("FAILED   {name:<28} {e}");
                return mk(Outcome::Failed(e));
            }

            // 3: clone (printed; output captured)
            println!("+ {command}");
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = git.run(Path::new("."), &refs);
            if !out.success {
                let e = out.stderr.trim().to_string();
                println!("FAILED   {name:<28} {}", e.lines().next().unwrap_or(""));
                return mk(Outcome::Failed(e));
            }

            // 4: built-ins (captured)
            if opts.submodule_branch {
                let _ = git.run(
                    &dir,
                    &["submodule", "foreach", "--recursive", SUBMODULE_SWITCH],
                );
            }
            // Stamp the solo workflow flag (conf `solo`, per-repo over global) so
            // `gkit logoff` reads it from `git config gkit.solo`. Printed like any
            // other step; only when the conf sets it.
            if let Some(v) = conf.solo_for(r) {
                let val = if v { "true" } else { "false" };
                println!("+ git config gkit.solo {val}");
                let _ = git.run(&dir, &["config", "gkit.solo", val]);
            }
            if opts.direnv && dir.join(".envrc").exists() {
                let _ = Command::new("direnv").arg("allow").arg(&dir).output(); // trust-only, no eval
            }

            // 5+6: post-clone hooks (cwd = the cloned repo)
            let post: Vec<String> = conf
                .post_clone
                .0
                .iter()
                .chain(r.post_clone.0.iter())
                .cloned()
                .collect();
            if let Err(e) = run_hooks(&post, &dir, &env) {
                println!("FAILED   {name:<28} {e}");
                return mk(Outcome::Failed(e));
            }

            println!("cloned   {name:<28} {dir_s}");
            mk(Outcome::Cloned)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::conf;

    #[test]
    fn builds_expected_url_shape() {
        let c = conf::parse("host = \"tlbb\"\nnamespace = \"example-org\"\n[[repo]]\ndir = \"$HOME/x/cosp\"\ndepth = 1\n").unwrap();
        assert_eq!(c.repo[0].name(), "cosp");
        assert_eq!(c.repo[0].depth, Some(1));
        let ns = c.namespace_for(&c.repo[0]).unwrap();
        let url = format!("{}:{}/{}.git", c.host, ns, c.repo[0].name());
        assert_eq!(url, "tlbb:example-org/cosp.git");
    }

    #[test]
    fn per_repo_namespace_drives_url() {
        let c = conf::parse("host=\"gh\"\n[[repo]]\ndir=\"$HOME/x/foo\"\nnamespace=\"alice\"\n")
            .unwrap();
        let ns = c.namespace_for(&c.repo[0]).unwrap();
        let url = format!("{}:{}/{}.git", c.host, ns, c.repo[0].name());
        assert_eq!(url, "gh:alice/foo.git");
    }
}
