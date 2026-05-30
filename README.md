# gkit

**A transparent, stateless git/ssh toolkit.** One small binary for the repetitive
git/ssh chores: provision ssh keys, clone a fleet of repos (with pre/post hooks),
verify everything is committed & pushed before you log off, and finish a feature
branch — all explicit, all printed, no hidden state.

[![CI](https://github.com/teeckoo/gkit/actions/workflows/ci.yml/badge.svg)](https://github.com/teeckoo/gkit/actions/workflows/ci.yml)
[![Release](https://github.com/teeckoo/gkit/actions/workflows/release.yml/badge.svg)](https://github.com/teeckoo/gkit/actions/workflows/release.yml)
![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)

---

## Why gkit

- **Transparent — no magic.** Every side effect is printed. `gkit clone` shows the
  exact `git … clone … <url> <dir>` it runs, with your flags in place.
- **Stateless.** gkit keeps no global registry. Your conf file and each repo's own
  metadata (`.gitmodules`, `.envrc`, git config) are the only state.
- **One tool for the whole loop.** ssh keys · config-driven fleet clone with hooks ·
  a **submodule-recursive log-off gate** (commit + push checks across every
  submodule) that no off-the-shelf tool ships · finish-a-feature.
- **Plain tools underneath.** It shells out to `git`/`ssh-keygen`/`ssh-add` — nothing
  to reimplement, cross-platform, easy to audit.

## Install

```sh
# Homebrew (macOS / Linux)
brew install teeckoo/homebrew-tap/gkit

# winget (Windows)
winget install teeckoo.gkit

# Shell installer (macOS / Linux)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/teeckoo/gkit/releases/latest/download/gkit-installer.sh | sh

# From source
cargo install --git https://github.com/teeckoo/gkit gkit
```

## Commands

**SSH key layer (start here — every workflow needs a key first):**

| Command | What it does |
|---|---|
| `gkit key add\|copy\|list` | Generate `id_<alias>`, manage the gkit-owned `~/.ssh/git_users` (OS-aware), copy a public key, list identities. |

**Repo layer (the everyday loop, once a key is in place):**

| Command | What it does |
|---|---|
| `gkit init [file]` | Scaffold a starter clone conf in the cwd (`host`/`namespace` inferred from `origin` when possible). |
| `gkit clone <conf…>` | Clone repos from the given conf file(s) (`repos.toml`, or `*.toml` for a whole dir — a directory arg isn't accepted); submodules switched onto their branch, `.envrc` trusted, every command printed. |
| `gkit logoff [path…]` | Is every repo **+ submodule** committed and pushed? Exit 0 = all clear. `--verbose` for a greppable per-check breakdown; **`--conf <conf…>`** to check every repo in your clone conf(s). |
| `gkit stmb [path]` | "Switch to main branch": return to the base branch, update it, and **safe-delete** the finished feature branch — recursively across submodules. |

## Quick start

Scaffold one with `gkit init` (it infers `host`/`namespace` from your `origin`), or
write it by hand. A clone conf is TOML — `host`/`namespace` at the top (so one ssh
key backs many per-namespace files), a `[[repo]]` block per repo, with optional
global and per-repo flags and pre/post-clone hooks:

```toml
host      = "tlbb"
namespace = "codogenics"      # GitHub org / GitLab group / user; URL = host:namespace/repo.git

clone-flags = ["--filter=blob:none"]          # raw flags for every clone (after `clone`)
post-clone  = ["echo done $GKIT_REPO"]        # runs after every repo's clone

[[repo]]
dir = "$HOME/work/cp-conf"

[[repo]]
dir         = "$HOME/work/cosp"
depth       = 1                               # shallow
branch      = "dev"                           # single branch
clone-flags = ["--no-tags"]                   # per-repo raw flags
post-clone  = ["mill compile"]                # per-repo hook

[[repo]]
dir  = "$HOME/work/cosp-mirror"               # clone into a differently-named dir:
name = "cosp"                                 #   remote repo `cosp` -> dir `cosp-mirror`
```

```sh
gkit clone repos.toml        # clones missing repos (prints each git command)
# or `gkit clone *.toml` (every conf in the cwd, via shell glob)
gkit logoff ~/work           # gate: everything committed & pushed? (recurses submodules)
gkit stmb  ~/work/cp-conf    # done with a feature -> back to base, delete it, verify
```

## Principles

- **Transparent, no magic** — every command is observable and printed.
- **Stateless** — no `~/.gkit`; the config repo and repo metadata are the state.
- **`--dry-run` + confirm** on anything that mutates (skip with `--yes`).
- **Simple > clever** — fewer rules, fewer flags; shell out to plain tools.

## Docs

Full guide and command reference: **https://teeckoo.github.io/gkit**

## Build

```sh
cargo build         # workspace: gkit-core (lib) + gkit (bin)
cargo test          # unit tests (no real git needed — checks run against a fake)
```

## License

MIT.
