//! Output formatting. Entries arrive already in the fixed post-order DFS order
//! (see `submodules`), so these just render — keeping the per-check order fixed
//! and the lines greppable (path-first, stable keys, trailing boolean).

use crate::checks::{RepoStatus, RuleId, RuleReport};
use crate::submodules::Entry;

/// Default: one line per repo — `<abs-path> <branch> true|false` (zsh-compatible).
/// For an unusable path the reason sits where the branch would be, so the line
/// still ends in `false` and stays greppable. A repo tolerating divergence
/// (`gkit.allowDiverged`) passes but carries a **trailing** marker after the
/// boolean — never before it, so `path/branch/status` field positions are stable.
pub fn print_default(entries: &[Entry]) {
    for e in entries {
        let middle = e.status.problem.as_deref().unwrap_or(&e.status.branch);
        let (path, ok) = (e.path.display(), e.status.ok());
        match e.status.base_sync.marker() {
            Some(m) => println!("{path} {middle} {ok} {m}"),
            None => println!("{path} {middle} {ok}"),
        }
    }
}

/// Verbose: one fact per line, path-first, tab-separated, stable check keys, in a
/// fixed order. Greppable: `grep -w false`, `grep <repo>`, `awk -F'\t' '$NF=="false"'`.
pub fn print_verbose(entries: &[Entry], reasons: bool) {
    for e in entries {
        let p = e.path.display().to_string();
        let s = &e.status;
        if let Some(reason) = &s.problem {
            println!("{p}\tRESULT\t{reason}\tfalse");
            continue;
        }
        emit_check(&p, s, RuleId::Committed, reasons);
        emit_check(&p, s, RuleId::AllCommitsPushed, reasons);
        emit_check(&p, s, RuleId::BranchesHaveRemote, reasons);
        emit_check(&p, s, RuleId::NotBehindRemote, reasons);
        // Contextual metadata lives at `-vv` only: `-v` is a pure pass/fail scan
        // (the five check lines + RESULT). At `-vv` we always show both — the
        // `branch-rule` line (team + solo) disambiguates the `R5 reason`.
        if reasons {
            println!("{p}\tbase-branch\t{}", s.base.describe());
            println!("{p}\tbranch-rule\t{}", s.rule.describe());
        }
        emit_check(&p, s, RuleId::CorrectBranch, reasons);
        emit_check(&p, s, RuleId::NotBehindBase, reasons);
        // The `gkit.allowDiverged` marker rides the RESULT line at every level (the
        // one default/`-v` addition); reason lines stay `-vv`-only. Trailing field,
        // so RESULT's branch/bool columns don't shift.
        match s.base_sync.marker() {
            Some(m) => println!("{p}\tRESULT\t{}\t{}\t{m}", s.branch, s.ok()),
            None => println!("{p}\tRESULT\t{}\t{}", s.branch, s.ok()),
        }
    }
}

/// One per-check line. At `-vv` (`reasons`) it gains an `R<n>` prefix and, when
/// the check failed, a following `R<n> reason\t<why>` line. At `-v` it stays the
/// bare, greppable `path\t<key>\t<bool>` (unchanged).
fn emit_check(p: &str, s: &RepoStatus, rule: RuleId, reasons: bool) {
    let passed = s.rule_passed(rule);
    if reasons {
        println!("{p}\t{} {}\t{}", rule.tag(), rule.key(), passed);
        if let Some(why) = s.failure_reason(rule) {
            println!("{p}\t{} reason\t{why}", rule.tag());
        }
    } else {
        println!("{p}\t{}\t{}", rule.key(), passed);
    }
}

/// Bare `logoff -e`: print the static rule catalog (one line per rule:
/// tag, key, description). Read-only; needs no repo. `-e <N>` instead renders the
/// repo-aware deep dive via [`print_rule_detail`].
pub fn print_rules() {
    for r in RuleId::ALL {
        println!("{}\t{}\t{}", r.tag(), r.key(), r.description());
    }
}

/// `logoff -e <N>`: render one rule's repo-aware deep dive — what it checks, this
/// repo's live state, and teaching examples. Human-readable (sectioned prose), not
/// the tab-separated machine format: it's for understanding, not scripting.
pub fn print_rule_detail(r: &RuleReport) {
    let status = if r.passed { "PASS" } else { "FAIL" };
    println!("{}  {}    [this repo: {status}]", r.id.tag(), r.id.key());

    println!("\n  What it checks");
    for line in wrap(r.id.description(), 72) {
        println!("    {line}");
    }

    println!("\n  This repo now");
    let label_w = r
        .facts
        .iter()
        .map(|(l, _)| l.len())
        .chain(std::iter::once("verdict".len()))
        .max()
        .unwrap_or(0);
    for (label, value) in &r.facts {
        println!("    {label:<label_w$}  {value}");
    }
    println!("    {:<label_w$}  {}", "verdict", r.verdict);

    println!("\n  Examples");
    let scen_w =
        r.id.examples()
            .iter()
            .map(|(s, _)| s.len())
            .max()
            .unwrap_or(0);
    for (scenario, outcome) in r.id.examples() {
        println!("    {scenario:<scen_w$}  {outcome}");
    }
}

/// Greedy word-wrap to `width` columns (no hyphenation). Used by the `-e <N>`
/// "What it checks" paragraph.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// The gate: true only if every entry passed.
pub fn all_ok(entries: &[Entry]) -> bool {
    entries.iter().all(|e| e.status.ok())
}
