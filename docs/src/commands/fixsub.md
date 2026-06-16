# gkit fixsub

*Fix submodule metadata over an existing tree.* Switches each submodule onto its
declared `.gitmodules` branch (un-detach) and inherits the root repo's git identity
into submodules that lack one — recursively, idempotently.

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

1. **Branch-switch (un-detach):** switch the submodule onto the branch its
   `.gitmodules` declares (falling back to `main`). This is the same built-in
   `gkit clone` runs.
2. **Identity inherit (set-if-unset):** copy the **root** repo's local `user.name` /
   `user.email` into each submodule that has **no local identity** — never clobbering
   a deliberately-different one.
3. **direnv** (unless `--no-direnv`): `direnv allow` each submodule that has an
   `.envrc`, re-trusting it after the branch flip (trust-only, no evaluation).

It prints every git command, and is safe to re-run (idempotent). Run it after
`git submodule update --init` brings a new submodule down detached.

What it deliberately does **not** do: project-specific config such as
`core.hooksPath`. Put those in the conf's `post-clone` and re-apply with
[`gkit stamp`](./stamp.md) — `fixsub` only does universal submodule hygiene.

## Caveat — pinned gitlink

The branch-switch moves a submodule to its branch **tip**. If that tip differs from
the superproject's pinned gitlink commit, the superproject will then show the
submodule as **changed** (a gitlink bump). Review and commit that intentionally — it
means your submodule pointer was behind its branch.

## Flags

| Flag | Effect |
|---|---|
| `--no-direnv` | Don't `direnv allow` submodules that have an `.envrc`. |
| `--dry-run` | Print the plan (the `submodule foreach` command) without changing anything. |
| `-y, --yes` | Skip the confirmation prompt. |

## Example

```text
$ git submodule update --init --recursive      # brings new-child down detached
$ gkit fixsub -y
fixsub plan:
+ git submodule foreach --recursive b=$(git config -f "$toplevel/.gitmodules" "submodule.$name.branch" 2>/dev/null || echo main); git switch "$b" 2>/dev/null || true; git config --local user.name >/dev/null 2>&1 || git config user.name 'Jane Dev'; …
Entering 'new-child'
Switched to branch 'dev'
```

After `fixsub`, `new-child` is on its branch with the inherited identity — and
[`gkit stamp`](./stamp.md) can re-apply its `gkit.baseBranch`/`gkit.solo`.
