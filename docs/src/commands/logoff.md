# gkit logoff

The log-off check: **is every repo and submodule committed and pushed?** A real
pass/fail gate — exit `0` when all clear, non-zero when something is pending.
Recursive into submodules; output is deterministic and greppable.

## Synopsis

```sh
gkit logoff [path] [--verbose] [--no-fetch] [--base-branch <b>]
```

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

## Flags

| Flag | Effect |
|---|---|
| `-v, --verbose` | Per-check breakdown (greppable). |
| `--no-fetch` | Don't fetch submodules first (faster / offline). |
| `--base-branch <b>` | Override the base branch (root repo only). |

> Parallelized for speed, but results are buffered and emitted in a fixed order, so
> output never depends on which thread finishes first.
