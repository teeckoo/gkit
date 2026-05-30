# Installation

gkit is a single binary. It needs `git` on your PATH (and `ssh-keygen`/`ssh-add`
for `gkit key`).

## Homebrew (macOS / Linux)

```sh
brew install teeckoo/tap/gkit
```

## winget (Windows)

```powershell
winget install teeckoo.gkit
```

## Shell installer (macOS / Linux)

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/teeckoo/gkit/releases/latest/download/gkit-installer.sh | sh
```

## PowerShell installer (Windows)

```powershell
irm https://github.com/teeckoo/gkit/releases/latest/download/gkit-installer.ps1 | iex
```

## From source

```sh
cargo install --git https://github.com/teeckoo/gkit gkit
```

## Verify

```sh
gkit --version
gkit --help
```
