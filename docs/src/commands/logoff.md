# gkit logoff

The log-off check: **is every repo and submodule committed and pushed?** A real
pass/fail gate — exit `0` when all clear, non-zero when something is pending.
Recursive into submodules; output is deterministic and greppable.

## Synopsis

```sh
gkit logoff [path…] [-v|-vv] [--no-fetch] [--base-branch <b>]
gkit logoff --conf <conf…>      # check every repo listed in the conf(s)
gkit logoff -e                  # explain: print the static rule catalog
gkit logoff -e <N> [path]       # explain rule R<N> in depth, with this repo's live state
```

- `gkit logoff` — the repo at the current directory.
- `gkit logoff <path>…` — those repo(s) + their submodules.
- `gkit logoff --conf <conf…>` — **fleet mode**: check every repo listed in the
  given clone conf(s). Takes **explicit conf file(s)** (required — a bare `--conf`
  errors, and a directory is not accepted); files may be from **different
  directories**. Use a shell glob for "all in here" (same rule as `gkit clone`):

```sh
gkit logoff --conf *.toml                 # every repo in the cwd's confs
gkit logoff --conf ~/a/x.toml ~/b/y.toml  # confs from different directories
```

In fleet mode each repo resolves its own base branch (`gkit.baseBranch`, then
remote `origin/main`/`origin/master`). Exit is non-zero if any repo fails a check
or any conf fails to parse.

## The six checks

For each repo and submodule, all must pass. Each rule has a stable id (`R1`..`R6`),
shown as a line prefix at `-vv` and looked up with [`-e`](#explaining-the-rules):

1. **R1 committed** — `git status -s` is empty.
2. **R2 all-commits-pushed** — no local commit is missing from a remote.
3. **R3 branches-have-remote** — every local branch has a remote counterpart.
4. **R4 not-behind-remote** — the current branch tracks a remote and isn't behind
   `origin/<branch>`. **Fail-closed**: if behind-ness can't be determined — a
   detached/unborn HEAD, or **no remote-tracking branch** to compare against — the
   check **fails** rather than passing vacuously. (It stays independent of R3: a
   branch with no upstream fails both.)
5. **R5 correct-branch** — are you parked on a *safe* branch? Shared preamble for both
   rules: **detached HEAD** → fails (a risky resting state); on a **feature** branch
   → passes (you're actively on your work). On an **integration** branch
   (`base`/`main`/`master`), one of two **mutually exclusive** rules runs, selected
   by `gkit.solo`:

   - **team** (default, `gkit.solo` off) — fails only if a **local** branch has
     commits **not merged into base** (your own unfinished work). Branches that exist
     only on the remote (others' work, stale branches) are **ignored**, so cleanly
     sitting on `main`/`dev` in a shared repo passes.
   - **solo** (`gkit.solo` on) — fails if the **remote** has **any** feature branch.
     For a solo developer every remote branch is yours, so a leftover one means
     unfinished/uncleaned work. Set `git config gkit.solo true` (per repo) or
     `git config --global gkit.solo true` (your default).

   The base is resolved from `--base-branch` → `git config gkit.baseBranch` → a
   remote-tracking branch (`origin/main`, else `origin/master`); `main`/`master` are
   always integration too. If **none** of those yield a base (e.g. a single-branch
   clone of a feature branch), the base is **unresolved** and this check **fails**
   rather than passing vacuously. Set `git config gkit.baseBranch <b>` to fix.

   The active rule is surfaced on a `branch-rule` line at `-vv` (always, team or
   solo); the bare `-v` scan and the default output print nothing about it.

6. **R6 not-behind-base** — the base-side twin of R4: on a **feature** branch, is it
   **behind base**? Fails when the branch has fallen behind the integration base
   (`git rev-list --left-right` against `gkit.baseBranch` / `origin/main`/`master`),
   in either form:

   - **diverged** — also *ahead* of base (you have unique commits): history has split,
     **rebase onto base**.
   - **merged / stale** — *not* ahead (no unique commits): the branch is done,
     **switch to base & delete** (that's [`gkit stmb`](./stmb.md)).

   A feature branch that's ahead of base but **not behind** (on top of base, ready to
   PR) passes. **Integration branches are skipped** (comparing base to itself is
   vacuous). **Fail-closed**, and **independent of R5**: a detached HEAD, an
   unresolved base, or a base whose ref can't be located all **fail** R6 with their
   own reason (they don't defer to R5). Accuracy depends on a fresh `origin/<base>`:
   under `--no-fetch` R6 compares against your last-fetched base.

   **Tolerate it with `gkit.allowDiverged`.** `git config gkit.allowDiverged true`
   (per repo, or `--global`) downgrades an R6 behind-base **failure** to a **pass** —
   but the default output still carries a marker (e.g. `… true (diverged, allowed by
   gkit.allowDiverged)`), so it stays visible and greppable (`gkit logoff | grep
   allowDiverged` audits the tolerated repos). It does **not** suppress R6's
   fail-closed cases (unresolved/absent base, detached) — those are config errors to
   fix, not divergence to tolerate.

## Rule philosophy

The checks follow a few deliberate principles — worth knowing, because they explain
"why did *that* fail?":

- **Rules are independent.** Each rule asks for exactly the inputs it needs and
  renders its own verdict; none defers to another. So an unresolved base makes **both**
  R5 (correct-branch) and R6 (not-behind-base) fail — each with its own reason. That's
  by design, not a double-report bug: every rule reports honestly on its own terms.
- **Fail-closed — never pass vacuously.** If a rule can't determine its answer (no
  base resolved, a base ref that can't be located, a detached HEAD, a missing upstream),
  it **fails** with an actionable reason rather than passing silently. A green check
  means "verified safe," never "couldn't tell." This is why R4 was tightened: a branch
  with no remote-tracking ref now fails R4 instead of vacuously passing.
- **Durability vs. integration are different questions.** R2 (all-commits-pushed)
  answers *"is my work safe?"* — once everything is on a remote, nothing is lost. R6
  answers a separate question, *"is my feature branch current with base?"* A branch can
  be perfectly pushed (R2 green) yet badly behind `dev` (R6 red). So **ahead + pushed is
  fine** (you're on top of base, ready to PR); **behind base is the defect** — whether
  diverged (rebase) or merged/stale (delete).
- **R4 and R6 are twins on different refs.** R4 compares HEAD to its **own** upstream
  (`origin/<branch>` — "did I pull my branch?"); R6 compares HEAD to the **integration
  base** (`dev`/`main` — "did my branch keep up with the trunk?"). Neither subsumes the
  other.
- **`-vv` is the "why did it fail" view.** Per-failure `R<n> reason` lines live only at
  `-vv`; the bare `-v` scan and the default output stay pure pass/fail. The single
  exception is the `gkit.allowDiverged` marker, which rides the default line so a repo
  *tolerating* divergence is still visible to someone who isn't drilling in with `-vv`.
- **The default line is an API.** It's `path  branch  status [marker]` with fixed
  field positions, so [fleet greps](#filtering-repos-with-grep) stay stable across
  releases.

## Output

Default (one line per repo, post-order: submodules before their parent):

```text
/path/repo/submodule-a   dev true
/path/repo               dev false
```

A repo tolerating divergence (`gkit.allowDiverged`) passes but the line carries a
**trailing** marker after the boolean (never before it, so `path branch status`
field positions stay stable):

```text
/path/repo   SCB-283 true (diverged, allowed by gkit.allowDiverged)
```

`-v` — a pure pass/fail **scan**: one fact per line, path-first, tab-separated,
fixed order. Just the six checks + `RESULT` (no contextual metadata):

```text
/path/repo	committed	true
/path/repo	all-commits-pushed	false
/path/repo	branches-have-remote	true
/path/repo	not-behind-remote	true
/path/repo	correct-branch	true
/path/repo	not-behind-base	true
/path/repo	RESULT	dev	false
```

### Filtering repos with grep

The default line is a stable, greppable contract — `path  branch  status [marker]`:
the **branch name is always field 2**, the **status (`true`/`false`) always field 3**,
and the only optional addition is the `gkit.allowDiverged` marker, appended as a
**trailing** field on passing lines (never shifting the first three). No marker
contains the substrings `true`/`false`, so `grep true`/`grep false` stay clean. That
makes the everyday fleet slices just work:

```sh
gkit logoff ~/work/*           | grep false              # repos that need attention
gkit logoff ~/work/*           | grep true | grep SCB-   # clean feature branches parked but unmerged
gkit logoff ~/work/*           | grep allowDiverged      # repos tolerating divergence (audit)
gkit logoff -v ~/work/* | awk -F'\t' '$NF=="false"'      # -v: failing checks, by column
gkit logoff -vv ~/work/* | grep 'not-behind-base.*false' # -vv: who's behind base
```

Because R6 flips a stale/diverged feature branch from `true` to `false`, `grep true |
grep SCB-` now returns only branches that are *also* current with base — the sharper
answer than before R6 existed (a branch silently rotting behind `dev` no longer hides
in the `true` set).

`-vv` is `-v` plus **context + why**: each check line gains its `R<n>` rule id; the
`base-branch` and `branch-rule` metadata lines appear (only here, not at `-v`); and
every **failing** check is followed by an `R<n> reason` line (R5 names the offending
branch, R6 the base + ahead/behind counts). Passing checks get no reason line:

```text
/path/repo	R1 committed	true
/path/repo	R4 not-behind-remote	true
/path/repo	base-branch	dev (from git config gkit.baseBranch)
/path/repo	branch-rule	team (gkit.solo off) — flags a local branch unmerged into base
/path/repo	R5 correct-branch	true
/path/repo	R6 not-behind-base	false
/path/repo	R6 reason	diverged from base 'dev': 1 ahead, 2 behind — rebase onto base
/path/repo	RESULT	SCB-283	false
```

The `gkit.allowDiverged` marker is the **one** thing that rides the default/`-v`
output (on the RESULT line and the default line); the per-failure `R<n> reason`
lines remain `-vv`-only — `-vv` is the "why did it fail" view.

The `base-branch` line shows the resolved base **and how it was derived** —
`(from --base-branch)`, `(from git config gkit.baseBranch)`, or
`(derived from remote origin/main)`; when it can't be resolved it reads
`UNRESOLVED — …` and `correct-branch` is `false`.

## Explaining the rules

Two forms, both exit `0` (informational, never the gate):

**Bare `-e`** — the static rule catalog (one line per rule: id, key, description).
Read-only; ignores paths and never touches git:

```sh
gkit logoff -e      # R1..R6, one per line
```

**`-e <N>`** — a **repo-aware deep dive** on one rule: what it checks, *this repo's*
live state (actual branch values, the resolved base, the active rule, the failing
verdict), and a few teaching examples. Reads a **single** repo — the cwd, or the
path you give — with no submodule recursion and no fetch. The natural follow-up
when `-vv` flags a rule and you want the full picture:

```text
$ gkit logoff -e 5

R5  correct-branch    [this repo: FAIL]

  What it checks
    parked on a safe branch: a feature branch always passes; on an
    integration branch the team rule (default) flags a local branch unmerged
    into base, …

  This repo now
    branch          main
    base            main (derived from remote origin/main)
    rule            team (gkit.solo off) — flags a local branch unmerged into base
    local branches  feat-x, main
    verdict         FAIL — local branch 'feat-x' is not merged into base …

  Examples
    on a feature branch                      PASS (actively on your work)
    on base/main, all local branches merged  PASS (parked clean)
    on base/main, local 'wip' unmerged       FAIL (team: unfinished work)
    detached HEAD                            FAIL (risky resting state)
```

An out-of-range rule number (not `1`..`6`) errors with a non-zero exit.

A path that **isn't a git repository** (or doesn't exist) **fails the gate** rather
than passing — the reason is shown where the branch would be, so the line still
ends in `false` and the exit code is non-zero:

```text
/path/not-a-repo   not a git repository   false
/path/missing      no such directory      false
```

(Without this, a non-repo would pass every check vacuously: an empty `git status`
reads as "nothing pending".)

## Flags

| Flag | Effect |
|---|---|
| `-v` | Per-check breakdown (greppable). Repeat (`-vv`) to add `R<n>` rule ids and a `reason` line for each failing check. |
| `-e` | Explain (exits 0). Bare = static rule catalog (no repo). `-e <N>` = repo-aware deep dive on rule R`N`: what it checks + this repo's live state + examples (single repo: cwd or the given path). |
| `--no-fetch` | Don't fetch submodules first (faster / offline). |
| `--base-branch <b>` | Override the base branch (root repo only). |

> Parallelized for speed, but results are buffered and emitted in a fixed order, so
> output never depends on which thread finishes first.
