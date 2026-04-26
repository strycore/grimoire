//! Detect the current Linux distribution from `/etc/os-release`.
//!
//! Read once via [`current`], cached for the lifetime of the process. Used by
//! the channel selector to pick which `cast.channels[*]` apply on this machine.
//!
//! Tests inject a fake file path via the `GRIMOIRE_OS_RELEASE` env var.

use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default)]
pub struct Distro {
    /// Lowercase distribution identifier from `ID=`. e.g. `"fedora"`,
    /// `"ubuntu"`, `"arch"`. Empty if unknown.
    pub id: String,
    /// Distros this one is derived from, from `ID_LIKE=`. e.g. Mint reports
    /// `["ubuntu", "debian"]`. Empty if not present.
    pub id_like: Vec<String>,
}

impl Distro {
    /// True if `target` matches this distro's `ID` or any entry in `ID_LIKE`.
    /// Comparison is exact, lowercase.
    pub fn matches(&self, target: &str) -> bool {
        self.id == target || self.id_like.iter().any(|s| s == target)
    }
}

/// Lazily-detected distro. Re-reads `os-release` only on first call.
pub fn current() -> &'static Distro {
    static CURRENT: OnceLock<Distro> = OnceLock::new();
    CURRENT.get_or_init(detect)
}

fn detect() -> Distro {
    let path = std::env::var_os("GRIMOIRE_OS_RELEASE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/os-release"));
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    parse(&content)
}

fn parse(content: &str) -> Distro {
    let mut d = Distro::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key.trim() {
            "ID" => d.id = value.to_lowercase(),
            "ID_LIKE" => {
                d.id_like = value.split_whitespace().map(|s| s.to_lowercase()).collect();
            }
            _ => {}
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fedora() {
        let d = parse(
            r#"
NAME="Fedora Linux"
ID=fedora
VERSION_ID=43
"#,
        );
        assert_eq!(d.id, "fedora");
        assert!(d.id_like.is_empty());
        assert!(d.matches("fedora"));
        assert!(!d.matches("ubuntu"));
    }

    #[test]
    fn ubuntu() {
        let d = parse(
            r#"
ID=ubuntu
ID_LIKE=debian
"#,
        );
        assert_eq!(d.id, "ubuntu");
        assert_eq!(d.id_like, vec!["debian"]);
        assert!(d.matches("ubuntu"));
        assert!(d.matches("debian"));
        assert!(!d.matches("arch"));
    }

    #[test]
    fn linux_mint_inherits_both() {
        let d = parse(
            r#"
ID=linuxmint
ID_LIKE="ubuntu debian"
"#,
        );
        assert!(d.matches("linuxmint"));
        assert!(d.matches("ubuntu"));
        assert!(d.matches("debian"));
    }

    #[test]
    fn handles_quoted_and_unquoted_values() {
        let d = parse(
            r#"
ID="arch"
ID_LIKE='archlinux arch'
"#,
        );
        assert_eq!(d.id, "arch");
        assert!(d.matches("arch"));
        assert!(d.matches("archlinux"));
    }

    #[test]
    fn ignores_comments_and_blanks() {
        let d = parse(
            r#"
# leading comment
ID=fedora

# inline comment
"#,
        );
        assert_eq!(d.id, "fedora");
    }

    #[test]
    fn missing_file_is_empty_distro() {
        let d = parse("");
        assert!(d.id.is_empty());
        assert!(d.id_like.is_empty());
        assert!(!d.matches("anything"));
    }
}
