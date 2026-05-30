# Design principles

gkit follows a few rules deliberately.

## Transparent — no magic

Every side effect is observable and printed. `gkit clone` prints the exact
`git … clone …` it runs; `stmb`/`key` print their plan and (for mutating actions)
confirm first. You should never have to guess what gkit did.

## Stateless

gkit keeps **no global state** — there is no `~/.gkit` registry, no remembered
profiles. The state lives where it belongs and is already version-controlled or
derivable:

- the **clone conf file** (which repos, where, with what flags);
- each repo's own **metadata** — `.gitmodules` (submodule branches), `.envrc`
  (direnv), and `git config` (e.g. `gkit.baseBranch`).

The one file gkit *owns* is `~/.ssh/git_users`: generated, disposable,
`.gitignore`-friendly, and rebuilt (deduped) from your inputs — never blind-appended.

## Plain tools for plain steps

gkit shells out to `git`, `ssh-keygen`, `ssh-add`, `direnv`. It reimplements no git
internals, which keeps it small, auditable, and cross-platform — and means its
behavior matches the tools you already know.

## `--dry-run` + confirm on mutation

Anything that changes your system supports `--dry-run` (print the plan) and prompts
for confirmation before acting (skip with `--yes`).

## The alias convention (`key`)

One short alias ties an identity together: it is the ssh `Host`, and the key is
`~/.ssh/id_<alias>`. Choosing the alias chooses the key name — no separate
`--key-name` to keep in sync.

## Scope

gkit is a set of **specific tools** (clone-with-hooks, the log-off gate, stmb, ssh
keys) — not a general multi-repo task runner. For "run any command across many
repos" use a dedicated tool; gkit deliberately doesn't reinvent that.
