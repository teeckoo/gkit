# gkit

**A transparent, stateless git/ssh toolkit.**

`gkit` is one small binary for the repetitive git/ssh chores a developer juggling
many repos and identities runs all the time:

- **`clone`** — clone a fleet of repos from a config file, with built-in
  submodule branch-switching, `.envrc` trust, and pre/post-clone flag passthrough.
- **`logoff`** — a *log-off check*: is every repo **and submodule** committed and
  pushed? It's a real pass/fail gate (exit code), recursive, and greppable.
- **`stmb`** — *switch to main branch*: finish a feature branch by returning to the
  base branch, updating it, and safely deleting the feature — recursively.
- **`key`** — generate ssh keys and manage the gkit-owned `~/.ssh/git_users`.

## The niche

Multi-repo managers (mani, tsrc, gita) clone and run commands across repos. Per-repo
identity tooling (`includeIf`, ssh host aliases) handles keys. But **nothing ships a
submodule-recursive "is everything committed *and* pushed" gate**, and no fleet tool
offers real clone hooks. gkit fills exactly that gap — and shells out to plain `git`,
so there's nothing magic to reverse-engineer.

## What makes it different

- **Transparent — no magic.** Every side effect is printed; `gkit clone` shows the
  exact `git … clone …` it runs.
- **Stateless.** No `~/.gkit` registry. Your conf file plus each repo's own metadata
  (`.gitmodules`, `.envrc`, git config) are the only state.
- **`--dry-run` + confirm** on anything that mutates.

Continue to [Installation](./installation.md).
