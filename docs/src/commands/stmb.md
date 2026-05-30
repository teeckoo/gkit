# gkit stmb

*Switch to main branch.* You've finished a feature branch — `stmb` returns to the
base branch, updates it, and **safely deletes** the feature, recursively across
submodules, then runs a verifying log-off check.

## Synopsis

```sh
gkit stmb [path] [--base <b>] [--no-recursive] [--force] [-y|--yes] [--dry-run]
```

## What it does

1. Resolve **one** base branch for the whole tree: `--base` → `git config
   gkit.baseBranch` → `origin/HEAD`. (Used for every repo, so a submodule's base is
   never mis-resolved.)
2. Walk the repo + submodules (post-order) and build a plan per repo:
   - on a feature branch → switch to base, pull, **delete the feature**;
   - already on base → switch/pull only;
   - dirty working tree or detached HEAD → **skip** (reported).
3. Print the plan. With `--dry-run`, stop here. Otherwise confirm (skip with `-y`).
4. Execute, **printing each git command** under a per-repo header (transparency,
   like `clone`): `checkout base` → `pull --rebase origin base` → delete feature →
   `remote prune origin`.
5. Automatically run `logoff` (recursive) to confirm everything is clean — after a
   blank line.

## Safe deletion

The feature branch is deleted with `git branch -d`, which **refuses to delete an
unmerged branch** — so you can't silently lose unpushed work. Pass `--force` to use
`-D` (and accept the loss). This is the key improvement over a blind `git branch -D`.

## Flags

| Flag | Effect |
|---|---|
| `--base <b>` | Base branch to switch to (root only). |
| `--no-recursive` | Only the top repo; don't recurse into submodules. |
| `--force` | Force-delete an unmerged feature branch. |
| `-y, --yes` | Skip the confirmation prompt. |
| `--dry-run` | Print the plan without changing anything. |

## Example

```text
$ gkit stmb --base dev --yes ~/work/repo
stmb plan (1 repo(s)):
  .  -> switch to 'dev', pull, delete 'feat-x'
.:
  + git checkout dev
  + git pull --rebase origin dev
  + git branch -d feat-x
  + git remote prune origin

--- logoff ---
/home/you/work/repo  dev  true
```

`--dry-run` prints just the plan (the first block) and stops.
