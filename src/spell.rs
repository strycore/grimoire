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
    /// Bare binary names that must be in `PATH` before this spell can cast
    /// (e.g. `cc`, `make`, `curl`). If missing, grimoire installs the
    /// matching distro package via the system package manager — these are
    /// not modeled as full spells.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_requires: Vec<String>,
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
    /// Bash snippet for shell-style channels (shell/dnf/apt/pacman/flatpak/
    /// snap/cargo/pip/npm). Required for those types and forbidden for
    /// `compose`, which derives its own invocation from the source/parameter
    /// fields below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
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

    // ── compose-only fields ──────────────────────────────────────────────
    /// Git URL to clone the compose source from. Required for `type:
    /// compose`; forbidden otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Git ref (tag, branch, commit SHA) to pin to. Required for `type:
    /// compose`. Pinning to a release tag is the recommended pattern —
    /// upstream composes change without warning.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// Subdirectory inside the cloned repo containing the compose file
    /// (and any sibling files that should be copied alongside it). Use `.`
    /// for repo-root composes. Required for `type: compose`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Compose filename within `path`. Defaults to `docker-compose.yml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_file: Option<String>,
    /// Declared environment parameters. Materialized into a `.env` next to
    /// the compose file at cast time. Compose picks them up via both
    /// `${VAR}` interpolation and `env_file:` automatically.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, Parameter>,
}

/// One declared environment parameter for a `compose` channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// `secret` ⇒ value is auto-generated on first cast and persisted in the
    /// service's `.env` so subsequent casts reuse it. Omit for plain values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ParameterKind>,
    /// True if the parameter must be supplied (no default, not a secret).
    /// Casting fails fast if a required parameter has no value resolved.
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ParameterKind {
    Secret,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Channel {
    /// Cross-field invariants that the JSON schema can't fully express.
    /// Validates the shell-vs-compose split and parameter shape.
    pub fn validate(&self, spell_name: &str, channel_key: &str) -> Result<()> {
        let where_ = || format!("spell {spell_name:?} channel {channel_key:?}");
        match self.kind {
            ChannelType::Compose => {
                if self.run.is_some() {
                    bail!(
                        "{}: `run` is not allowed for `type: compose` (the channel \
                         derives its invocation from repo/ref/path)",
                        where_(),
                    );
                }
                if self.requires_sudo {
                    bail!(
                        "{}: `requires_sudo` is not meaningful for `type: compose`",
                        where_(),
                    );
                }
                let missing: Vec<&str> = [
                    ("repo", self.repo.is_none()),
                    ("ref", self.git_ref.is_none()),
                    ("path", self.path.is_none()),
                ]
                .into_iter()
                .filter(|(_, m)| *m)
                .map(|(n, _)| n)
                .collect();
                if !missing.is_empty() {
                    bail!(
                        "{}: `type: compose` requires {}",
                        where_(),
                        missing.join(", "),
                    );
                }
                for (pname, p) in &self.parameters {
                    if !pname
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                        || pname.is_empty()
                        || pname.chars().next().is_some_and(|c| c.is_ascii_digit())
                    {
                        bail!(
                            "{}: parameter name {pname:?} must match [A-Z_][A-Z0-9_]*",
                            where_(),
                        );
                    }
                    if p.kind == Some(ParameterKind::Secret) && p.default.is_some() {
                        bail!(
                            "{}: parameter {pname:?} is `kind: secret` and \
                             cannot have a `default`",
                            where_(),
                        );
                    }
                    if p.required && (p.default.is_some() || p.kind.is_some()) {
                        bail!(
                            "{}: parameter {pname:?} is `required: true` and \
                             cannot also have `default` or `kind`",
                            where_(),
                        );
                    }
                }
            }
            _ => {
                if self.run.is_none() {
                    bail!(
                        "{}: `run` is required for `type: {:?}`",
                        where_(),
                        self.kind
                    );
                }
                let extras: Vec<&str> = [
                    ("repo", self.repo.is_some()),
                    ("ref", self.git_ref.is_some()),
                    ("path", self.path.is_some()),
                    ("compose_file", self.compose_file.is_some()),
                    ("parameters", !self.parameters.is_empty()),
                ]
                .into_iter()
                .filter(|(_, p)| *p)
                .map(|(n, _)| n)
                .collect();
                if !extras.is_empty() {
                    bail!(
                        "{}: fields {} are only valid for `type: compose`",
                        where_(),
                        extras.join(", "),
                    );
                }
            }
        }
        Ok(())
    }

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
    Compose,
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
            | ChannelType::Npm
            | ChannelType::Compose => &[],
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

        // Per-channel cross-field invariants. The schema can't fully express
        // "this field is only valid for that type", so we enforce it here.
        for (key, ch) in &self.cast.channels {
            ch.validate(&self.name, key)?;
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
name: rust
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
        assert_eq!(s.name, "rust");
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
