# Configuration

A clone conf is a **TOML** file: `host`/`namespace`, optional global flags and
hooks, and a `[[repo]]` block per repo. It is the only state `gkit clone` needs.

```toml
host      = "tlbb"            # ssh Host alias (from ~/.ssh/config)
namespace = "codogenics"      # GitHub org / GitLab group / user

# global, all optional
git-flags   = ["-c", "http.lowSpeedLimit=1000"]   # raw, BEFORE `clone`
clone-flags = ["--filter=blob:none"]              # raw, AFTER `clone`
pre-clone   = "echo starting $GKIT_REPO"           # string OR list of strings
post-clone  = ["direnv allow ."]

[[repo]]
dir = "$CP_HOME/cp-conf"

[[repo]]
dir         = "$CP_COMMON_LIBS/cosp"
depth       = 1
branch      = "dev"
clone-flags = ["--no-tags"]
pre-clone   = "echo prepping cosp"
post-clone  = ["mill compile"]
```

The clone URL is `<host>:<namespace>/<repo>.git`, where **repo = basename(dir)**.
Because `host`/`namespace` live in the file (not the filename), **one ssh key can
back many conf files** — e.g. one per namespace.

## Top-level keys

| Key | Meaning |
|---|---|
| `host` | ssh `Host` alias. |
| `namespace` | org/group/user; the URL's owner segment. |
| `git-flags` | raw flags injected **before** `clone` (git-level). |
| `clone-flags` | raw flags injected **after** `clone`, every repo. |
| `pre-clone` / `post-clone` | global hook commands (string or list). |

## `[[repo]]` keys

| Key | Meaning |
|---|---|
| `dir` | local destination; `$VAR`/`${VAR}`/`~` expanded. |
| `name` | remote repo name (URL's last segment). Defaults to `basename(dir)`; set it to clone a repo into a **differently-named** dir (e.g. `dir = ".../cosp-mirror"`, `name = "cosp"`). |
| `depth = N` | shallow clone (`--depth N`, implies single-branch). |
| `branch = "B"` | `--branch B --single-branch`. |
| `clone-flags` | per-repo raw flags **after** `clone`. |
| `pre-clone` / `post-clone` | per-repo hook commands (string or list). |

## Execution order (per repo)

1. global `pre-clone`
2. repo `pre-clone`
3. `git <git-flags> clone [--depth N] [--branch B --single-branch] --recurse-submodules <clone-flags> <repo clone-flags> <url> <dir>` — **printed**, output captured
4. **built-ins** (unless disabled): submodule init + branch-switch, `direnv allow`
5. global `post-clone`
6. repo `post-clone`

Hooks run via `sh -c`, output shown live, with `$GKIT_REPO`, `$GKIT_DIR`,
`$GKIT_URL`, `$GKIT_HOST`, `$GKIT_NAMESPACE` set (pre runs in the parent of the
target dir; post runs inside the cloned repo). A hook that exits non-zero fails
that repo.

## Built-in, stateless post-clone

Derived from each repo's own on-disk metadata — no config needed:

- **submodules** → `update --init --recursive`, then each switched onto its
  `.gitmodules` branch (no detached HEAD). Disable with `--no-submodule-branch`.
- **`.envrc`** → `direnv allow` (trust-only; it does **not** evaluate the file, so an
  `.envrc` that runs e.g. `glow ReadMe.md` won't taint output). Disable with
  `--no-direnv`.
