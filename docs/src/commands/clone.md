# gkit clone

Clone the repos listed in a conf file. Existing repos are skipped. Every `git`
command is printed (transparency); all subprocess output is captured so a noisy
`.envrc` can't distort it.

## Synopsis

```sh
gkit clone <conf…> [--no-submodule-branch] [--no-direnv]
```

`conf…` are **explicit conf file(s)** — at least one is required, and a directory
is not accepted (use a shell glob for "every conf here"):

```sh
gkit clone example-org.toml acme.toml   # explicit list
gkit clone *.toml                      # every conf in the cwd (shell glob)
gkit clone confs/*.toml                # every conf in confs/ (shell glob)
```

`gkit clone` with no file — or with a directory like `gkit clone confs/` — is an
error. This matches how [`logoff --conf`](./logoff.md) takes confs. When several
confs are given they're processed in turn (with a `== <conf> ==` header); each has
its own `host`/`namespace`. A conf that fails to parse is reported and skipped; the
rest still run and the exit code is non-zero if anything failed.

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
namespace = "example-org"
clone-flags = ["--filter=blob:none"]

[[repo]]
dir         = "$HOME/work/cosp"
branch      = "dev"
clone-flags = ["--no-tags"]
post-clone  = ["echo done $GKIT_REPO"]
```

```text
$ gkit clone repos.toml
+ git clone --branch dev --single-branch --recurse-submodules --filter=blob:none --no-tags tlbb:example-org/cosp.git /Users/you/work/cosp
+ echo done $GKIT_REPO
done cosp
cloned   cosp     /Users/you/work/cosp
```
