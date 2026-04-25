use crate::constraint::{self, Requirement};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// A grimoire spell — one declared state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spell {
    pub name: String,
    pub summary: String,
    pub verify: String,
    pub cast: Cast,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintainer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub distros: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_check: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    #[serde(default, skip_serializing_if = "Provides::is_empty")]
    pub provides: Provides,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cast {
    pub default: String,
    pub channels: BTreeMap<String, Channel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    #[serde(rename = "type")]
    pub kind: ChannelType,
    pub summary: String,
    pub run: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_hint: Option<VersionHint>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_sudo: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Shell,
    Dnf,
    Flatpak,
    Snap,
    Cargo,
    Pip,
    Npm,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VersionHint {
    Latest,
    Distro,
    Pinned,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provides {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binaries: Vec<String>,
}

impl Provides {
    pub fn is_empty(&self) -> bool {
        self.binaries.is_empty()
    }
}

impl Spell {
    /// Parse a spell from raw YAML, then validate cross-field invariants.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let spell: Spell = serde_yml::from_str(yaml).context("parsing spell YAML")?;
        spell.validate()?;
        Ok(spell)
    }

    /// Load a spell from a file. The file's stem is used as the expected
    /// `name`; mismatches are reported.
    pub fn from_file(path: &Path) -> Result<Self> {
        let yaml = std::fs::read_to_string(path)
            .with_context(|| format!("reading spell {}", path.display()))?;
        let spell =
            Self::from_yaml(&yaml).with_context(|| format!("loading spell {}", path.display()))?;
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && stem != spell.name
        {
            bail!(
                "spell file {} declares name {:?} but filename stem is {:?}",
                path.display(),
                spell.name,
                stem,
            );
        }
        Ok(spell)
    }

    fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;

        if self.cast.channels.is_empty() {
            bail!("spell {:?} declares no cast channels", self.name);
        }
        if !self.cast.channels.contains_key(&self.cast.default) {
            bail!(
                "spell {:?} default channel {:?} is not in channels",
                self.name,
                self.cast.default,
            );
        }

        // Each `requires` entry must parse as a Requirement.
        for r in &self.requires {
            constraint::parse(r).with_context(|| {
                format!("spell {:?}: invalid requires entry {:?}", self.name, r)
            })?;
        }

        Ok(())
    }

    pub fn parsed_requires(&self) -> Result<Vec<Requirement>> {
        self.requires.iter().map(|r| constraint::parse(r)).collect()
    }
}

fn validate_name(name: &str) -> Result<()> {
    if !name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        bail!("spell name {name:?} must start with a lowercase letter");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("spell name {name:?} must match [a-z][a-z0-9-]*");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_dev() -> &'static str {
        r#"
name: rust-dev
summary: Rust toolchain
verify: command -v cargo && command -v rustc
version_check: rustc --version | awk '{print $2}'
cast:
  default: rustup
  channels:
    rustup:
      type: shell
      summary: Upstream rustup
      run: curl -sSf https://sh.rustup.rs | sh -s -- -y
      version_hint: latest
    dnf:
      type: dnf
      summary: Fedora packages
      run: sudo dnf install -y rust cargo
      version_hint: distro
      requires_sudo: true
category: development
"#
    }

    #[test]
    fn parses_minimal_spell() {
        let s = Spell::from_yaml(rust_dev()).unwrap();
        assert_eq!(s.name, "rust-dev");
        assert_eq!(s.cast.default, "rustup");
        assert!(s.cast.channels.contains_key("dnf"));
        assert_eq!(s.cast.channels["dnf"].kind, ChannelType::Dnf);
        assert!(s.cast.channels["dnf"].requires_sudo);
    }

    #[test]
    fn rejects_default_channel_not_in_channels() {
        let yaml = r#"
name: x
summary: bad default
verify: "true"
cast:
  default: nope
  channels:
    real:
      type: shell
      summary: ""
      run: "true"
"#;
        assert!(Spell::from_yaml(yaml).is_err());
    }

    #[test]
    fn rejects_bad_name() {
        let yaml = r#"
name: Bad-Name
summary: bad
verify: "true"
cast:
  default: shell
  channels:
    shell:
      type: shell
      summary: ""
      run: "true"
"#;
        assert!(Spell::from_yaml(yaml).is_err());
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = r#"
name: x
summary: x
verify: "true"
mystery_field: 42
cast:
  default: shell
  channels:
    shell:
      type: shell
      summary: ""
      run: "true"
"#;
        assert!(Spell::from_yaml(yaml).is_err());
    }

    #[test]
    fn requires_must_be_parseable() {
        let yaml = r#"
name: x
summary: x
verify: "true"
requires: ["valid >= 1.0", "INVALID NAME"]
cast:
  default: shell
  channels:
    shell:
      type: shell
      summary: ""
      run: "true"
"#;
        assert!(Spell::from_yaml(yaml).is_err());
    }
}
