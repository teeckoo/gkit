# gkit init

Write a starter clone conf in the current directory, with sensible defaults. If the
directory is a git repo with an `origin` like `<host>:<namespace>/<repo>.git`,
`host` and `namespace` are **inferred** from it; otherwise they're left as
placeholders to fill in.

## Synopsis

```sh
gkit init [file] [--force]
```

- `file` — defaults to `repos.toml`.
- `--force` — overwrite an existing file (otherwise `init` refuses).

## Example

```text
$ gkit init
created repos.toml
  host/namespace inferred from origin: tlbb:example-org
```

Generated `repos.toml`:

```toml
# gkit clone config — run `gkit clone <this-file>`.
host      = "tlbb"        # ssh Host alias (~/.ssh/config); URL = host:namespace/repo.git
namespace = "example-org"  # GitHub org / GitLab group / user (optional — a repo may set its own)

# `gkit.baseBranch` = this repo's integration branch. `gkit logoff` and `gkit stmb`
# read it as the "base": the branch stmb returns to, and the one logoff checks
# against. Stamped on every cloned repo here:
post-clone = ["git config gkit.baseBranch main"]   # change to your convention: master / dev

# More optional global settings (uncomment as needed):
# git-flags   = ["-c", "http.lowSpeedLimit=1000"]   # raw flags BEFORE `clone`
# clone-flags = ["--filter=blob:none"]              # raw flags AFTER `clone`
# pre-clone   = "echo cloning $GKIT_REPO"

# One [[repo]] block per repo (name = basename of dir; $VAR/~ expanded):
[[repo]]
dir = "$HOME/work/example"
# namespace   = "other-org"   # override the global namespace for THIS repo
# name        = "example"     # remote repo name if it differs from the dir basename
# depth       = 1
# branch      = "dev"
# clone-flags = ["--no-tags"]
# post-clone  = ["mill compile"]
```

Edit it, then `gkit clone repos.toml`. See [Configuration](../configuration.md) for
every field.
