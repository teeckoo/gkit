# Quick start

## 1. Write a clone conf

A TOML file — `host`/`namespace` at the top (not the filename), so one ssh key can
back many per-namespace confs. A `[[repo]]` block per repo; the name is the dir's
basename.

```toml
host      = "tlbb"            # ssh Host alias  ->  clone URL = host:namespace/repo.git
namespace = "codogenics"      # org / group / user

[[repo]]
dir = "$HOME/work/cp-conf"

[[repo]]
dir   = "$HOME/work/cosp"
depth = 1                     # shallow

[[repo]]
dir    = "$HOME/work/big-lib"
branch = "dev"                # single branch
```

## 2. Clone the fleet

```sh
gkit clone repos.toml        # one conf
gkit clone                   # or: every *.toml in the current dir
gkit clone confs/            # or: every *.toml in a directory
```

gkit prints the exact command for each repo, clones missing ones, switches their
submodules onto the right branch, and trusts any `.envrc`:

```text
+ git clone --recurse-submodules tlbb:codogenics/cp-conf.git /Users/you/work/cp-conf
cloned   cp-conf      /Users/you/work/cp-conf
+ git clone --depth 1 --recurse-submodules tlbb:codogenics/cosp.git /Users/you/work/cosp
cloned   cosp         /Users/you/work/cosp
```

## 3. Before you log off — is everything safe?

```sh
gkit logoff ~/work/cp-conf
```

```text
/Users/you/work/cp-conf/submodule-a   dev true
/Users/you/work/cp-conf               dev true
```

Exit code `0` means every repo and submodule is committed **and** pushed. Add
`--verbose` for a per-check breakdown you can `grep`.

## 4. Done with a feature branch?

```sh
gkit stmb ~/work/cp-conf
```

Switches back to the base branch, pulls, **safe-deletes** the feature branch
(refuses if unmerged unless you pass `--force`), recursively across submodules, and
runs a verifying log-off check. Use `--dry-run` to preview.

Next: the full [Configuration](./configuration.md) reference.
