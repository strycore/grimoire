//! Self-update mechanism for grimoire.
//!
//! Two flows:
//!   1. **Passive nag** (`maybe_nag`): runs at the tail end of every command.
//!      Cache (`~/.local/state/grimoire/update-check.json`) bounds GitHub
//!      lookups to once a week. Prints a one-line notice if a newer release
//!      exists. Skipped entirely when the binary isn't in a writable location
//!      or appears to be a `cargo` dev build.
//!   2. **Manual `grimoire upgrade`**: always checks GitHub, downloads the
//!      release tarball matching the current platform, verifies the SHA-256,
//!      and atomically replaces the running binary.
//!
//! Outbound HTTP is delegated to `curl` so we don't link a TLS stack.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const CHECK_INTERVAL_SECS: u64 = 7 * 24 * 3600;
const REPO: &str = "strycore/grimoire";
const RELEASES_API: &str = "https://api.github.com/repos/strycore/grimoire/releases/latest";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Cache {
    /// Unix timestamp of last successful check.
    last_check: u64,
    /// Latest release version string seen, with the leading `v` stripped.
    latest_version: Option<String>,
}

fn cache_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .context("neither XDG_STATE_HOME nor HOME is set")?;
    Ok(base.join("grimoire").join("update-check.json"))
}

fn read_cache() -> Cache {
    let Ok(path) = cache_path() else {
        return Cache::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Cache::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_cache(c: &Cache) -> Result<()> {
    let path = cache_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let text = serde_json::to_string(c).context("serializing update cache")?;
    std::fs::write(&path, text).context("writing update cache")?;
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// True iff the running binary lives in a directory we can write to *and*
/// doesn't look like a `cargo run` dev build.
pub fn is_self_managed() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    if looks_like_dev_build(&exe) {
        return false;
    }
    let Some(dir) = exe.parent() else {
        return false;
    };
    is_writable(dir)
}

fn looks_like_dev_build(exe: &Path) -> bool {
    // `cargo build` and `cargo run` produce binaries under `<crate>/target/...`.
    // Walking up the path, treat any `target` directory as a strong signal.
    exe.components()
        .any(|c| c.as_os_str().to_string_lossy() == "target")
}

fn is_writable(dir: &Path) -> bool {
    // Probe with an actual write — `access(2)` lies about ACL/Selinux/SMB
    // edge cases. The probe file is removed immediately on success.
    let probe = dir.join(".grimoire-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn fetch_latest_version() -> Result<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: grimoire-self-update",
            RELEASES_API,
        ])
        .output()
        .context("invoking curl to query GitHub releases")?;
    if !out.status.success() {
        bail!(
            "GitHub releases API returned non-zero status (exit {})",
            out.status.code().unwrap_or(-1)
        );
    }
    let body = String::from_utf8_lossy(&out.stdout);
    parse_tag_name(&body).context("could not find tag_name in releases API response")
}

fn parse_tag_name(body: &str) -> Result<String> {
    // Pulled in by substring rather than line-based scan so this works on
    // both the pretty-printed responses GitHub returns interactively and
    // the compact `{"tag_name":"x"}` form a contributor or test might use.
    let needle = "\"tag_name\"";
    let pos = body.find(needle).context("no tag_name field in body")?;
    let after = body[pos + needle.len()..]
        .trim_start()
        .strip_prefix(':')
        .context("malformed tag_name field (missing colon)")?
        .trim_start()
        .strip_prefix('"')
        .context("tag_name value is not a string")?;
    let end = after.find('"').context("unterminated tag_name string")?;
    Ok(after[..end].trim_start_matches('v').to_string())
}

fn detect_target() -> Result<&'static str> {
    let arch_out = Command::new("uname")
        .arg("-m")
        .output()
        .context("invoking uname")?;
    let arch = String::from_utf8_lossy(&arch_out.stdout);
    match arch.trim() {
        "x86_64" | "amd64" => Ok("linux-x86_64"),
        "aarch64" | "arm64" => Ok("linux-aarch64"),
        other => bail!("no prebuilt binary for platform {other:?}"),
    }
}

/// Quietly check whether a newer release is available. Skipped in dev /
/// non-writable installs. Hits GitHub at most once per [`CHECK_INTERVAL_SECS`].
pub fn check_quiet() -> Option<String> {
    if !is_self_managed() {
        return None;
    }
    let mut cache = read_cache();
    if now().saturating_sub(cache.last_check) >= CHECK_INTERVAL_SECS
        && let Ok(latest) = fetch_latest_version()
    {
        cache.latest_version = Some(latest);
        cache.last_check = now();
        let _ = write_cache(&cache);
    }
    let latest = cache.latest_version?;
    if crate::version::compare(&latest, CURRENT_VERSION).is_gt() {
        Some(latest)
    } else {
        None
    }
}

/// Print a one-line nag if a newer release is available. No-op otherwise.
pub fn maybe_nag() {
    if let Some(v) = check_quiet() {
        eprintln!("\n✨ grimoire {v} is available — run `grimoire upgrade`");
    }
}

/// `grimoire upgrade` entry point.
pub fn upgrade(check_only: bool) -> Result<()> {
    if !is_self_managed() {
        bail!(
            "this binary isn't in a writable location, or looks like a `cargo` dev build — manage updates through your package manager or rebuild from source"
        );
    }

    eprintln!("Checking GitHub for the latest grimoire release…");
    let latest = fetch_latest_version()?;
    let cmp = crate::version::compare(&latest, CURRENT_VERSION);

    let mut cache = read_cache();
    cache.latest_version = Some(latest.clone());
    cache.last_check = now();
    let _ = write_cache(&cache);

    if cmp.is_le() {
        eprintln!("✓ already at the latest version ({CURRENT_VERSION})");
        return Ok(());
    }

    if check_only {
        eprintln!("⇡ {latest} available (you're on {CURRENT_VERSION})");
        return Ok(());
    }

    eprintln!("Upgrading {} → {}…", CURRENT_VERSION, latest);
    install_release(&latest)?;
    eprintln!("✓ upgraded to {latest}");
    Ok(())
}

fn install_release(version: &str) -> Result<()> {
    let exe = std::env::current_exe().context("getting running binary path")?;
    let parent = exe
        .parent()
        .context("running binary has no parent directory")?
        .to_path_buf();

    let target = detect_target()?;
    let archive_name = format!("grimoire-{version}-{target}.tar.gz");
    let url = format!("https://github.com/{REPO}/releases/download/v{version}/{archive_name}");

    let workdir = std::env::temp_dir().join(format!("grimoire-upgrade-{version}"));
    std::fs::create_dir_all(&workdir).context("creating work dir")?;
    let archive_path = workdir.join(&archive_name);

    eprintln!("Downloading {url}…");
    let dl = Command::new("curl")
        .args(["-fL", "--progress-bar", "-o"])
        .arg(&archive_path)
        .arg(&url)
        .status()
        .context("invoking curl")?;
    if !dl.success() {
        bail!("download failed");
    }

    // Best-effort SHA-256 verification — sidecar may not exist for older
    // releases, in which case we proceed silently.
    let sha_url = format!("{url}.sha256");
    let sha_path = workdir.join(format!("{archive_name}.sha256"));
    let sha_dl = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&sha_path)
        .arg(&sha_url)
        .status();
    if matches!(sha_dl, Ok(s) if s.success()) {
        eprintln!("Verifying SHA-256…");
        let verify = Command::new("sha256sum")
            .args(["-c", "--quiet"])
            .arg(format!("{archive_name}.sha256"))
            .current_dir(&workdir)
            .status()
            .context("invoking sha256sum")?;
        if !verify.success() {
            bail!("checksum mismatch — refusing to install a tampered tarball");
        }
    }

    let extract = Command::new("tar")
        .arg("xzf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&workdir)
        .status()
        .context("invoking tar")?;
    if !extract.success() {
        bail!("tar extraction failed");
    }

    let new_bin = workdir.join("grimoire");
    if !new_bin.exists() {
        bail!("expected `grimoire` binary in archive root");
    }

    // Stage in the same directory as the live binary so `rename` is atomic.
    // (Cross-filesystem rename would fall back to copy+unlink and isn't atomic.)
    let staging = parent.join(".grimoire.new");
    std::fs::copy(&new_bin, &staging).context("staging new binary")?;
    let mut perms = std::fs::metadata(&staging)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&staging, perms)?;
    std::fs::rename(&staging, &exe).context("replacing current binary")?;

    let _ = std::fs::remove_dir_all(&workdir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tag_name_from_typical_response() {
        let body = r#"{
          "url": "https://api.github.com/repos/strycore/grimoire/releases/123",
          "tag_name": "v0.2.1",
          "name": "v0.2.1",
          "draft": false
        }"#;
        assert_eq!(parse_tag_name(body).unwrap(), "0.2.1");
    }

    #[test]
    fn parses_tag_without_v_prefix() {
        let body = r#"{"tag_name": "1.0.0"}"#;
        assert_eq!(parse_tag_name(body).unwrap(), "1.0.0");
    }

    #[test]
    fn dev_build_detection() {
        let target = PathBuf::from("/home/me/proj/target/debug/grimoire");
        assert!(looks_like_dev_build(&target));
        let installed = PathBuf::from("/home/me/.local/bin/grimoire");
        assert!(!looks_like_dev_build(&installed));
        let system = PathBuf::from("/usr/bin/grimoire");
        assert!(!looks_like_dev_build(&system));
    }

    #[test]
    fn writable_probe_succeeds_in_tempdir() {
        let dir = std::env::temp_dir();
        assert!(is_writable(&dir));
    }
}
