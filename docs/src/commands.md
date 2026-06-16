# Commands

gkit uses noun-style subcommands, in two layers.

## SSH key layer — start here

Every workflow begins with an ssh identity: you need a key on the host before you
can clone anything. This layer is repo-independent — it manages keys and the
gkit-owned `~/.ssh/git_users`.

| Command | Summary |
|---|---|
| [`key`](./commands/key.md) | Generate `id_<alias>` ssh keys, copy a public key, and manage `~/.ssh/git_users`. |

## Repo layer — the everyday loop

Once a key is in place, these act on git repositories (a single repo or a whole
fleet from a conf):

| Command | Summary |
|---|---|
| [`init`](./commands/init.md) | Scaffold a starter clone conf in the current directory. |
| [`clone`](./commands/clone.md) | Clone the repos in a conf file, with hooks and transparent commands. |
| [`stamp`](./commands/stamp.md) | Re-apply a conf's `post-clone` over existing repos (e.g. stamp config on a late-added submodule). |
| [`logoff`](./commands/logoff.md) | Gate: is every repo + submodule committed and pushed? |
| [`stmb`](./commands/stmb.md) | Switch to the base branch and safe-delete a finished feature, recursively. |
| [`fixsub`](./commands/fixsub.md) | Fix submodule metadata: switch each onto its `.gitmodules` branch and inherit the root identity. |

Run `gkit <command> --help` for the authoritative flag list.
