//! `key` — ssh key/identity management. Pure, testable core: rendering and
//! regenerating the gkit-owned `~/.ssh/git_users` file (the disposable, `Include`d
//! ssh config), ensuring the `Include` line, and listing hosts. Side effects
//! (ssh-keygen, ssh-add, clipboard, file IO) live in the CLI layer.
//!
//! The single convention is the **alias**: it is the ssh `Host`, and the key is
//! `~/.ssh/id_<alias>`. gkit OWNS `git_users` and rebuilds blocks (dedup), never
//! blind-appends. macOS blocks include `UseKeychain yes`; Linux/Windows omit it.
//!
//! Cross-OS: the home directory and clipboard tool differ per platform —
//! [`home_from_env`] and [`clipboard_candidates`] are the pure, testable pieces;
//! the CLI layer wires them to the real environment and processes.

use std::path::PathBuf;

/// Resolve the user's home directory from an env lookup, across OSes:
/// `HOME` (Unix/macOS) → `USERPROFILE` (Windows) → `HOMEDRIVE`+`HOMEPATH`
/// (older Windows). Empty values are ignored. Returns `None` if none are set.
pub fn home_from_env(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let nonempty = |k: &str| get(k).filter(|v| !v.is_empty());
    if let Some(h) = nonempty("HOME") {
        return Some(PathBuf::from(h));
    }
    if let Some(up) = nonempty("USERPROFILE") {
        return Some(PathBuf::from(up));
    }
    if let (Some(d), Some(p)) = (nonempty("HOMEDRIVE"), nonempty("HOMEPATH")) {
        return Some(PathBuf::from(format!("{d}{p}")));
    }
    None
}

/// Ordered clipboard programs to try for a target OS (pass
/// `std::env::consts::OS`: `"macos"`, `"windows"`, else treated as Linux/Unix).
/// Each is `(program, args)`; the CLI runs them in order and the first that
/// spawns and accepts the public key on stdin wins (else it prints the key).
pub fn clipboard_candidates(os: &str) -> Vec<(&'static str, Vec<&'static str>)> {
    match os {
        "macos" => vec![("pbcopy", vec![])],
        "windows" => vec![("clip", vec![])],
        // Linux/BSD: Wayland first, then X11 (xclip, then xsel).
        _ => vec![
            ("wl-copy", vec![]),
            ("xclip", vec!["-selection", "clipboard"]),
            ("xsel", vec!["--clipboard", "--input"]),
        ],
    }
}

/// Standard git hosts offered by `key add`'s interactive provider menu, in menu
/// order. The first is the default; a final "other" entry (index `len + 1` in the
/// menu) lets the user type a custom/private hostname.
pub const PROVIDERS: &[&str] = &["github.com", "bitbucket.org", "gitlab.com"];

/// Outcome of a provider-menu selection (the raw line the user typed).
#[derive(Debug, PartialEq, Eq)]
pub enum ProviderChoice {
    /// A resolved hostname: empty input → the default, a standard pick, or a
    /// hostname typed verbatim (so power users can skip the menu).
    Host(String),
    /// The "other" entry — the caller should prompt for a custom hostname.
    Custom,
    /// A bare number outside the menu range — the caller should re-ask.
    Invalid,
}

/// Map a provider-menu line to a [`ProviderChoice`]. Empty → default (first
/// provider). A bare number selects a menu entry (`len + 1` = "other"); an
/// out-of-range number is [`ProviderChoice::Invalid`]. Any non-numeric input is
/// taken as a literal hostname.
pub fn provider_choice(raw: &str) -> ProviderChoice {
    let t = raw.trim();
    if t.is_empty() {
        return ProviderChoice::Host(PROVIDERS[0].to_string());
    }
    if let Ok(n) = t.parse::<usize>() {
        if (1..=PROVIDERS.len()).contains(&n) {
            return ProviderChoice::Host(PROVIDERS[n - 1].to_string());
        }
        if n == PROVIDERS.len() + 1 {
            return ProviderChoice::Custom;
        }
        return ProviderChoice::Invalid;
    }
    ProviderChoice::Host(t.to_string())
}

/// Render the ssh `Host` block for an alias.
pub fn host_block(alias: &str, hostname: &str, port: Option<u16>, macos: bool) -> String {
    let mut s = String::new();
    s.push_str(&format!("Host {alias}\n"));
    s.push_str(&format!("  HostName {hostname}\n"));
    s.push_str("  User git\n");
    s.push_str("  AddKeysToAgent yes\n");
    if macos {
        s.push_str("  UseKeychain yes\n");
    }
    s.push_str("  IdentitiesOnly yes\n");
    s.push_str(&format!("  IdentityFile ~/.ssh/id_{alias}\n"));
    if let Some(p) = port {
        s.push_str(&format!("  Port {p}\n"));
    }
    s
}

/// Regenerate `git_users` with `alias`'s block upserted: remove any existing block
/// for that alias, then append the new one. Never blind-appends duplicates.
pub fn upsert_block(existing: &str, alias: &str, block: &str) -> String {
    let kept = remove_host_block(existing, alias);
    let mut out = kept.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(block.trim_end());
    out.push('\n');
    out
}

/// Remove the `Host <alias>` block (from its `Host` line to the next `Host` line
/// or EOF), keeping everything else.
fn remove_host_block(content: &str, alias: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in content.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("Host ") {
            skipping = rest.split_whitespace().next() == Some(alias);
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Ensure `~/.ssh/config` `Include`s `git_users`. Returns the updated content if a
/// change is needed, else `None`.
pub fn ensure_include(ssh_config: &str) -> Option<String> {
    if ssh_config.lines().any(|l| l.trim() == "Include git_users") {
        return None;
    }
    let mut s = String::from("Include git_users\n");
    if !ssh_config.trim().is_empty() {
        s.push('\n');
        s.push_str(ssh_config);
        if !ssh_config.ends_with('\n') {
            s.push('\n');
        }
    }
    Some(s)
}

/// List `(alias, identity_file)` pairs parsed from `git_users`.
pub fn list_hosts(git_users: &str) -> Vec<(String, String)> {
    let mut hosts: Vec<(String, String)> = Vec::new();
    for line in git_users.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("Host ") {
            if let Some(a) = rest.split_whitespace().next() {
                hosts.push((a.to_string(), String::new()));
            }
        } else if let Some(idf) = t.strip_prefix("IdentityFile ") {
            if let Some(last) = hosts.last_mut() {
                last.1 = idf.trim().to_string();
            }
        }
    }
    hosts
}

/// The `HostName` of the `Host <alias>` block in `git_users`, or `None` if the alias
/// has no block / no `HostName`. Lets `gkit clone` resolve an ssh alias (e.g. `tlbb`)
/// to its real host (`bitbucket.org`) so it can write the namespace-scoped
/// `url."<alias>:<ns>/".insteadOf "git@<hostname>:<ns>/"` rewrite — keeping the alias
/// out of checked-in URLs while still routing through the per-alias key.
pub fn hostname_for(git_users: &str, alias: &str) -> Option<String> {
    let mut in_block = false;
    for line in git_users.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("Host ") {
            in_block = rest.split_whitespace().next() == Some(alias);
        } else if in_block {
            if let Some(h) = t.strip_prefix("HostName ") {
                return Some(h.trim().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_menu_maps_input() {
        // Empty → default (first provider).
        assert_eq!(
            provider_choice(""),
            ProviderChoice::Host("github.com".into())
        );
        assert_eq!(
            provider_choice("  "),
            ProviderChoice::Host("github.com".into())
        );
        // Standard numbered picks.
        assert_eq!(
            provider_choice("1"),
            ProviderChoice::Host("github.com".into())
        );
        assert_eq!(
            provider_choice("2"),
            ProviderChoice::Host("bitbucket.org".into())
        );
        assert_eq!(
            provider_choice("3"),
            ProviderChoice::Host("gitlab.com".into())
        );
        // The "other" entry (len + 1) → Custom.
        assert_eq!(provider_choice("4"), ProviderChoice::Custom);
        // Out-of-range number → Invalid (re-ask).
        assert_eq!(provider_choice("9"), ProviderChoice::Invalid);
        assert_eq!(provider_choice("0"), ProviderChoice::Invalid);
        // Non-numeric → literal hostname (power users skip the menu).
        assert_eq!(
            provider_choice("git.mycorp.com"),
            ProviderChoice::Host("git.mycorp.com".into())
        );
    }

    #[test]
    fn block_is_os_aware() {
        let mac = host_block("acme", "github.com", None, true);
        assert!(mac.contains("Host acme"));
        assert!(mac.contains("IdentityFile ~/.ssh/id_acme"));
        assert!(mac.contains("UseKeychain yes"));
        let linux = host_block("acme", "github.com", None, false);
        assert!(!linux.contains("UseKeychain"));
    }

    #[test]
    fn block_includes_port_when_set() {
        assert!(host_block("a", "h", Some(2222), false).contains("Port 2222"));
        assert!(!host_block("a", "h", None, false).contains("Port"));
    }

    #[test]
    fn upsert_replaces_existing_alias_keeps_others() {
        let existing = "Include project_config\n\nHost acme\n  HostName old\n  IdentityFile ~/.ssh/id_acme\n\nHost other\n  HostName github.com\n";
        let new_block = host_block("acme", "github.com", None, true);
        let out = upsert_block(existing, "acme", &new_block);
        assert_eq!(
            out.matches("Host acme").count(),
            1,
            "exactly one acme block:\n{out}"
        );
        assert!(out.contains("HostName github.com")); // new value
        assert!(!out.contains("HostName old")); // old acme block gone
        assert!(out.contains("Host other")); // unrelated block preserved
        assert!(out.contains("Include project_config")); // preamble preserved
    }

    #[test]
    fn ensure_include_adds_only_when_missing() {
        assert!(ensure_include("Host x\n")
            .unwrap()
            .starts_with("Include git_users"));
        assert_eq!(ensure_include("Include git_users\nHost x\n"), None);
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn home_resolves_home_then_userprofile_then_homedrive() {
        // HOME wins (Unix/macOS).
        assert_eq!(
            home_from_env(env_of(&[
                ("HOME", "/home/u"),
                ("USERPROFILE", "C:\\Users\\u")
            ])),
            Some(PathBuf::from("/home/u"))
        );
        // Windows: no HOME → USERPROFILE.
        assert_eq!(
            home_from_env(env_of(&[("USERPROFILE", "C:\\Users\\u")])),
            Some(PathBuf::from("C:\\Users\\u"))
        );
        // Empty HOME is ignored, falls through.
        assert_eq!(
            home_from_env(env_of(&[("HOME", ""), ("USERPROFILE", "C:\\Users\\u")])),
            Some(PathBuf::from("C:\\Users\\u"))
        );
        // Older Windows: HOMEDRIVE + HOMEPATH.
        assert_eq!(
            home_from_env(env_of(&[("HOMEDRIVE", "C:"), ("HOMEPATH", "\\Users\\u")])),
            Some(PathBuf::from("C:\\Users\\u"))
        );
        // Nothing set → None (CLI then falls back to ".").
        assert_eq!(home_from_env(env_of(&[])), None);
    }

    #[test]
    fn clipboard_candidates_are_os_specific() {
        let names = |os| {
            clipboard_candidates(os)
                .into_iter()
                .map(|(p, _)| p)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("macos"), vec!["pbcopy"]);
        assert_eq!(names("windows"), vec!["clip"]);
        assert_eq!(names("linux"), vec!["wl-copy", "xclip", "xsel"]);
    }

    #[test]
    fn lists_hosts_with_identity() {
        let g =
            "Host acme\n  IdentityFile ~/.ssh/id_acme\nHost work\n  IdentityFile ~/.ssh/id_work\n";
        assert_eq!(
            list_hosts(g),
            vec![
                ("acme".into(), "~/.ssh/id_acme".into()),
                ("work".into(), "~/.ssh/id_work".into())
            ]
        );
    }

    #[test]
    fn hostname_for_resolves_per_block() {
        // Two aliases on the same host with different keys (the multi-client case):
        // each resolves its OWN HostName, scoped to its block.
        let g = "Host ltlgh\n  HostName github.com\n  IdentityFile ~/.ssh/id_ltlgh\n\
                 Host tlbb\n  HostName bitbucket.org\n  IdentityFile ~/.ssh/id_tlbb\n";
        assert_eq!(hostname_for(g, "ltlgh").as_deref(), Some("github.com"));
        assert_eq!(hostname_for(g, "tlbb").as_deref(), Some("bitbucket.org"));
        // unknown alias, or a block with no HostName → None
        assert_eq!(hostname_for(g, "nope"), None);
        assert_eq!(
            hostname_for("Host x\n  IdentityFile ~/.ssh/id_x\n", "x"),
            None
        );
    }
}
