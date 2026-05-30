# gkit key

Manage ssh keys/identities. The single convention is the **alias**: it is the ssh
`Host`, and the key is `~/.ssh/id_<alias>`. gkit **owns** `~/.ssh/git_users` — a
generated, disposable file (regenerated, never blind-appended), `Include`d by
`~/.ssh/config`.

## Subcommands

```sh
gkit key add <alias> --email <e> [--host github.com] [--port N] [--dry-run] [-y]
gkit key copy <alias>
gkit key list
```

### `add`

1. `ssh-keygen -t ed25519 -C <email> -f ~/.ssh/id_<alias>` (skipped if it exists).
2. Upsert a `Host <alias>` block into `~/.ssh/git_users` (replacing any old block
   for that alias). The block is **OS-aware** — macOS includes `UseKeychain yes`,
   Linux omits it.
3. Ensure `~/.ssh/config` has `Include git_users`.
4. `ssh-add` the key, then print the public key to upload to your provider.

`--dry-run` prints the full plan (keygen command, the exact `Host` block, the
config change) without touching anything.

Generated block (macOS):

```text
Host acme
  HostName github.com
  User git
  AddKeysToAgent yes
  UseKeychain yes
  IdentitiesOnly yes
  IdentityFile ~/.ssh/id_acme
```

### `copy`

Copies `~/.ssh/id_<alias>.pub` to the clipboard (`pbcopy`), or prints it.

### `list`

Lists the `Host` aliases (and their `IdentityFile`) gkit owns in `~/.ssh/git_users`.
