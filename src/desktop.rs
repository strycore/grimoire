//! Materialize XDG `.desktop` entries (and bundled icons) for spells that
//! declare a `desktop:` block.
//!
//! Many spells install software that doesn't auto-create a menu entry — raw
//! AppImages dropped into `$SOFTWARE_DIR`, manually-extracted tarballs,
//! GitHub-release binaries — and the user is left with a working binary on
//! `$PATH` but nothing they can launch from their app menu. The `desktop:`
//! block fixes that: grimoire writes
//! `~/.local/share/applications/<spell>.desktop` itself, and copies a bundled
//! icon into the user's hicolor tree so `Icon=<name>` resolves.
//!
//! Reconciliation is idempotent — every successful cast (including a no-op
//! "already cast" verify-pass) rewrites the entry from the spell, so editing
//! the spell and re-casting is enough to update the menu entry.

use crate::spell::{DesktopEntry, Spell};
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Write the `.desktop` file and install the icon (if any). No-op for spells
/// without a `desktop:` block.
///
/// `dry_run` only prints what would happen and never touches the filesystem.
pub fn materialize(spell: &Spell, dry_run: bool) -> Result<()> {
    let Some(desktop) = &spell.desktop else {
        return Ok(());
    };

    let apps_dir = applications_dir()?;
    let desktop_path = apps_dir.join(format!("{}.desktop", spell.name));

    if dry_run {
        eprintln!("would write desktop entry to {}", desktop_path.display());
        if let Some(icon) = &desktop.icon
            && !icon.starts_with('/')
            && let Some((_, ext)) = crate::icons::lookup(icon)
        {
            eprintln!(
                "would install icon {icon}.{ext} into {}",
                hicolor_app_dir(ext)?.display(),
            );
        }
        return Ok(());
    }

    if let Some(icon) = &desktop.icon {
        install_icon(icon)?;
    }

    std::fs::create_dir_all(&apps_dir)
        .with_context(|| format!("creating {}", apps_dir.display()))?;
    let body = render(spell, desktop);
    std::fs::write(&desktop_path, body)
        .with_context(|| format!("writing {}", desktop_path.display()))?;

    Ok(())
}

/// Install a bundled or personal icon under `~/.local/share/icons/hicolor/`.
/// If `icon` is an absolute path, do nothing — assume the spell author has
/// already placed the file. If no matching bundled icon is found, do
/// nothing — `Icon=<name>` may still resolve via the system theme.
fn install_icon(icon: &str) -> Result<()> {
    if icon.starts_with('/') {
        return Ok(());
    }
    let Some((bytes, ext)) = crate::icons::lookup(icon) else {
        return Ok(());
    };
    let dir = hicolor_app_dir(ext)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let target = dir.join(format!("{icon}.{ext}"));
    std::fs::write(&target, &bytes)
        .with_context(|| format!("writing icon to {}", target.display()))?;
    // Best-effort cache refresh so newly-added icons show up in app menus
    // without re-login on environments that don't watch the dir. KDE Plasma
    // and modern GNOME watch the dir directly, so this is just for the long
    // tail. Silence stderr — gtk-update-icon-cache complains when there's no
    // `index.theme` next to the cache, which is fine on environments that
    // don't use it.
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .args(["-q", "-t"])
        .arg(icons_root()?.join("hicolor"))
        .stderr(std::process::Stdio::null())
        .status();
    Ok(())
}

/// Render a `.desktop` file body. Per the XDG Desktop Entry spec, `Type`,
/// `Name`, and `Exec` are required for an `Application` entry; everything
/// else is optional.
fn render(spell: &Spell, d: &DesktopEntry) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(256);
    s.push_str("[Desktop Entry]\n");
    s.push_str("Version=1.0\n");
    s.push_str("Type=Application\n");
    let _ = writeln!(s, "Name={}", d.name);
    let comment = d.comment.as_deref().unwrap_or(spell.summary.as_str());
    let _ = writeln!(s, "Comment={comment}");
    let _ = writeln!(s, "Exec={}", d.exec);
    if let Some(icon) = &d.icon {
        let _ = writeln!(s, "Icon={icon}");
    }
    let _ = writeln!(s, "Terminal={}", d.terminal);
    if !d.categories.is_empty() {
        // Trailing semicolon is required by the spec for list values.
        let _ = writeln!(s, "Categories={};", d.categories.join(";"));
    }
    if !d.mime_types.is_empty() {
        let _ = writeln!(s, "MimeType={};", d.mime_types.join(";"));
    }
    if let Some(class) = &d.startup_wm_class {
        let _ = writeln!(s, "StartupWMClass={class}");
    }
    s
}

fn applications_dir() -> Result<PathBuf> {
    Ok(xdg_data_home()?.join("applications"))
}

fn icons_root() -> Result<PathBuf> {
    Ok(xdg_data_home()?.join("icons"))
}

fn hicolor_app_dir(ext: &str) -> Result<PathBuf> {
    let bucket = match ext {
        "svg" => "scalable",
        "png" => "256x256",
        other => anyhow::bail!("unsupported icon extension {other:?}"),
    };
    Ok(icons_root()?.join("hicolor").join(bucket).join("apps"))
}

fn xdg_data_home() -> Result<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .ok_or_else(|| anyhow::anyhow!("HOME/XDG_DATA_HOME not set; cannot locate XDG data dir"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spell::Spell;

    fn spell_with_desktop() -> Spell {
        let yaml = r#"
name: lmstudio
summary: Local LLM GUI.
verify: command -v lmstudio
cast:
  default: shell
  channels:
    shell:
      type: shell
      summary: ""
      run: "true"
desktop:
  name: LM Studio
  exec: lmstudio %U
  icon: lmstudio
  categories: [Development, Science]
  terminal: false
  startup_wm_class: LM Studio
"#;
        Spell::from_yaml(yaml).unwrap()
    }

    #[test]
    fn renders_required_keys() {
        let s = spell_with_desktop();
        let d = s.desktop.as_ref().unwrap();
        let body = render(&s, d);
        assert!(body.starts_with("[Desktop Entry]\n"));
        assert!(body.contains("Type=Application\n"));
        assert!(body.contains("Name=LM Studio\n"));
        assert!(body.contains("Exec=lmstudio %U\n"));
        // Comment falls back to spell.summary.
        assert!(body.contains("Comment=Local LLM GUI.\n"));
        assert!(body.contains("Icon=lmstudio\n"));
        assert!(body.contains("Terminal=false\n"));
        assert!(body.contains("Categories=Development;Science;\n"));
        assert!(body.contains("StartupWMClass=LM Studio\n"));
    }

    #[test]
    fn comment_override_wins_over_summary() {
        let yaml = r#"
name: x
summary: spell summary
verify: "true"
cast:
  default: shell
  channels:
    shell: { type: shell, summary: "", run: "true" }
desktop:
  name: X
  exec: x
  comment: explicit comment
"#;
        let s = Spell::from_yaml(yaml).unwrap();
        let body = render(&s, s.desktop.as_ref().unwrap());
        assert!(body.contains("Comment=explicit comment\n"));
        assert!(!body.contains("Comment=spell summary\n"));
    }

    #[test]
    fn no_desktop_block_is_a_noop() {
        let yaml = r#"
name: x
summary: y
verify: "true"
cast:
  default: shell
  channels:
    shell: { type: shell, summary: "", run: "true" }
"#;
        let s = Spell::from_yaml(yaml).unwrap();
        assert!(s.desktop.is_none());
        materialize(&s, true).unwrap();
    }
}
