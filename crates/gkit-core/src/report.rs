//! Output formatting. Entries arrive already in the fixed post-order DFS order
//! (see `submodules`), so these just render — keeping the per-check order fixed
//! and the lines greppable (path-first, stable keys, trailing boolean).

use crate::submodules::Entry;

/// Default: one line per repo — `<abs-path> <branch> true|false` (zsh-compatible).
pub fn print_default(entries: &[Entry]) {
    for e in entries {
        println!("{} {} {}", e.path.display(), e.status.branch, e.status.ok());
    }
}

/// Verbose: one fact per line, path-first, tab-separated, stable check keys, in a
/// fixed order. Greppable: `grep -w false`, `grep <repo>`, `awk -F'\t' '$NF=="false"'`.
pub fn print_verbose(entries: &[Entry]) {
    for e in entries {
        let p = e.path.display();
        let s = &e.status;
        println!("{p}\tcommitted\t{}", s.committed);
        println!("{p}\tall-commits-pushed\t{}", s.all_commits_pushed);
        println!("{p}\tbranches-have-remote\t{}", s.branches_have_remote);
        println!("{p}\tnot-behind-remote\t{}", s.not_behind_remote);
        println!("{p}\tcorrect-branch\t{}", s.correct_branch);
        println!("{p}\tRESULT\t{}\t{}", s.branch, s.ok());
    }
}

/// The gate: true only if every entry passed.
pub fn all_ok(entries: &[Entry]) -> bool {
    entries.iter().all(|e| e.status.ok())
}
