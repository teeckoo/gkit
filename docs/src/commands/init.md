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
  host/namespace inferred from origin: tlbb:codogenics
```

Generated `repos.toml`:

```toml
# gkit clone config — run `gkit clone <this-file>`.
host      = "tlbb"        # ssh Host alias (~/.ssh/config); URL = host:namespace/repo.git
namespace = "codogenics"  # GitHub org / GitLab group / user

# Optional global settings (uncomment as needed):
# git-flags   = ["-c", "http.lowSpeedLimit=1000"]   # raw flags BEFORE `clone`
# clone-flags = ["--filter=blob:none"]              # raw flags AFTER `clone`
# pre-clone   = "echo cloning $GKIT_REPO"
# post-clone  = ["direnv allow ."]

# One [[repo]] block per repo (name = basename of dir; $VAR/~ expanded):
[[repo]]
dir = "$HOME/work/example"
# depth       = 1
# branch      = "dev"
# clone-flags = ["--no-tags"]
# post-clone  = ["mill compile"]
```

Edit it, then `gkit clone repos.toml`. See [Configuration](../configuration.md) for
every field.
