use crate::constraint::{self, Requirement};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A manifest declares which spells (with optional version constraints) a
/// project or machine wants in scope.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub spells: Vec<String>,

    #[serde(default)]
    pub overrides: BTreeMap<String, Override>,

    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    #[serde(default)]
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub spells: Vec<String>,
}

impl Manifest {
    pub fn from_toml(s: &str) -> Result<Self> {
        let m: Manifest = toml::from_str(s).context("parsing manifest TOML")?;
        m.validate()?;
        Ok(m)
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path)
            .with_context(|| format!("reading manifest {}", path.display()))?;
        Self::from_toml(&s).with_context(|| format!("loading manifest {}", path.display()))
    }

    fn validate(&self) -> Result<()> {
        for r in &self.spells {
            constraint::parse(r).with_context(|| format!("invalid spells entry {r:?}"))?;
        }
        for (pname, profile) in &self.profiles {
            for r in &profile.spells {
                constraint::parse(r)
                    .with_context(|| format!("invalid profile {pname:?} spells entry {r:?}"))?;
            }
        }
        for name in self.overrides.keys() {
            // Override keys are bare spell names — must validate as such.
            constraint::parse(name)
                .with_context(|| format!("override key {name:?} is not a valid spell name"))?;
        }
        Ok(())
    }

    /// Resolve to a list of Requirements. If `profile` is given, use that
    /// profile's spell list; otherwise use the top-level `spells`.
    pub fn requirements(&self, profile: Option<&str>) -> Result<Vec<Requirement>> {
        let entries = match profile {
            Some(p) => match self.profiles.get(p) {
                Some(prof) => &prof.spells,
                None => bail!("manifest has no profile named {p:?}"),
            },
            None => &self.spells,
        };
        entries.iter().map(|s| constraint::parse(s)).collect()
    }
}

/// What we found, and where.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub manifest: Manifest,
    pub path: PathBuf,
    pub source: ManifestSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestSource {
    /// `.grimoire.toml` found by walking up from cwd.
    Project,
    /// `~/.config/grimoire/manifest.toml`.
    User,
    /// Path supplied on the CLI via `--manifest`.
    Explicit,
}

/// Load a manifest from an explicit path, bypassing discovery.
pub fn load_explicit(path: &Path) -> Result<Discovered> {
    let manifest = Manifest::from_file(path)?;
    Ok(Discovered {
        manifest,
        path: path.to_path_buf(),
        source: ManifestSource::Explicit,
    })
}

/// Discover the active manifest. Search precedence:
/// 1. `.grimoire.toml` walking up from `start` to `/`.
/// 2. `~/.config/grimoire/manifest.toml`.
///
/// Returns `Ok(None)` if neither is found.
pub fn discover(start: &Path) -> Result<Option<Discovered>> {
    if let Some(p) = find_project_manifest(start) {
        let manifest = Manifest::from_file(&p)?;
        return Ok(Some(Discovered {
            manifest,
            path: p,
            source: ManifestSource::Project,
        }));
    }
    if let Some(p) = user_manifest_path()
        && p.exists()
    {
        let manifest = Manifest::from_file(&p)?;
        return Ok(Some(Discovered {
            manifest,
            path: p,
            source: ManifestSource::User,
        }));
    }
    Ok(None)
}

fn find_project_manifest(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = Some(start);
    while let Some(dir) = cur {
        let candidate = dir.join(".grimoire.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        cur = dir.parent();
    }
    None
}

fn user_manifest_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("grimoire").join("manifest.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_manifest() {
        let s = r#"
spells = [
  "rust >= 1.95",
  "bun >= 1.0",
  "android-studio",
]

[overrides.rust]
channel = "rustup"
"#;
        let m = Manifest::from_toml(s).unwrap();
        assert_eq!(m.spells.len(), 3);
        assert_eq!(m.overrides["rust"].channel.as_deref(), Some("rustup"));

        let reqs = m.requirements(None).unwrap();
        assert_eq!(reqs[0].name, "rust");
        assert!(reqs[0].satisfied_by("1.95.0"));
        assert!(!reqs[0].satisfied_by("1.94.0"));
    }

    #[test]
    fn parses_profiles() {
        let s = r#"
spells = []

[profiles.work]
spells = ["rust", "slack"]

[profiles.gaming]
spells = ["lutris", "discord"]
"#;
        let m = Manifest::from_toml(s).unwrap();
        let work = m.requirements(Some("work")).unwrap();
        assert_eq!(work.len(), 2);
        assert_eq!(work[0].name, "rust");
    }

    #[test]
    fn rejects_invalid_spells_entry() {
        let s = r#"spells = ["BAD NAME >= 1.0"]"#;
        assert!(Manifest::from_toml(s).is_err());
    }

    #[test]
    fn rejects_unknown_profile() {
        let m = Manifest::from_toml("spells = []").unwrap();
        assert!(m.requirements(Some("ghost")).is_err());
    }

    #[test]
    fn discover_finds_project_manifest_in_cwd() {
        let dir = std::env::temp_dir().join(format!(
            "grimoire-discover-cwd-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".grimoire.toml"), r#"spells = ["rust"]"#).unwrap();

        let d = discover(&dir).unwrap().expect("manifest should be found");
        assert_eq!(d.source, ManifestSource::Project);
        assert_eq!(d.path, dir.join(".grimoire.toml"));
        assert_eq!(d.manifest.spells.len(), 1);
    }

    #[test]
    fn load_explicit_reads_arbitrary_path() {
        let dir = std::env::temp_dir().join(format!(
            "grimoire-explicit-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("custom.toml");
        std::fs::write(&path, r#"spells = ["rust"]"#).unwrap();

        let d = load_explicit(&path).expect("explicit load works");
        assert_eq!(d.source, ManifestSource::Explicit);
        assert_eq!(d.path, path);
        assert_eq!(d.manifest.spells.len(), 1);
    }

    #[test]
    fn discover_walks_up_to_parent() {
        let root = std::env::temp_dir().join(format!(
            "grimoire-discover-up-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let nested = root.join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join(".grimoire.toml"), r#"spells = []"#).unwrap();

        let d = discover(&nested).unwrap().expect("walks up to root");
        assert_eq!(d.path, root.join(".grimoire.toml"));
    }
}
