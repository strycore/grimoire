//! Bundled application icons.
//!
//! Icons live in the top-level `icons/` directory and are embedded at build
//! time. Spells reference them by stem name in `desktop.icon`. At cast time,
//! `crate::desktop` copies the matching file into the user's XDG hicolor
//! tree so `Icon=<name>` in the `.desktop` file resolves natively.
//!
//! Personal overrides at `~/.config/grimoire/icons/<name>.{svg,png}` win
//! over embedded icons, mirroring how personal spells override embedded
//! ones.

use include_dir::{Dir, include_dir};
use std::path::PathBuf;

static EMBEDDED_ICONS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/icons");

/// One of the icon formats grimoire knows how to install. SVG goes to
/// `hicolor/scalable/apps/`; PNG goes to `hicolor/256x256/apps/`.
pub const SUPPORTED_EXTS: &[&str] = &["svg", "png"];

/// Locate an icon by stem name. Returns the file bytes and its extension
/// (without the dot). Personal icons win over embedded ones.
pub fn lookup(name: &str) -> Option<(Vec<u8>, &'static str)> {
    if let Some(dir) = personal_icons_dir() {
        for ext in SUPPORTED_EXTS {
            let p = dir.join(format!("{name}.{ext}"));
            if let Ok(bytes) = std::fs::read(&p) {
                return Some((bytes, *ext));
            }
        }
    }
    for ext in SUPPORTED_EXTS {
        if let Some(f) = EMBEDDED_ICONS.get_file(format!("{name}.{ext}")) {
            return Some((f.contents().to_vec(), *ext));
        }
    }
    None
}

fn personal_icons_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("grimoire").join("icons"))
}
