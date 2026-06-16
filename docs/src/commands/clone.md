# gkit clone

Clone the repos listed in a conf file. Existing repos are skipped. Every `git`
command is printed (transparency); all subprocess output is captured so a noisy
`.envrc` can't distort it.

## Synopsis

```sh
gkit clone <conf…> [--user-name <n>] [--user-email <e>] [--no-submodule-branch] [--no-direnv] [--no-insteadof]
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
3. **Git identity** → `git config user.name`/`user.email` on the repo **and every
   submodule** (recursive — a submodule is its own repo with its own config), if
   resolved (see below; printed, since the values are your explicit input).
4. **`gkit.conf`** → `git config gkit.conf <absolute conf path>` on the repo, so a
   later [`gkit stamp`](./stamp.md) run *inside* the repo (no arg) can find the conf
   that drives it. (Older clones get this back-filled by `gkit stamp --conf`.)
5. **Submodules** → init + switch each onto its `.gitmodules` branch
   (`--no-submodule-branch` to skip).
6. **`.envrc`** → `direnv allow` (trust-only, no evaluation; `--no-direnv` to skip).

## SSH-alias routing (`insteadOf`) — once per conf

The ssh alias (the conf's `host`, e.g. `tlbb`) is **local key-selection**, so it
shouldn't appear in checked-in URLs (a teammate without that alias can't resolve
`tlbb:org/repo.git`). Submodule URLs in `.gitmodules` should therefore be **canonical**
— `git@<hostname>:<ns>/repo.git` — which anyone can clone with their own key. To keep
*your* clones routing through the alias's key, `gkit clone` writes a **namespace-scoped
`insteadOf` rule** (printed, idempotent), once per namespace in the conf:

```sh
git config -f ~/.gitconfig-gkit --replace-all url."tlbb:codogenics/".insteadOf "git@bitbucket.org:codogenics/"
```

so git rewrites a canonical `git@bitbucket.org:codogenics/x.git` → `tlbb:codogenics/x.git`
→ `~/.ssh/id_tlbb`. The **namespace scope** (`codogenics/`) means multiple aliases on
the *same host* (different clients) each keep their own key. The hostname is resolved
from the `Host <alias>` block gkit wrote in `~/.ssh/git_users`.

gkit writes these rules to a **gkit-owned file** (`~/.gitconfig-gkit`) and ensures one
`[include]` line in `~/.gitconfig` — mirroring how it owns `~/.ssh/git_users` and adds
one `Include` to `~/.ssh/config`. So the rules are **regenerable** (delete the file → a
re-clone rebuilds them) and gkit never edits your `~/.gitconfig` otherwise. Skip with
**`--no-insteadof`**; if the alias has no `HostName` in `git_users`, gkit warns and
skips (your clone still works via the alias).

## Git identity (`--user-name` / `--user-email`)

Identity is **per-invocation, never in the conf** — the conf is shared across a
team, so writing one person's name/email into it would stamp *everyone's* clones.
Instead you supply it when you run the command:

- pass `--user-name` / `--user-email`, **or**
- omit them and gkit prompts (in a terminal), defaulting to your current
  `git config user.name`/`user.email` (Enter keeps the default; empty with no
  default skips that field).

With **no flag and no terminal** (e.g. CI) the field is left unset, so the clone
inherits your global git identity — the command never hangs waiting for input.

The resolved identity is applied to the superproject **and recursively to every
submodule** (each is a separate repo, so commits there use the same identity rather
than your global one).

The resolved values are also exported to hooks as `$GKIT_USER_NAME` /
`$GKIT_USER_EMAIL` (empty when unset).

## Flags

| Flag | Effect |
|---|---|
| `--user-name <n>` | `git config user.name` to stamp on each cloned repo (prompted if omitted in a terminal). |
| `--user-email <e>` | `git config user.email` to stamp on each cloned repo (prompted if omitted in a terminal). |
| `--no-submodule-branch` | Leave submodules detached (don't switch to their branch). |
| `--no-direnv` | Don't `direnv allow` repos that have an `.envrc`. |
| `--no-insteadof` | Don't write the namespace-scoped `insteadOf` routing rule for the conf's ssh alias. |

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
$ gkit clone repos.toml --user-name "Jane Dev" --user-email jane@example-org.com
+ git clone --branch dev --recurse-submodules --filter=blob:none --no-tags tlbb:example-org/cosp.git /Users/you/work/cosp
+ git config user.name Jane Dev
+ git config user.email jane@example-org.com
+ git submodule foreach --recursive git config user.name 'Jane Dev'; git config user.email 'jane@example-org.com'
+ echo done $GKIT_REPO
done cosp
cloned   cosp     /Users/you/work/cosp
```
