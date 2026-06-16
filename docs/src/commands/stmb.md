# gkit stmb

*Switch to main branch.* You've finished a feature branch — `stmb` returns to the
base branch, updates it, and **deletes the feature only after verifying it's merged**,
recursively across submodules, then runs a verifying log-off check.

## Synopsis

```sh
gkit stmb [path] [--base <b>] [--no-recursive] [-y|--yes] [--dry-run]
```

## What it does

1. Resolve each repo's **own** base: `--base` applies to the **root only**; every
   submodule resolves its own `git config gkit.baseBranch` → `origin/HEAD` (the same
   per-repo resolution `logoff` uses). This matters for a tree where a submodule tracks
   a *different* integration branch (e.g. one on `dev` while the root is on `main`): a
   single tree-wide base would switch that submodule onto the root's base and treat its
   real base as a deletable feature. A repo whose base can't be resolved is **skipped**
   (never forced onto another repo's base).
2. Walk the repo + submodules (post-order) and build a plan per repo, **showing each
   repo's current branch** (`[on <branch>]`) so a wrong base is obvious before you confirm:
   - on a feature branch → switch to base, pull, **delete the feature if merged**;
   - already on base → update (pull) only;
   - dirty working tree or detached HEAD → **skip** (reported).
3. Print the plan. With `--dry-run`, stop here. Otherwise confirm (skip with `-y`).
4. Execute, **printing each git command** under a per-repo header (transparency,
   like `clone`): `switch base` (**skipped if already on base** — just pull) →
   `pull --rebase origin base` → (verify, then) delete feature → `remote prune origin`.
   (`git switch`, not `checkout`, so a worktree path named like the base — e.g. a `main/`
   dir — can't make the branch switch ambiguous.)
5. Automatically run `logoff` (recursive) to confirm everything is clean — after a
   blank line.

## Verified deletion (and why there's no `--force`)

Before deleting a feature branch, stmb **proves it is merged into base** and prints a
readable reason + a one-line conclusion (`=> deleting …` / `=> NOT deleting …`). The
check has two stages — base is pulled *first*, so both see fresh history:

1. **Reachability** — is the feature tip in base's history? Catches **merge-commit**
   and **fast-forward** merges. Deletes with `git branch -d` (git agrees).
2. **Patch-id equivalence** — if not reachable, are all of the branch's commits already
   in base *by content*? Catches **squash** and **rebase** merges (which rewrite the
   commit hash, so reachability alone wrongly reports "not merged"). Deletes with
   `git branch -D` — stmb has vouched for it via `git rev-list --cherry-pick`.

If **neither** holds — the branch has commits not in base, or stmb hits a git error —
it is **fail-closed**: stmb **refuses to delete**, explains why, and tells you to
discard it yourself:

```text
  'feat-x' has 1 commit(s) not present in main (by content)
  => NOT deleting 'feat-x'. If you're sure, discard it with: git branch -D feat-x
```

There is deliberately **no `--force` flag**. A force flag that's needed on every
squash/rebase merge trains people to pass it reflexively — which is exactly how real
work gets deleted. By verifying content instead, the common squash/rebase case deletes
automatically, and the only branches stmb won't touch are ones it *couldn't prove are
merged*. For those rare cases (e.g. a squash that resolved conflicts, so patch-ids no
longer match) run plain `git branch -D <branch>` yourself — discarding unverified work
is a raw-git operation, not fleet cleanup.

## Flags

| Flag | Effect |
|---|---|
| `--base <b>` | Base branch to switch to (root only). |
| `--no-recursive` | Only the top repo; don't recurse into submodules. |
| `-y, --yes` | Skip the confirmation prompt. |
| `--dry-run` | Print the plan without changing anything. |

## Example

```text
$ gkit stmb --yes ~/work/repo          # no --base → each repo resolves its own base
stmb plan (2 repo(s)):
  a-submodule-on-dev  [on dev]  -> update 'dev' (pull)
  .  [on feat-x]  -> switch to 'main', pull, delete 'feat-x' if merged
.:
  + git switch main
  + git pull --rebase origin main
  'feat-x' has no commits missing from main — its changes are already in main (squash/rebase-merged)
  => deleting 'feat-x' (verified merged by content).
  + git branch -D feat-x
  + git remote prune origin

--- logoff ---
/home/you/work/repo  main  true
```

Each repo resolves its own base from `git config gkit.baseBranch` (then `origin/HEAD`); the
`[on <branch>]` prefix shows where each repo currently sits. `--base <b>` overrides the base
for the **root only** (`gkit stmb --base dev`); submodules keep their own — so a submodule
that tracks `dev` is updated on `dev`, not switched to the root's `main`.

`--dry-run` prints just the plan (the first block) and stops.
