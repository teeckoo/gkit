# gkit key

Manage ssh keys/identities. The single convention is the **alias**: it is the ssh
`Host`, and the key is `~/.ssh/id_<alias>`. gkit **owns** `~/.ssh/git_users` — a
generated, disposable file (regenerated, never blind-appended), `Include`d by
`~/.ssh/config`.

## Subcommands

```sh
gkit key add [alias] [--email <e>] [--host <hostname>] [--port N] [--dry-run] [-y]
gkit key list
```

### `add`

`alias`, `--email`, and `--host` are all **optional on the command line** — when
omitted, `add` **prompts** for them (in an interactive terminal; a non-interactive
run without them is an error rather than a hang). `--host` is asked via a small
**provider menu**:

```text
provider:
  1) github.com  (default)
  2) bitbucket.org
  3) gitlab.com
  4) other (custom hostname)
choose [1-4]:
```

A bare Enter picks the default (github.com); option **4** (or any hostname typed
directly) sets a custom/private host (e.g. `git.mycorp.com`).

Then `add`:

1. `ssh-keygen -t ed25519 -C <email> -f ~/.ssh/id_<alias>` (skipped if it exists).
2. Upsert a `Host <alias>` block into `~/.ssh/git_users` (replacing any old block
   for that alias). The block is **OS-aware** — macOS includes `UseKeychain yes`,
   Linux/Windows omit it.
3. **Check `~/.ssh/config` for `Include git_users`.** This is your own,
   hand-managed file, so gkit treats it carefully:
   - if the line is already there, it says so and leaves the file untouched;
   - if it's missing, it explains that ssh will ignore gkit's host blocks without
     it and **asks for permission** before adding the line, **defaulting to yes**
     (a bare Enter adds it; `-y`/`--yes` adds it without asking; declining is fine —
     it tells you to add it yourself).
4. `ssh-add` the key.
5. **Copy the public key to the clipboard**, ready to paste into your provider. The
   tool is chosen per OS: **macOS** `pbcopy`, **Windows** `clip`, **Linux**
   `wl-copy` → `xclip` → `xsel` (first one installed wins). If none is found, it
   prints the key.

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
- **Clipboard** for the public-key copy in `add` — see the per-OS tools above.
