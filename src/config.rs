//! User-level configuration: `~/.config/grimoire/config.toml`.
//!
//! Today the only thing it carries is an `[env]` table — key/value pairs
//! exported into every shell snippet's environment before it runs. Spells use
//! these for personal-preference paths and similar knobs:
//!
//! ```toml
//! [env]
//! SOFTWARE_DIR = "/home/me/Apps"
//! ```
//!
//! Spells should always reference these with a shell default so they keep
//! working when no config is present:
//!
//! ```bash
//! "${SOFTWARE_DIR:-$HOME/Software}"
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).context("parsing config TOML")
    }

    /// Load `~/.config/grimoire/config.toml` if it exists; otherwise return
    /// an empty config. Missing-file is not an error — most users won't have
    /// one.
    pub fn load() -> Result<Self> {
        match config_path() {
            Some(p) if p.exists() => {
                let s = std::fs::read_to_string(&p)
                    .with_context(|| format!("reading config {}", p.display()))?;
                Self::from_toml(&s).with_context(|| format!("loading config {}", p.display()))
            }
            _ => Ok(Self::default()),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("grimoire").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_section() {
        let s = r#"
[env]
SOFTWARE_DIR = "/home/me/Software"
DEV_DIR = "/home/me/dev"
"#;
        let c = Config::from_toml(s).unwrap();
        assert_eq!(c.env.get("SOFTWARE_DIR").unwrap(), "/home/me/Software");
        assert_eq!(c.env.get("DEV_DIR").unwrap(), "/home/me/dev");
    }

    #[test]
    fn empty_config_ok() {
        let c = Config::from_toml("").unwrap();
        assert!(c.env.is_empty());
    }

    #[test]
    fn rejects_unknown_top_level() {
        let s = r#"mystery = 42"#;
        assert!(Config::from_toml(s).is_err());
    }
}
