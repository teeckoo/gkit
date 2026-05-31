//! Clone config — structured TOML.
//!
//! ```toml
//! host      = "tlbb"
//! namespace = "example-org"   # GitHub org / GitLab group / user; URL = host:namespace/repo.git
//!
//! # global (all optional; `namespace` too — a repo may set its own instead)
//! git-flags   = ["-c", "http.lowSpeedLimit=1000"]   # raw, BEFORE `clone`
//! clone-flags = ["--filter=blob:none"]              # raw, AFTER `clone`
//! pre-clone   = "echo starting $GKIT_REPO"           # string OR list of strings
//! post-clone  = ["direnv allow ."]
//!
//! [[repo]]
//! dir = "$CP_HOME/cp-conf"
//!
//! [[repo]]
//! dir         = "$CP_COMMON_LIBS/cosp"
//! namespace   = "other-org"   # overrides the global namespace for THIS repo
//! depth       = 1
//! branch      = "dev"
//! clone-flags = ["--no-tags"]
//! post-clone  = ["mill compile"]
//! ```
//!
//! `host` (and optionally `namespace`) live in the file (not the filename) → one
//! ssh key can back many confs. A repo's effective namespace is its own
//! `namespace`, else the global one; at least one must be present. gkit keeps no
//! global state: this file + each repo's own metadata are the state.

use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct CloneConf {
    pub host: String,
    /// Global namespace (org/group/user). Optional — a repo may set its own; every
    /// repo must resolve one (see [`CloneConf::validate`]).
    #[serde(default)]
    pub namespace: Option<String>,
    /// Raw flags applied BEFORE `clone` (git-level, e.g. `-c k=v`).
    #[serde(default)]
    pub git_flags: Vec<String>,
    /// Raw flags applied AFTER `clone` for every repo.
    #[serde(default)]
    pub clone_flags: Vec<String>,
    /// Commands run before every repo's clone.
    #[serde(default)]
    pub pre_clone: Hooks,
    /// Commands run after every repo's clone.
    #[serde(default)]
    pub post_clone: Hooks,
    /// Solo-developer workflow (global default). When set, `gkit clone` stamps
    /// `git config gkit.solo <bool>` on each repo; `gkit logoff`'s correct-branch
    /// check then also flags sitting on the integration branch while feature
    /// branches exist on the remote. A repo may override via its own `solo`.
    #[serde(default)]
    pub solo: Option<bool>,
    #[serde(default)]
    pub repo: Vec<Repo>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Repo {
    /// Local destination dir (raw; `$VAR`/`~` expanded at clone time).
    pub dir: String,
    /// Per-repo namespace (org/group/user) — overrides the global `namespace` for
    /// this repo. One of repo/global namespace must be set.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Remote repo name (the URL's last segment). Defaults to `basename(dir)`; set
    /// this to clone a repo into a differently-named local directory.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub depth: Option<u32>,
    #[serde(default)]
    pub branch: Option<String>,
    /// Per-repo raw flags AFTER `clone`.
    #[serde(default)]
    pub clone_flags: Vec<String>,
    #[serde(default)]
    pub pre_clone: Hooks,
    #[serde(default)]
    pub post_clone: Hooks,
    /// Per-repo solo override (wins over the global `solo`).
    #[serde(default)]
    pub solo: Option<bool>,
}

impl CloneConf {
    /// Effective namespace for a repo: its own `namespace`, else the global one.
    pub fn namespace_for<'a>(&'a self, repo: &'a Repo) -> Option<&'a str> {
        repo.namespace.as_deref().or(self.namespace.as_deref())
    }

    /// Effective `solo` for a repo: its own `solo`, else the global one. `None`
    /// means "not configured" — `gkit clone` then stamps nothing (logoff defaults
    /// to team).
    pub fn solo_for(&self, repo: &Repo) -> Option<bool> {
        repo.solo.or(self.solo)
    }

    /// Every repo must resolve a namespace (per-repo or global). Returns an error
    /// naming the offending dir(s) — call before cloning so nothing runs when a
    /// namespace is missing.
    pub fn validate(&self) -> Result<(), String> {
        let missing: Vec<&str> = self
            .repo
            .iter()
            .filter(|r| self.namespace_for(r).is_none())
            .map(|r| r.dir.as_str())
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "no namespace for {} — set a global `namespace` or a per-repo `namespace`",
                missing.join(", ")
            ))
        }
    }
}

impl Repo {
    /// Remote repo name (drives the clone URL): explicit `name`, else basename(dir).
    pub fn name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            self.dir
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(&self.dir)
                .to_string()
        })
    }
}

/// A hook field: TOML may give a single string or a list of strings.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Hooks(pub Vec<String>);

impl<'de> Deserialize<'de> for Hooks {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OneOrMany {
            One(String),
            Many(Vec<String>),
        }
        Ok(match OneOrMany::deserialize(d)? {
            OneOrMany::One(s) => Hooks(vec![s]),
            OneOrMany::Many(v) => Hooks(v),
        })
    }
}

/// Parse the TOML clone config.
pub fn parse(text: &str) -> Result<CloneConf, String> {
    toml::from_str(text).map_err(|e| e.message().to_string())
}

/// Expand a leading `~` and `$VAR`/`${VAR}` using `get` (e.g. `|k| std::env::var(k).ok()`).
/// Unset variables expand to empty (like a shell).
pub fn expand_path(raw: &str, get: impl Fn(&str) -> Option<String>) -> String {
    let mut s = raw.to_string();
    if s == "~" {
        return get("HOME").unwrap_or_default();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        s = format!("{}/{}", get("HOME").unwrap_or_default(), rest);
    }
    expand_vars(&s, get)
}

fn expand_vars(s: &str, get: impl Fn(&str) -> Option<String>) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let (name, next) = if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                match s[i + 2..].find('}').map(|e| i + 2 + e) {
                    Some(e) => (&s[i + 2..e], e + 1),
                    None => (&s[i + 1..i + 1], i + 1),
                }
            } else {
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                (&s[i + 1..j], j)
            };
            if name.is_empty() {
                out.push('$');
                i += 1;
            } else {
                out.push_str(&get(name).unwrap_or_default());
                i = next;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Parse an scp-like `host:namespace/repo.git` URL into `(host, namespace)`.
/// Returns `None` for `https://` or `user@host` forms (gkit uses ssh Host aliases),
/// so `init` only pre-fills when it can do so cleanly.
pub fn scp_url_parts(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    if url.contains("://") || url.contains('@') {
        return None;
    }
    let (host, path) = url.split_once(':')?;
    let (namespace, _repo) = path.rsplit_once('/')?;
    if host.is_empty() || namespace.is_empty() {
        return None;
    }
    Some((host.to_string(), namespace.to_string()))
}

/// A starter clone config (sensible defaults + commented examples). `host`/
/// `namespace` are pre-filled when known, else left as placeholders.
pub fn template(host: Option<&str>, namespace: Option<&str>, solo: bool) -> String {
    let host = host.unwrap_or("<ssh-host-alias>");
    let namespace = namespace.unwrap_or("<namespace>");
    format!(
        r#"# gkit clone config — run `gkit clone <this-file>`.
host      = "{host}"        # ssh Host alias (~/.ssh/config); URL = host:namespace/repo.git
namespace = "{namespace}"   # GitHub org / GitLab group / user (optional — a repo may set its own)

# solo developer? `gkit clone` stamps this into `git config gkit.solo`. When true,
# `gkit logoff` also flags you for sitting on the integration branch while feature
# branches still exist on the remote (a leftover branch = unfinished work). Team
# workflow (false, default) ignores others' remote branches.
solo = {solo}

# `gkit.baseBranch` = this repo's integration branch. `gkit logoff` and `gkit stmb`
# read it as the "base": the branch stmb returns to, and the one logoff checks
# against. Stamped on every cloned repo here:
post-clone = ["git config gkit.baseBranch main"]   # change to your convention: master / dev

# More optional global settings (uncomment as needed):
# git-flags   = ["-c", "http.lowSpeedLimit=1000"]   # raw flags BEFORE `clone`
# clone-flags = ["--filter=blob:none"]              # raw flags AFTER `clone`
# pre-clone   = "echo cloning $GKIT_REPO"

# One [[repo]] block per repo (name = basename of dir; $VAR/~ expanded):
[[repo]]
dir = "$HOME/work/example"
# namespace   = "other-org"   # override the global namespace for THIS repo
# name        = "example"     # remote repo name if it differs from the dir basename
# depth       = 1
# branch      = "dev"
# clone-flags = ["--no-tags"]
# post-clone  = ["mill compile"]
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn parses_minimal_toml() {
        let c = parse(
            "host = \"tlbb\"\nnamespace = \"example-org\"\n[[repo]]\ndir = \"$CP_HOME/cp-conf\"\n",
        )
        .unwrap();
        assert_eq!(c.host, "tlbb");
        assert_eq!(c.namespace.as_deref(), Some("example-org"));
        assert_eq!(c.repo.len(), 1);
        assert_eq!(c.repo[0].name(), "cp-conf");
        assert!(c.git_flags.is_empty() && c.pre_clone.0.is_empty());
    }

    #[test]
    fn parses_full_toml_with_hooks_and_flags() {
        let c = parse(
            r#"
host = "tlbb"
namespace = "example-org"
git-flags = ["-c", "http.x=y"]
clone-flags = ["--filter=blob:none"]
pre-clone = "echo global pre"
post-clone = ["direnv allow ."]

[[repo]]
dir = "$D/cosp"
depth = 1
branch = "dev"
clone-flags = ["--no-tags"]
post-clone = ["mill compile", "echo done"]
"#,
        )
        .unwrap();
        assert_eq!(c.git_flags, ["-c", "http.x=y"]); // PRE
        assert_eq!(c.clone_flags, ["--filter=blob:none"]); // POST global
        assert_eq!(c.pre_clone.0, ["echo global pre"]); // string -> 1-elem list
        assert_eq!(c.post_clone.0, ["direnv allow ."]);
        let r = &c.repo[0];
        assert_eq!(r.depth, Some(1));
        assert_eq!(r.branch.as_deref(), Some("dev"));
        assert_eq!(r.clone_flags, ["--no-tags"]);
        assert_eq!(r.post_clone.0, ["mill compile", "echo done"]); // list kept
    }

    #[test]
    fn name_overrides_basename_for_url() {
        // clone the remote repo `cosp` into a differently-named local dir
        let c = parse(
            "host=\"h\"\nnamespace=\"o\"\n[[repo]]\ndir=\"$HOME/work/my-cosp\"\nname=\"cosp\"\n",
        )
        .unwrap();
        assert_eq!(c.repo[0].name(), "cosp"); // URL uses `cosp`, dir is `my-cosp`
                                              // default (no name) still uses basename
        let d =
            parse("host=\"h\"\nnamespace=\"o\"\n[[repo]]\ndir=\"$HOME/work/my-cosp\"\n").unwrap();
        assert_eq!(d.repo[0].name(), "my-cosp");
    }

    #[test]
    fn requires_host() {
        // host is required by serde; missing it is a parse error.
        assert!(parse("namespace = \"o\"\n").unwrap_err().contains("host"));
    }

    #[test]
    fn namespace_optional_at_parse() {
        // global namespace is now optional — host alone parses (validation is
        // separate and per-repo).
        let c = parse("host = \"h\"\n").unwrap();
        assert_eq!(c.namespace, None);
        assert!(c.validate().is_ok()); // no repos -> nothing to resolve
    }

    #[test]
    fn per_repo_namespace_overrides_global() {
        let c = parse(
            "host=\"gh\"\nnamespace=\"glob\"\n[[repo]]\ndir=\"$H/a\"\n[[repo]]\ndir=\"$H/b\"\nnamespace=\"bob\"\n",
        )
        .unwrap();
        assert_eq!(c.namespace_for(&c.repo[0]), Some("glob")); // falls back to global
        assert_eq!(c.namespace_for(&c.repo[1]), Some("bob")); // per-repo wins
    }

    #[test]
    fn validate_ok_with_per_repo_namespace_no_global() {
        // no global namespace, but each repo supplies its own -> valid
        let c = parse(
            "host=\"gh\"\n[[repo]]\ndir=\"$H/a\"\nnamespace=\"alice\"\n[[repo]]\ndir=\"$H/b\"\nnamespace=\"bob\"\n",
        )
        .unwrap();
        assert!(c.validate().is_ok());
        assert_eq!(c.namespace_for(&c.repo[0]), Some("alice"));
    }

    #[test]
    fn validate_errors_when_no_namespace() {
        // no global, and this repo has none -> validate names the offending dir
        let c = parse("host=\"gh\"\n[[repo]]\ndir=\"$H/lonely\"\n").unwrap();
        let err = c.validate().unwrap_err();
        assert!(err.contains("$H/lonely"), "names the dir: {err}");
        assert!(err.contains("namespace"));
    }

    #[test]
    fn rejects_unknown_field() {
        assert!(parse("host=\"h\"\nnamespace=\"o\"\nbogus=1\n").is_err());
    }

    #[test]
    fn scp_url_parses_alias_form_only() {
        assert_eq!(
            scp_url_parts("tlbb:example-org/cosp.git"),
            Some(("tlbb".into(), "example-org".into()))
        );
        assert_eq!(
            scp_url_parts("ctl:grp/sub/repo.git"),
            Some(("ctl".into(), "grp/sub".into()))
        ); // gitlab subgroup
        assert_eq!(scp_url_parts("git@github.com:org/repo.git"), None); // user@ form -> skip
        assert_eq!(scp_url_parts("https://github.com/org/repo.git"), None); // https -> skip
        assert_eq!(scp_url_parts("tlbb:noslash"), None);
    }

    #[test]
    fn solo_global_and_per_repo_override() {
        // global solo applies to a repo without its own; per-repo wins.
        let c = parse(
            "host=\"h\"\nnamespace=\"o\"\nsolo=true\n[[repo]]\ndir=\"$H/a\"\n[[repo]]\ndir=\"$H/b\"\nsolo=false\n",
        )
        .unwrap();
        assert_eq!(c.solo, Some(true));
        assert_eq!(c.solo_for(&c.repo[0]), Some(true)); // inherits global
        assert_eq!(c.solo_for(&c.repo[1]), Some(false)); // per-repo wins
                                                         // unset entirely -> None (clone stamps nothing)
        let d = parse("host=\"h\"\nnamespace=\"o\"\n[[repo]]\ndir=\"$H/a\"\n").unwrap();
        assert_eq!(d.solo_for(&d.repo[0]), None);
    }

    #[test]
    fn template_fills_or_placeholders() {
        let filled = template(Some("tlbb"), Some("example-org"), false);
        assert!(filled.contains("host      = \"tlbb\""));
        assert!(filled.contains("namespace = \"example-org\""));
        assert!(filled.contains("solo = false"));
        assert!(filled.contains("[[repo]]"));
        assert!(filled.contains(r#"post-clone = ["git config gkit.baseBranch main"]"#));
        // solo=true variant emits the bool
        assert!(template(Some("h"), Some("o"), true).contains("solo = true"));
        let blank = template(None, None, false);
        assert!(blank.contains("<ssh-host-alias>") && blank.contains("<namespace>"));
        // the template must itself be valid TOML that parses
        assert!(parse(&filled).is_ok());
    }

    #[test]
    fn expands_home_and_vars() {
        let get = env(&[("HOME", "/h"), ("CP_HOME", "/c"), ("X", "/x")]);
        assert_eq!(expand_path("~/foo", &get), "/h/foo");
        assert_eq!(expand_path("$CP_HOME/cp-conf", &get), "/c/cp-conf");
        assert_eq!(expand_path("${X}/b", &get), "/x/b");
        assert_eq!(expand_path("/abs", &get), "/abs");
        assert_eq!(expand_path("$UNSET/y", &get), "/y");
    }
}
