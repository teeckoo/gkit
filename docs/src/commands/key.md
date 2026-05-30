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
   Linux/Windows omit it.
3. **Check `~/.ssh/config` for `Include git_users`.** This is your own,
   hand-managed file, so gkit treats it carefully:
   - if the line is already there, it says so and leaves the file untouched;
   - if it's missing, it explains that ssh will ignore gkit's host blocks without
     it and **asks for permission** before adding the line (declining is fine —
     it tells you to add it yourself; `-y`/`--yes` adds it without asking).
4. `ssh-add` the key.
5. **Copy the public key to the clipboard** (OS-aware — see `copy` below), ready to
   paste into your provider. If no clipboard tool is found, it prints the key.

`--dry-run` prints the full plan (keygen command, the exact `Host` block, the
`Include` status) without touching anything or prompting.

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

Copies `~/.ssh/id_<alias>.pub` to the clipboard, or prints it if no clipboard
tool is found. The tool is chosen per OS: **macOS** `pbcopy`, **Windows** `clip`,
**Linux** `wl-copy` → `xclip` → `xsel` (first one installed wins).

### `list`

Lists the `Host` aliases (and their `IdentityFile`) gkit owns in `~/.ssh/git_users`.

## Cross-platform

`key` works on macOS, Linux, and Windows:

- **Home / `~/.ssh`** — resolved from `HOME` (Unix/macOS), falling back to
  `USERPROFILE` then `HOMEDRIVE`+`HOMEPATH` on Windows.
- **`UseKeychain yes`** and `ssh-add --apple-use-keychain` are emitted **only on
  macOS**; Linux and Windows omit them.
- **`ssh-keygen` / `ssh-add`** come from OpenSSH (built into modern macOS, Linux,
  and Windows 10+).
- **Clipboard** for `copy` — see the per-OS tools above.
