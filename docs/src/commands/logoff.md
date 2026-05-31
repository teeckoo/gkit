# gkit logoff

The log-off check: **is every repo and submodule committed and pushed?** A real
pass/fail gate — exit `0` when all clear, non-zero when something is pending.
Recursive into submodules; output is deterministic and greppable.

## Synopsis

```sh
gkit logoff [path…] [--verbose] [--no-fetch] [--base-branch <b>]
gkit logoff --conf <conf…>      # check every repo listed in the conf(s)
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

## The five checks

For each repo and submodule, all must pass:

1. **committed** — `git status -s` is empty.
2. **all-commits-pushed** — no local commit is missing from a remote.
3. **branches-have-remote** — every local branch has a remote counterpart.
4. **not-behind-remote** — current branch isn't behind `origin/<branch>`.
5. **correct-branch** — are you parked on a *safe* branch? Shared preamble for both
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
     `git config --global gkit.solo true` (your default); `gkit clone` stamps it from
     the conf's `solo` field.

   The base is resolved from `--base-branch` → `git config gkit.baseBranch` → a
   remote-tracking branch (`origin/main`, else `origin/master`); `main`/`master` are
   always integration too. If **none** of those yield a base (e.g. a single-branch
   clone of a feature branch), the base is **unresolved** and this check **fails**
   rather than passing vacuously. Set `git config gkit.baseBranch <b>` to fix.

   In `--verbose`, when the non-default **solo** rule is active a `branch-rule` line
   states which rule ran and why; the default team rule prints nothing (no noise).

## Output

Default (one line per repo, post-order: submodules before their parent):

```text
/path/repo/submodule-a   dev true
/path/repo               dev false
```

`--verbose` — one fact per line, path-first, tab-separated, fixed order:

```text
/path/repo	committed	true
/path/repo	all-commits-pushed	false
/path/repo	base-branch	dev (from git config gkit.baseBranch)
/path/repo	correct-branch	true
/path/repo	RESULT	dev	false
```

The `base-branch` line shows the resolved base **and how it was derived** —
`(from --base-branch)`, `(from git config gkit.baseBranch)`, or
`(derived from remote origin/main)`. When it can't be resolved it reads
`UNRESOLVED — …` and `correct-branch` is `false`. When the **solo** rule is active a
`branch-rule` line precedes `correct-branch`; the default team rule prints none:

```text
/path/repo	base-branch	dev (from git config gkit.baseBranch)
/path/repo	branch-rule	solo (gkit.solo on) — flags any feature branch on the remote
/path/repo	correct-branch	false
```

Greppable: `gkit logoff -v | grep -w false`, `… | awk -F'\t' '$NF=="false"'`.

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
| `-v, --verbose` | Per-check breakdown (greppable). |
| `--no-fetch` | Don't fetch submodules first (faster / offline). |
| `--base-branch <b>` | Override the base branch (root repo only). |

> Parallelized for speed, but results are buffered and emitted in a fixed order, so
> output never depends on which thread finishes first.
