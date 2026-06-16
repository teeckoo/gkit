# gkit stamp

*Re-apply a conf's `post-clone` over existing repos.* `gkit clone` runs a conf's
`post-clone` hooks once, right after cloning. `gkit stamp` re-runs those same hooks
on repos that are **already on disk** — no cloning, no fetching.

## Synopsis

```sh
gkit stamp <conf…> [--dry-run] [-y|--yes]
```

`conf…` are **explicit conf file(s)** — the same rule as
[`clone`](./clone.md): at least one is required and a directory is rejected (use a
shell glob like `confs/*.toml`). `gkit stamp` with no conf is an error.

## Why it exists

`post-clone` is where a team stamps the per-repo git config the gate reads —
`git config gkit.baseBranch …`, `git config gkit.solo …`, usually with a
`git submodule foreach --recursive '…'` so submodules get it too:

```toml
post-clone = [
  "git config gkit.baseBranch dev",
  "git submodule foreach --recursive 'git config gkit.baseBranch dev'",
  "git config gkit.solo true",
  "git submodule foreach --recursive 'git config gkit.solo true'",
]
```

But `clone` runs that **once**, over the submodules that existed *then*. A submodule
added **later** — e.g. on a feature branch that pins a new submodule — is never
stamped: it comes up with no `gkit.baseBranch` (so [`logoff`](./logoff.md)'s base
falls back to `origin/main`/`master`) and no `gkit.solo` (so it uses the team rule).
`gkit stamp <conf>` re-runs the conf's `post-clone` over the existing repos so those
values converge. It's safe to re-run: the hooks are `git config` writes, which are
idempotent.

## What it does

1. Read + validate every conf (a conf that fails is reported and skipped; the rest
   still run, and the exit code is non-zero if anything failed).
2. **Print the plan**: each repo's dir and the `post-clone` commands that would run.
   With `--dry-run`, stop here.
3. Confirm (skip with `-y`).
4. Per repo, in conf order: a **missing dir or non-git dir fails** (never a silent
   skip — you want to know); a repo with **no `post-clone` is skipped**; otherwise
   the hooks run in the repo dir, each printed `+ <cmd>`, with the same `$GKIT_*`
   env [`clone`](./clone.md) sets.

`stamp` does **not** clone, fetch, run `pre-clone`, or run clone's built-ins
(submodule branch-switch, `direnv allow`). Git **identity** is a `clone` concern, so
`$GKIT_USER_NAME`/`$GKIT_USER_EMAIL` are empty under `stamp`.

## Conf-only (no in-repo mode)

Unlike `logoff`/`stmb`, `stamp` always takes a conf and has no path mode. The conf
is the source of truth for each repo's *intended* values — and those differ per
submodule (one repo's base is `dev`, another's is `main`). Without it there's no
correct value to write to a freshly-added submodule, so `stamp` requires the conf
rather than guessing.

## Flags

| Flag | Effect |
|---|---|
| `--dry-run` | Print the plan (repos + hooks) without changing anything. |
| `-y, --yes` | Skip the confirmation prompt. |

## Example

```toml
# repos.toml
host      = "tlbb"
namespace = "example-org"
post-clone = [
  "git config gkit.baseBranch dev",
  "git submodule foreach --recursive 'git config gkit.baseBranch dev'",
]

[[repo]]
dir = "$HOME/work/superproject"
```

```text
$ gkit stamp repos.toml -y
stamp plan:
  superproject  (/home/you/work/superproject):
    + git config gkit.baseBranch dev
    + git submodule foreach --recursive 'git config gkit.baseBranch dev'
+ git config gkit.baseBranch dev
+ git submodule foreach --recursive 'git config gkit.baseBranch dev'
Entering 'new-child'
stamped  superproject                 /home/you/work/superproject
```

After `stamp`, the newly-pinned `new-child` submodule carries `gkit.baseBranch=dev`,
so `gkit logoff` resolves its base correctly.
