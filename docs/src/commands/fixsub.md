# gkit fixsub

*Fix submodule metadata over an existing tree.* **Un-detaches** each submodule onto its
declared `.gitmodules` branch, **reports** (never yanks) a submodule deliberately on a
different branch, and inherits the root repo's git identity into submodules that lack one —
recursively, idempotently, printing every per-submodule outcome.

## Synopsis

```sh
gkit fixsub [path] [--dry-run] [-y|--yes] [--no-direnv]
```

`path` defaults to the current directory (like [`logoff`](./logoff.md)/[`stmb`](./stmb.md)).

## Why it exists

`git submodule update --init` checks out each submodule at the **pinned commit** in
**detached HEAD** — a gitlink is a SHA, not a branch, so plain `update` ignores
`.gitmodules branch=`. A submodule added after `gkit clone` (e.g. pinned on a feature
branch) therefore comes up detached and, having missed clone's built-ins, with no git
identity. `clone` fixes both at clone time; `fixsub` applies the same fixes to a tree
that's already on disk.

## What it does

Over every **initialized** submodule in the tree (recursively):

1. **Un-detach — and only un-detach.** It looks at each submodule's HEAD and the branch
   its `.gitmodules` declares (falling back to `main`), and prints one outcome per
   submodule:
   - **detached HEAD** → **switch** onto the declared branch (`git switch`, printed);
   - already **on the declared branch** → **kept** (no-op);
   - on a **different named branch** (active feature work) → **left as-is**, with the
     divergence reported — `fixsub` **never silently moves a named branch**.
2. **Identity inherit (set-if-unset):** copy the **root** repo's local `user.name` /
   `user.email` into each submodule that has **no local identity** — never clobbering a
   deliberately-different one.
3. **direnv** (unless `--no-direnv`): `direnv allow` each submodule that has an `.envrc`,
   re-trusting it after the un-detach re-points the working tree (trust-only, no
   evaluation).

Every git command and every per-submodule decision is printed (nothing is swallowed), and
it's safe to re-run (idempotent). Run it after `git submodule update --init` brings a new
submodule down detached.

What it deliberately does **not** do: project-specific config such as `core.hooksPath`
(put those in the conf's `post-clone` and re-apply with [`gkit stamp`](./stamp.md)), and it
does **not** yank a submodule off a feature branch.

## Why there's no `--force` / `--switch-all`

`fixsub` only ever moves a submodule that's in **detached HEAD**. A submodule sitting on a
*named* branch is your active work, so it's reported and left alone. There is deliberately
**no flag to force-switch** named branches: a flag needed routinely trains reflexive use,
and bulk-switching scatters in-progress submodules off their branches at once (the same
footgun removed from [`stmb`](./stmb.md)). To move one submodule onto its branch yourself,
that's a deliberate, per-submodule `git switch <branch>` inside it.

When a submodule is reported as diverged, you have two real choices (not symmetric — see
[`logoff`](./logoff.md) and the submodule-pin notes): **merge** the branch into the
configured one (the normal answer, keeps the team's tracked branch stable), or **update
`.gitmodules`** to track the branch you're on (a deliberate policy change for everyone).

## Caveat — pinned gitlink

The un-detach moves a submodule to its branch **tip**. If that tip differs from the
superproject's pinned gitlink commit, the superproject will then show the submodule as
**changed** (a gitlink bump). Review and commit that intentionally — it means your
submodule pointer was behind its branch.

## Flags

| Flag | Effect |
|---|---|
| `--no-direnv` | Don't `direnv allow` submodules that have an `.envrc`. |
| `--dry-run` | Print the plan (per-submodule would-switch/keep/leave + the identity/direnv command) without changing anything. |
| `-y, --yes` | Skip the confirmation prompt. |

## Example

```text
$ git submodule update --init --recursive      # brings new-child down detached
$ gkit fixsub -y
fixsub plan:
  new-child: detached HEAD → switching to 'dev'
    + git switch dev
  shared-lib: on 'main' (matches .gitmodules) — kept
  my-feature-sub: on 'SCB-554-x'; .gitmodules tracks 'main' — left as-is (merge it into 'main', or update .gitmodules)
+ git submodule foreach --recursive git config --local user.name >/dev/null 2>&1 || git config user.name 'Jane Dev'; …
```

After `fixsub`, detached submodules are back on their branch with the inherited identity —
and [`gkit stamp`](./stamp.md) can re-apply their `gkit.baseBranch`/`gkit.solo`. A submodule
on a feature branch is untouched and flagged for you to reconcile.
