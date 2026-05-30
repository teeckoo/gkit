# Why gkit (and what we tried first)

gkit exists to fill a specific gap. Before writing it, we evaluated the established
multi-repo tools and even adopted one for a while. Here's what we tried and why we
ended up building our own — including what gkit deliberately does **not** do.

## The need

Working across many repos in several orgs, with multiple ssh identities and lots of
submodules, a few chores repeat constantly:

- clone a **fleet** of repos to set local paths;
- before stepping away, confirm **everything is committed *and* pushed** — including
  every submodule;
- finish a feature branch (switch back to base, update it, delete the branch);
- provision ssh keys per identity.

## What the existing tools do — and don't

- **gita / tsrc / ghq** — solid multi-repo managers (clone, sync, run commands,
  groups). But they are **status reporters / runners**: none ship a
  **submodule-recursive "committed *and* pushed" pass/fail gate**. The closest
  reporter, `mgitstatus`, explicitly `--ignore-submodules`. The "is my work safe to
  walk away from?" check — recursive, with an exit code — isn't in any of them.
- **git `includeIf` + ssh `Host` aliases** — handle per-repo identity well; gkit
  builds on them rather than replacing them.

## The mani trial

[mani](https://manicli.com/) is a genuinely good repo manager + task runner, so we
adopted it first:

- **clone** via `mani sync` (a manifest of repos → local paths);
- **log-off / setup** by wrapping our existing zsh check as a `mani run` task.

Two things kept it from being the whole solution:

1. **No clone hooks.** Our clone needs per-repo work *during* cloning — toggle
   `direnv` (some `.envrc`s launch a pager like `glow` and corrupt command output),
   switch submodules onto their branch, stamp config. mani has **no pre/post-clone
   hook** and no global default clone command. The only workaround was a bespoke
   `safeclone` shell wrapper around `mani sync` — which isn't shippable to other
   users, defeating the purpose.
2. **It's a *general* task runner.** mani's other half (`run`/`exec`/`tui`) is
   powerful, but it's a general multi-repo runner; our real needs are a handful of
   **specific** operations. Depending on a general runner — and re-implementing the
   parts it lacks — wasn't the right fit.

And crucially, the **log-off gate itself is domain logic no tool provides** (five
checks + submodule recursion + a real exit code). Running it through mani still left
that logic as ours.

## The decision

Build **gkit**: a small, transparent, stateless binary that **shells out to `git`**
and does exactly the specific jobs — a **config-driven clone with the pre/post hooks
mani lacks**, the **submodule-recursive log-off gate**, `stmb`, and `key`.

We deliberately **do not** rebuild mani's general task runner / parallel exec / TUI —
that *would* be reinventing the wheel. gkit is a set of specific tools, **not** a
fleet runner; for "run any command across many repos," reach for mani or similar.

### In short

- **Reuse where tools fit** — git itself, ssh `Host` aliases, and a general runner if
  you want one.
- **Build only the genuine gaps** — clone-with-hooks and the submodule-recursive
  log-off gate (plus `stmb`/`key` to round out the workflow).
- **Stay transparent** (print every command) and **stateless** (the conf file and
  each repo's own metadata are the only state).
