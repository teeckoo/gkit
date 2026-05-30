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

In fleet mode each repo resolves its own base branch (`gkit.baseBranch` → HEAD).
Exit is non-zero if any repo fails a check or any conf fails to parse.

## The five checks

For each repo and submodule, all must pass:

1. **committed** — `git status -s` is empty.
2. **all-commits-pushed** — no local commit is missing from a remote.
3. **branches-have-remote** — every local branch has a remote counterpart.
4. **not-behind-remote** — current branch isn't behind `origin/<branch>`.
5. **correct-branch** — you're not sitting on the base/integration branch while
   feature branches exist on the remote. The base is resolved from
   `git config gkit.baseBranch` → `--base-branch` → current HEAD; `main`/`master`
   are always treated as integration branches too.

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
/path/repo	RESULT	dev	false
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
