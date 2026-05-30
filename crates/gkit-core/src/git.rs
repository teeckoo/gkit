//! Thin abstraction over invoking `git`, so the checks are unit-testable without
//! a real repository. The real impl shells out to `git -C <dir> …`; tests use a
//! `FakeGit` keyed by the command's args.

use std::path::Path;
use std::process::Command;

/// Captured result of a single git invocation.
#[derive(Clone, Debug)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
    /// True when git exited 0.
    pub success: bool,
}

impl GitOutput {
    /// stdout with surrounding whitespace trimmed.
    pub fn trimmed(&self) -> &str {
        self.stdout.trim()
    }
}

/// Anything that can run a git command in a directory.
pub trait Git {
    /// Run `git -C <dir> <args…>` and capture stdout/stderr/exit status.
    fn run(&self, dir: &Path, args: &[&str]) -> GitOutput;
}

/// Real implementation: shells out to the system `git`.
pub struct SystemGit;

impl Git for SystemGit {
    fn run(&self, dir: &Path, args: &[&str]) -> GitOutput {
        match Command::new("git").arg("-C").arg(dir).args(args).output() {
            Ok(o) => GitOutput {
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
                success: o.status.success(),
            },
            Err(e) => GitOutput {
                stdout: String::new(),
                stderr: format!("failed to run git: {e}"),
                success: false,
            },
        }
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;

    /// Deterministic fake `Git`, keyed by the space-joined args.
    #[derive(Default)]
    pub struct FakeGit {
        responses: HashMap<String, GitOutput>,
    }

    impl FakeGit {
        pub fn new() -> Self {
            Self::default()
        }

        /// Register a successful (exit 0) response for the given args (space-joined).
        pub fn ok(mut self, args: &str, stdout: &str) -> Self {
            self.responses.insert(
                args.to_string(),
                GitOutput {
                    stdout: stdout.to_string(),
                    stderr: String::new(),
                    success: true,
                },
            );
            self
        }

        /// Register a failing (exit != 0) response for the given args.
        pub fn fail(mut self, args: &str) -> Self {
            self.responses.insert(
                args.to_string(),
                GitOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    success: false,
                },
            );
            self
        }

        /// Register a successful response scoped to a specific directory (display
        /// string), so recursion over multiple repos can be tested.
        pub fn ok_in(mut self, dir: &str, args: &str, stdout: &str) -> Self {
            self.responses.insert(
                format!("{dir}\u{0}{args}"),
                GitOutput {
                    stdout: stdout.to_string(),
                    stderr: String::new(),
                    success: true,
                },
            );
            self
        }
    }

    impl Git for FakeGit {
        fn run(&self, dir: &Path, args: &[&str]) -> GitOutput {
            let joined = args.join(" ");
            let dir_key = format!("{}\u{0}{}", dir.display(), joined);
            // Prefer a directory-scoped response, else fall back to an args-only one.
            self.responses
                .get(&dir_key)
                .or_else(|| self.responses.get(&joined))
                .cloned()
                .unwrap_or(GitOutput {
                    stdout: String::new(),
                    stderr: format!(
                        "FakeGit: no response for `git {joined}` in {}",
                        dir.display()
                    ),
                    success: false,
                })
        }
    }
}
