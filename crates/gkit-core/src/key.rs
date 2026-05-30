//! `key` — ssh key/identity management. Pure, testable core: rendering and
//! regenerating the gkit-owned `~/.ssh/git_users` file (the disposable, `Include`d
//! ssh config), ensuring the `Include` line, and listing hosts. Side effects
//! (ssh-keygen, ssh-add, clipboard, file IO) live in the CLI layer.
//!
//! The single convention is the **alias**: it is the ssh `Host`, and the key is
//! `~/.ssh/id_<alias>`. gkit OWNS `git_users` and rebuilds blocks (dedup), never
//! blind-appends. macOS blocks include `UseKeychain yes`; Linux omits it.

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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(out.matches("Host acme").count(), 1, "exactly one acme block:\n{out}");
        assert!(out.contains("HostName github.com")); // new value
        assert!(!out.contains("HostName old")); // old acme block gone
        assert!(out.contains("Host other")); // unrelated block preserved
        assert!(out.contains("Include project_config")); // preamble preserved
    }

    #[test]
    fn ensure_include_adds_only_when_missing() {
        assert!(ensure_include("Host x\n").unwrap().starts_with("Include git_users"));
        assert_eq!(ensure_include("Include git_users\nHost x\n"), None);
    }

    #[test]
    fn lists_hosts_with_identity() {
        let g = "Host acme\n  IdentityFile ~/.ssh/id_acme\nHost work\n  IdentityFile ~/.ssh/id_work\n";
        assert_eq!(
            list_hosts(g),
            vec![("acme".into(), "~/.ssh/id_acme".into()), ("work".into(), "~/.ssh/id_work".into())]
        );
    }
}
