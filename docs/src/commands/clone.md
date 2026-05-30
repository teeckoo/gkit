# gkit clone

Clone the repos listed in a conf file. Existing repos are skipped. Every `git`
command is printed (transparency); all subprocess output is captured so a noisy
`.envrc` can't distort it.

## Synopsis

```sh
gkit clone [paths…] [--no-submodule-branch] [--no-direnv]
```

`paths` are conf files and/or directories:

```sh
gkit clone                       # no arg  -> every *.toml in the current dir
gkit clone confs/                # a dir   -> every *.toml inside it
gkit clone codogenics.toml acme.toml   # explicit list
```

Each conf has its own `host`/`namespace`, so multiple per-namespace confs in one
directory are processed in turn (sorted, with a `== <conf> ==` header). A conf that
fails to parse is reported and skipped; the rest still run and the exit code is
non-zero if anything failed.

## What it does, per repo

1. Build and **print** `git <git-flags> clone [tokens] --recurse-submodules
   <clone-flags> <-- flags> <url> <dir>`.
2. Skip if the directory already exists; otherwise clone (output captured).
3. **Submodules** → init + switch each onto its `.gitmodules` branch
   (`--no-submodule-branch` to skip).
4. **`.envrc`** → `direnv allow` (trust-only, no evaluation; `--no-direnv` to skip).

## Flags

| Flag | Effect |
|---|---|
| `--no-submodule-branch` | Leave submodules detached (don't switch to their branch). |
| `--no-direnv` | Don't `direnv allow` repos that have an `.envrc`. |

Per-repo customization (`depth`, `branch`, `clone-flags`), global
`git-flags`/`clone-flags`, and `pre-clone`/`post-clone` hooks live in the
[conf file](../configuration.md). The full step order (global/repo pre → clone →
built-ins → global/repo post) is documented there.

## Example

```toml
host      = "tlbb"
namespace = "codogenics"
clone-flags = ["--filter=blob:none"]

[[repo]]
dir         = "$HOME/work/cosp"
branch      = "dev"
clone-flags = ["--no-tags"]
post-clone  = ["echo done $GKIT_REPO"]
```

```text
$ gkit clone repos.toml
+ git clone --branch dev --single-branch --recurse-submodules --filter=blob:none --no-tags tlbb:codogenics/cosp.git /Users/you/work/cosp
+ echo done $GKIT_REPO
done cosp
cloned   cosp     /Users/you/work/cosp
```
