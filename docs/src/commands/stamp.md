# gkit stamp

*Re-apply a conf's `post-clone` over existing repos.* `gkit clone` runs a conf's
`post-clone` hooks once, right after cloning. `gkit stamp` re-runs those same hooks
on repos that are **already on disk** — no cloning, no fetching.

## Synopsis

```sh
gkit stamp [paths…] [--dry-run] [-y|--yes]     # repo-mode (default)
gkit stamp --conf <conf…> [--dry-run] [-y]     # conf-mode
```

Two modes, with the same `--conf` shape as [`logoff`](./logoff.md):

- **repo-mode (default):** the positional `paths` are **repo paths** (default: the
  current directory). Each repo reads its own `gkit.conf` git-config key — the
  absolute conf path that `clone` (or a prior conf-mode run) stamped — to find the
  conf that drives it, and re-applies that repo's `post-clone`. **Fails clearly if
  `gkit.conf` is unset** rather than guessing.
- **conf-mode (`--conf`):** the positional args are **conf file(s)** (explicit
  files, same rule as `clone`). Re-applies each conf to every repo it lists, **and
  back-fills `gkit.conf`** (set-if-unset) so those repos become usable in repo-mode.

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
Re-running `stamp` re-applies the conf's `post-clone` so those values converge. It's
safe to re-run: the hooks are `git config` writes, which are idempotent. (A submodule
must be **initialized** for the `submodule foreach` to reach it — run
`git submodule update --init` first, then [`gkit fixsub`](./fixsub.md) to un-detach
it.)

## What it does

**repo-mode** (per repo path / cwd): resolve the repo's own `gkit.conf`; parse that
conf; find the `[[repo]]` whose `dir` matches this repo (else fall back to the conf's
global `post-clone`); print the plan; confirm (skip with `-y`); run the hooks in the
repo dir. A repo with **no `gkit.conf`** fails with an actionable hint; a non-git dir
fails.

**conf-mode** (`--conf`): read + validate every conf; print the plan; confirm; per
repo (in conf order) **back-fill `gkit.conf`** where missing, then run the effective
`post-clone`. A missing/non-git dir fails; a repo with no hooks is skipped.

`stamp` does **not** clone, fetch, run `pre-clone`, or run clone's built-ins. Git
**identity** and the submodule **branch-switch** are not `stamp`'s job — identity is a
`clone` concern, and the branch-switch lives in [`gkit fixsub`](./fixsub.md). So
`$GKIT_USER_NAME`/`$GKIT_USER_EMAIL` are empty under `stamp`.

## Flags

| Flag | Effect |
|---|---|
| `--conf` | Treat the args as clone conf file(s) (conf-mode) and back-fill `gkit.conf`. Without it, stamp is repo-mode. |
| `--dry-run` | Print the plan without changing anything. |
| `-y, --yes` | Skip the confirmation prompt. |

## Example

```text
# conf-mode: re-apply + stamp gkit.conf on each repo
$ gkit stamp --conf repos.toml -y
stamp plan (conf mode):
  superproject  (/home/you/work/superproject):
    + git config gkit.baseBranch dev
+ git config gkit.conf /home/you/confs/repos.toml  (/home/you/work/superproject)
+ git config gkit.baseBranch dev
stamped  superproject                 /home/you/work/superproject

# later, from inside the repo — no conf needed (reads gkit.conf):
$ cd /home/you/work/superproject && gkit stamp -y
stamp plan (repo mode):
  /home/you/work/superproject  (conf: /home/you/confs/repos.toml)
    + git config gkit.baseBranch dev
+ git config gkit.baseBranch dev
stamped  superproject                 /home/you/work/superproject
```
