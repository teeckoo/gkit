//! gkit-core — shared library for the gkit toolkit.
//!
//! Side effects go through `git` (a `Git` trait, shelling out to the real binary),
//! so logic stays unit-testable. It houses the `clone`, `logoff`, `stmb`, and
//! `key` logic behind the `gkit` CLI.

pub mod checks;
pub mod clone;
pub mod conf;
pub mod config;
pub mod fixsub;
pub mod git;
pub mod key;
pub mod report;
pub mod stamp;
pub mod stmb;
pub mod submodules;
