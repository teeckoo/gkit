# Configuration

A clone conf is a **TOML** file: `host`/`namespace`, optional global flags and
hooks, and a `[[repo]]` block per repo. It is the only state `gkit clone` needs.

```toml
host      = "tlbb"            # ssh Host alias (from ~/.ssh/config)
namespace = "example-org"      # GitHub org / GitLab group / user (optional — see below)

# global, all optional
git-flags   = ["-c", "http.lowSpeedLimit=1000"]   # raw, BEFORE `clone`
clone-flags = ["--filter=blob:none"]              # raw, AFTER `clone`
pre-clone   = "echo starting $GKIT_REPO"           # string OR list of strings
post-clone  = ["direnv allow ."]

[[repo]]
dir = "$CP_HOME/cp-conf"

[[repo]]
dir         = "$CP_COMMON_LIBS/cosp"
namespace   = "other-org"     # overrides the global namespace for THIS repo
depth       = 1
branch      = "dev"
clone-flags = ["--no-tags"]
pre-clone   = "echo prepping cosp"
post-clone  = ["mill compile"]
```

The clone URL is `<host>:<namespace>/<repo>.git`, where **repo = basename(dir)**.
Because `host`/`namespace` live in the file (not the filename), **one ssh key can
back many conf files** — e.g. one per namespace.

### Namespace: global or per-repo

`namespace` may be set globally, **per `[[repo]]`, or both**. A repo's effective
namespace is its **own `namespace` if present, otherwise the global one** — so a
single conf can span repos from different orgs/users (same `host`):

```toml
host = "gh"                   # one ssh alias; global namespace omitted
[[repo]]
dir = "$HOME/work/foo"
namespace = "alice"           # -> gh:alice/foo.git
[[repo]]
dir = "$HOME/work/bar"
namespace = "bob-org"         # -> gh:bob-org/bar.git
```

The **global `namespace` is optional**, but **every repo must resolve one** (its
own or the global). If any repo has neither, `gkit clone` errors **before cloning
anything**, naming the offending dir.

## Top-level keys

| Key | Meaning |
|---|---|
| `host` | ssh `Host` alias. **Required.** |
| `namespace` | org/group/user; the URL's owner segment. **Optional** — a `[[repo]]` may set its own; every repo must resolve one. |
| `git-flags` | raw flags injected **before** `clone` (git-level). |
| `clone-flags` | raw flags injected **after** `clone`, every repo. |
| `pre-clone` / `post-clone` | global hook commands (string or list). |

## `[[repo]]` keys

| Key | Meaning |
|---|---|
| `dir` | local destination; `$VAR`/`${VAR}`/`~` expanded. |
| `namespace` | org/group/user for **this** repo; overrides the global `namespace`. Required only if there's no global one. |
| `name` | remote repo name (URL's last segment). Defaults to `basename(dir)`; set it to clone a repo into a **differently-named** dir (e.g. `dir = ".../cosp-mirror"`, `name = "cosp"`). |
| `depth = N` | shallow clone (`--depth N`, implies single-branch). |
| `branch = "B"` | `--branch B --single-branch`. |
| `clone-flags` | per-repo raw flags **after** `clone`. |
| `pre-clone` / `post-clone` | per-repo hook commands (string or list). |

## Execution order (per repo)

1. global `pre-clone`
2. repo `pre-clone`
3. `git <git-flags> clone [--depth N] [--branch B --single-branch] --recurse-submodules <clone-flags> <repo clone-flags> <url> <dir>` — **printed**, output captured
4. **built-ins** (unless disabled): git identity (`user.name`/`user.email` on the
   repo **and recursively on every submodule**, if resolved — **printed**), submodule
   init + branch-switch, `direnv allow`
5. global `post-clone`
6. repo `post-clone`

Hooks run via `sh -c`, output shown live, with `$GKIT_REPO`, `$GKIT_DIR`,
`$GKIT_URL`, `$GKIT_HOST`, `$GKIT_NAMESPACE` set — plus `$GKIT_USER_NAME` /
`$GKIT_USER_EMAIL` (the resolved identity, empty when unset). Pre runs in the
parent of the target dir; post runs inside the cloned repo. A hook that exits
non-zero fails that repo.

Git identity is **not** a conf key (the conf is shared across a team): it comes
from the `clone` `--user-name`/`--user-email` flags, or an interactive prompt when
omitted — see [`gkit clone`](./commands/clone.md).

The `post-clone` hooks run only at clone time. To **re-apply** them over repos that
already exist (e.g. to stamp `gkit.baseBranch`/`gkit.solo` on a submodule added
after the initial clone), run [`gkit stamp`](./commands/stamp.md).

### The `gkit.conf` git-config key

`gkit clone` stamps **`gkit.conf`** — the absolute path of the conf that cloned the
repo — into each repo's local git config (and [`gkit stamp --conf`](./commands/stamp.md)
back-fills it on older clones). It lets `gkit stamp` run *inside* a repo with no
argument: it reads `gkit.conf`, re-parses that conf, and re-applies this repo's
`post-clone`. The value is an **absolute, machine-local path** — if the tree moves,
re-anchor it with `git config gkit.conf <new-path>` or `gkit stamp --conf`.

Project-specific config that isn't a gkit concept (e.g. `core.hooksPath .githooks`)
belongs in `post-clone`, not in a gkit built-in — so `gkit stamp` re-applies it like
any other hook.

### The ssh alias vs checked-in URLs (`insteadOf` routing)

The conf's `host` is an **ssh Host alias** (`tlbb`) — purely *local key-selection*
(`tlbb` ≡ `git@bitbucket.org` + `~/.ssh/id_tlbb`). It must **not** end up in checked-in
URLs: a teammate without that alias can't resolve `tlbb:org/repo.git`. So submodule URLs
in `.gitmodules` should be **canonical** — `git@<hostname>:<ns>/repo.git` — and each gkit
developer gets a local rewrite that routes them through the alias's key:

```sh
git config url."tlbb:codogenics/".insteadOf "git@bitbucket.org:codogenics/"
```

`gkit clone` writes this rule for you (see [`clone`](./commands/clone.md) → *SSH-alias
routing*), **namespace-scoped** so multiple aliases on the same host (different clients)
keep their own keys, into a gkit-owned `~/.gitconfig-gkit` that `~/.gitconfig`
`[include]`s. The rules are derived (alias + namespace from the conf, hostname from
`git_users`), so they're regenerable. Migrating an existing repo's `.gitmodules` from
alias to canonical URLs is a one-time manual git op (`git config -f .gitmodules
submodule.<n>.url git@<host>:<ns>/x.git` + `git submodule sync`).

## Built-in, stateless post-clone

Derived from each repo's own on-disk metadata — no config needed:

- **git identity** → `git config user.name`/`user.email` when resolved from
  `--user-name`/`--user-email` or the prompt (your input, not the conf), applied to
  the repo **and every submodule** (`submodule foreach --recursive`). Printed.
- **submodules** → `update --init --recursive`, then each switched onto its
  `.gitmodules` branch (no detached HEAD). Disable with `--no-submodule-branch`.
- **`.envrc`** → `direnv allow` (trust-only; it does **not** evaluate the file, so an
  `.envrc` that runs e.g. `glow ReadMe.md` won't taint output). Disable with
  `--no-direnv`.
