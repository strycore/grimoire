use crate::spell::Spell;
use anyhow::{Context, Result};
use include_dir::{Dir, include_dir};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Embedded shareable spells, bundled at build time.
static EMBEDDED_SPELLS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/spells");

/// The combined collection: shareable spells embedded in the binary plus
/// personal spells loaded from the user's config dir. Personal spells with
/// the same name as an embedded one win.
pub struct Grimoire {
    spells: BTreeMap<String, SpellEntry>,
}

#[derive(Debug, Clone)]
pub struct SpellEntry {
    pub spell: Spell,
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Embedded,
    Personal,
}

impl Grimoire {
    /// Load embedded spells + personal spells from `~/.config/grimoire/spells/`.
    pub fn load() -> Result<Self> {
        let mut spells = load_embedded()?;
        if let Some(dir) = personal_spells_dir() {
            merge_personal(&mut spells, &dir)?;
        }
        Ok(Self { spells })
    }

    /// Load only the embedded spells. Useful for tests and offline tooling.
    pub fn embedded_only() -> Result<Self> {
        Ok(Self {
            spells: load_embedded()?,
        })
    }

    /// Build a grimoire from a hand-rolled list of spells. Test-only.
    #[cfg(test)]
    pub fn from_spells(spells: Vec<crate::spell::Spell>) -> Self {
        let map = spells
            .into_iter()
            .map(|s| {
                (
                    s.name.clone(),
                    SpellEntry {
                        spell: s,
                        source: Source::Embedded,
                    },
                )
            })
            .collect();
        Self { spells: map }
    }

    pub fn get(&self, name: &str) -> Option<&SpellEntry> {
        self.spells.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &SpellEntry)> {
        self.spells.iter()
    }

    pub fn len(&self) -> usize {
        self.spells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spells.is_empty()
    }
}

fn load_embedded() -> Result<BTreeMap<String, SpellEntry>> {
    let mut out = BTreeMap::new();
    for f in EMBEDDED_SPELLS.files() {
        if f.path().extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let yaml = f
            .contents_utf8()
            .with_context(|| format!("embedded spell {} is not UTF-8", f.path().display()))?;
        let spell = Spell::from_yaml(yaml)
            .with_context(|| format!("embedded spell {}", f.path().display()))?;
        let stem = f
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if stem != spell.name {
            anyhow::bail!(
                "embedded spell {} declares name {:?} but filename stem is {:?}",
                f.path().display(),
                spell.name,
                stem,
            );
        }
        out.insert(
            spell.name.clone(),
            SpellEntry {
                spell,
                source: Source::Embedded,
            },
        );
    }
    Ok(out)
}

fn merge_personal(into: &mut BTreeMap<String, SpellEntry>, dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading personal spells dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let spell = Spell::from_file(&path)?;
        into.insert(
            spell.name.clone(),
            SpellEntry {
                spell,
                source: Source::Personal,
            },
        );
    }
    Ok(())
}

fn personal_spells_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("grimoire").join("spells"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_spells_load() {
        let g = Grimoire::embedded_only().expect("embedded spells should parse");
        assert!(!g.is_empty(), "at least one embedded spell expected");
        // Reference spells we shipped with the spec:
        for required in [
            "rust-dev",
            "bun-dev",
            "java-sdk",
            "android-dev",
            "blender",
            "discord",
        ] {
            assert!(
                g.get(required).is_some(),
                "embedded spell {required:?} missing"
            );
        }
    }
}
