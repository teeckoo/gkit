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
   like `clone`): `switch base` → `pull --rebase origin base` → delete feature →
   `remote prune origin`. (`git switch`, not `checkout`, so a worktree path named like
   the base — e.g. a `main/` dir — can't make the branch switch ambiguous.)
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
$ gkit stmb --yes ~/work/repo          # no --base → base resolved from gkit.baseBranch
stmb plan (1 repo(s)):
  .  -> switch to 'main', pull, delete 'feat-x'
.:
  + git switch main
  + git pull --rebase origin main
  + git branch -d feat-x
  + git remote prune origin

--- logoff ---
/home/you/work/repo  main  true
```

The base comes from `git config gkit.baseBranch` (then `origin/HEAD`) — pass `--base <b>`
only to override it for the root, e.g. `gkit stmb --base dev`.

`--dry-run` prints just the plan (the first block) and stops.
