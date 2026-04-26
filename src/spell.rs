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
    /// Distros this channel applies to. Empty ⇒ fall back to the implicit
    /// list from `ChannelType` (e.g. `dnf` ⇒ Fedora-family). Use this only
    /// to override that default — for instance to restrict a `shell` channel
    /// to a single distro, or to broaden a `dnf` channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub distros: Vec<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Channel {
    /// True if this channel applies on `distro`. Universal channel types
    /// (whose implicit list is empty) apply everywhere unless explicitly
    /// restricted via the per-channel `distros: [...]` field.
    pub fn applies_to(&self, distro: &crate::distro::Distro) -> bool {
        if !self.distros.is_empty() {
            return self.distros.iter().any(|d| distro.matches(d));
        }
        let implicit = self.kind.implicit_distros();
        if implicit.is_empty() {
            true
        } else {
            implicit.iter().any(|d| distro.matches(d))
        }
    }
}

impl Cast {
    /// Pick a channel for this distro.
    /// - If `override_name` is given, that channel is used (or an error is
    ///   returned if it doesn't apply on `distro`).
    /// - Else `cast.default` is used if it applies.
    /// - Else the first channel (in alphabetical key order) that applies.
    /// - Else an error is returned saying the spell isn't supported here.
    pub fn pick_channel<'a>(
        &'a self,
        distro: &crate::distro::Distro,
        override_name: Option<&'a str>,
    ) -> anyhow::Result<(&'a str, &'a Channel)> {
        if let Some(name) = override_name {
            let ch = self.channels.get(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "channel {name:?} not found (available: {})",
                    self.channels.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            })?;
            if !ch.applies_to(distro) {
                anyhow::bail!("channel {name:?} doesn't apply on distro {:?}", distro.id,);
            }
            return Ok((name, ch));
        }
        if let Some(ch) = self.channels.get(&self.default)
            && ch.applies_to(distro)
        {
            return Ok((self.default.as_str(), ch));
        }
        for (name, ch) in &self.channels {
            if ch.applies_to(distro) {
                return Ok((name.as_str(), ch));
            }
        }
        anyhow::bail!(
            "no channel applies on distro {:?} (channels: {})",
            distro.id,
            self.channels.keys().cloned().collect::<Vec<_>>().join(", "),
        )
    }

    /// True if at least one channel applies on `distro`.
    pub fn any_applicable(&self, distro: &crate::distro::Distro) -> bool {
        self.channels.values().any(|c| c.applies_to(distro))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Shell,
    Dnf,
    Apt,
    Pacman,
    Flatpak,
    Snap,
    Cargo,
    Pip,
    Npm,
}

impl ChannelType {
    /// Distros this channel type implicitly targets when the channel doesn't
    /// override with its own `distros: [...]` list. Empty list ⇒ universal.
    pub fn implicit_distros(self) -> &'static [&'static str] {
        match self {
            ChannelType::Dnf => &[
                "fedora",
                "rhel",
                "centos",
                "rocky",
                "almalinux",
                "opensuse",
                "opensuse-tumbleweed",
                "opensuse-leap",
            ],
            ChannelType::Apt => &[
                "debian",
                "ubuntu",
                "linuxmint",
                "pop",
                "elementary",
                "raspbian",
            ],
            ChannelType::Pacman => &[
                "arch",
                "manjaro",
                "endeavouros",
                "garuda",
                "cachyos",
                "artix",
                "archlinux",
            ],
            // Cross-distro channel types — apply everywhere by default.
            ChannelType::Shell
            | ChannelType::Flatpak
            | ChannelType::Snap
            | ChannelType::Cargo
            | ChannelType::Pip
            | ChannelType::Npm => &[],
        }
    }
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
