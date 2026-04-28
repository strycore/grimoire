//! Compose-channel execution.
//!
//! Casting a `compose` channel is three steps:
//!   1. **Resolve parameters** — declared params → values, layering
//!      persisted values (so generated secrets carry over across casts) and
//!      an optional user override env.
//!   2. **Fetch the source** — shallow-clone the repo at the pinned `ref`
//!      and copy the `path` subtree into the service dir.
//!   3. **Bring the project up** — `docker compose -p <name> -f <file> up -d`.
//!
//! All filesystem state for a service lives at
//! `~/.local/share/grimoire/services/<spell-name>/`. The materialized `.env`
//! is the single source of truth for a service's env state — both
//! `${VAR}` interpolation in the compose file and `env_file:` directives
//! pick it up automatically.

use crate::shell;
use crate::spell::{Channel, ChannelType, Parameter, ParameterKind, Spell};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const DEFAULT_COMPOSE_FILE: &str = "docker-compose.yml";

/// Execute a compose channel for `spell`. Idempotent — re-running re-fetches
/// the source and re-runs `up -d`, which is how upgrades work (bump `ref` in
/// the spell, re-cast).
pub fn cast_compose(spell: &Spell, channel: &Channel, dry_run: bool) -> Result<()> {
    debug_assert!(matches!(channel.kind, ChannelType::Compose));

    let service_dir = service_dir(&spell.name)?;
    let compose_file = channel
        .compose_file
        .as_deref()
        .unwrap_or(DEFAULT_COMPOSE_FILE);
    let repo = channel
        .repo
        .as_deref()
        .expect("validated by Spell::validate");
    let git_ref = channel
        .git_ref
        .as_deref()
        .expect("validated by Spell::validate");
    let path = channel
        .path
        .as_deref()
        .expect("validated by Spell::validate");

    if dry_run {
        eprintln!(
            "would clone {repo} @ {git_ref} (path: {path}) into {}",
            service_dir.display(),
        );
        eprintln!(
            "would materialize .env from {} declared parameter(s)",
            channel.parameters.len()
        );
        eprintln!(
            "would run: docker compose -p {} -f {compose_file} up -d --remove-orphans",
            spell.name,
        );
        return Ok(());
    }

    fs::create_dir_all(&service_dir)
        .with_context(|| format!("creating service dir {}", service_dir.display()))?;

    // 1. Resolve parameters before fetching, so missing-required errors fail
    //    fast without paying for a clone.
    let resolved = resolve_parameters(&spell.name, &channel.parameters)?;
    write_env(&service_dir.join(".env"), &resolved).context("writing service .env")?;

    // 2. Fetch the source on top of the existing service dir. Re-running
    //    overwrites the materialized files; volumes and `.env` aren't
    //    touched (volumes live wherever the compose says; `.env` is written
    //    after the fetch).
    fetch_source(repo, git_ref, path, &service_dir)?;

    // Re-write .env after the fetch in case the fetch overwrote a file the
    // user-declared params would have populated.
    write_env(&service_dir.join(".env"), &resolved).context("re-writing service .env")?;

    // 3. Bring the project up.
    let snippet = format!(
        "cd {sd}\n\
         docker compose -p {name} -f {file} up -d --remove-orphans",
        sd = sh_quote(&service_dir.display().to_string()),
        name = sh_quote(&spell.name),
        file = sh_quote(compose_file),
    );
    let exit = shell::run("compose up", &snippet)
        .with_context(|| format!("docker compose up for {}", spell.name))?;
    if exit != 0 {
        bail!("docker compose exited with code {exit}");
    }
    Ok(())
}

/// `~/.local/share/grimoire/services/<spell-name>/`. Honors `XDG_DATA_HOME`.
pub fn service_dir(spell_name: &str) -> Result<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .ok_or_else(|| anyhow::anyhow!("HOME/XDG_DATA_HOME not set; cannot locate service dir"))?;
    Ok(base.join("grimoire").join("services").join(spell_name))
}

/// Optional user-level override env at `~/.config/grimoire/services/<name>.env`.
fn user_override_env(spell_name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(
        base.join("grimoire")
            .join("services")
            .join(format!("{spell_name}.env")),
    )
}

/// Resolve declared parameters to concrete values.
///
/// Priority (highest first):
///   1. user override env (`~/.config/grimoire/services/<name>.env`)
///   2. persisted service `.env` (carries over generated secrets)
///   3. spell-declared `default`
///   4. generated secret if `kind: secret`
///
/// Required parameters with no resolved value are collected and reported
/// together so the user fixes them in one pass. Unknown keys in the
/// persisted `.env` (e.g. user-added extras) are preserved.
pub fn resolve_parameters(
    spell_name: &str,
    declared: &BTreeMap<String, Parameter>,
) -> Result<BTreeMap<String, String>> {
    let persisted = read_env_optional(&service_dir(spell_name)?.join(".env"))?.unwrap_or_default();
    let override_env = user_override_env(spell_name)
        .map(|p| read_env_optional(&p))
        .transpose()?
        .flatten()
        .unwrap_or_default();

    // Start with persisted so non-declared keys (user extras) survive re-cast.
    let mut out = persisted.clone();

    let mut missing_required: Vec<String> = Vec::new();
    for (name, p) in declared {
        let resolved = if let Some(v) = override_env.get(name) {
            Some(v.clone())
        } else if let Some(v) = persisted.get(name).filter(|v| !v.is_empty()) {
            Some(v.clone())
        } else if let Some(d) = &p.default {
            Some(d.clone())
        } else if p.kind == Some(ParameterKind::Secret) {
            Some(generate_secret()?)
        } else if p.required {
            missing_required.push(name.clone());
            None
        } else {
            None
        };

        if let Some(v) = resolved {
            out.insert(name.clone(), v);
        }
    }

    if !missing_required.is_empty() {
        let where_to_set = user_override_env(spell_name)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.config/grimoire/services/<name>.env".into());
        bail!(
            "compose channel: missing required parameter(s) for {spell_name:?}: {}\n\
             set them in {where_to_set}",
            missing_required.join(", "),
        );
    }

    // Layer the override on top so it always wins, including for keys not
    // declared in the spell.
    for (k, v) in override_env {
        out.insert(k, v);
    }

    Ok(out)
}

/// Shallow-clone `repo` at `git_ref` into a tempdir, then copy the contents
/// of `path` into `dest`. The `.git` directory is removed in case `path` was
/// the repo root.
fn fetch_source(repo: &str, git_ref: &str, path: &str, dest: &Path) -> Result<()> {
    let path = path.trim_end_matches('/');
    let path = if path.is_empty() { "." } else { path };
    let snippet = format!(
        "tmp=$(mktemp -d)\n\
         trap 'rm -rf \"$tmp\"' EXIT\n\
         git clone --depth 1 --branch {ref_} {repo} \"$tmp\" >&2\n\
         # `cp -T` (GNU): treat dest as a directory to fill, not a target to nest into.\n\
         cp -RT \"$tmp\"/{path} {dest}\n\
         # If `path` was the repo root, .git came along for the ride. Drop it.\n\
         rm -rf {dest}/.git\n",
        ref_ = sh_quote(git_ref),
        repo = sh_quote(repo),
        path = path,
        dest = sh_quote(&dest.display().to_string()),
    );
    let exit = shell::run("compose fetch", &snippet).context("fetching compose source")?;
    if exit != 0 {
        bail!("compose source fetch exited with code {exit}");
    }
    Ok(())
}

fn read_env_optional(path: &Path) -> Result<Option<BTreeMap<String, String>>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("reading env file {}", path.display()))?;
    Ok(Some(parse_env(&raw)))
}

/// Tiny `.env` parser: `KEY=VALUE` per line; `#` comments and blank lines
/// ignored. No interpolation, no quote stripping — values are taken verbatim.
fn parse_env(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in s.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                out.insert(k.to_string(), v.to_string());
            }
        }
    }
    out
}

fn write_env(path: &Path, values: &BTreeMap<String, String>) -> Result<()> {
    let mut s = String::with_capacity(values.len() * 32);
    s.push_str("# Generated by grimoire — re-cast to refresh.\n");
    s.push_str("# To override values without losing them, edit\n");
    s.push_str("# ~/.config/grimoire/services/<spell-name>.env\n");
    for (k, v) in values {
        s.push_str(k);
        s.push('=');
        s.push_str(v);
        s.push('\n');
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, s).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// 32-byte hex random secret. Reads `/dev/urandom` directly so we don't add
/// a dependency just for this.
fn generate_secret() -> Result<String> {
    let mut buf = [0u8; 32];
    let mut f = fs::File::open("/dev/urandom").context("opening /dev/urandom")?;
    f.read_exact(&mut buf).context("reading /dev/urandom")?;
    let mut s = String::with_capacity(64);
    for b in buf {
        s.push_str(&format!("{b:02x}"));
    }
    Ok(s)
}

/// POSIX shell single-quote escape. Safe to interpolate into bash `'...'`
/// contexts even when the input contains apostrophes.
fn sh_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_with_comments_and_blanks() {
        let m = parse_env("# comment\n\nKEY=value\n  OTHER=value with spaces\n");
        assert_eq!(m.get("KEY").unwrap(), "value");
        assert_eq!(m.get("OTHER").unwrap(), "value with spaces");
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn ignores_lines_without_equals() {
        let m = parse_env("just-a-word\nKEY=ok\n");
        assert_eq!(m.get("KEY").unwrap(), "ok");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn shell_quote_handles_apostrophes() {
        assert_eq!(sh_quote("plain"), "'plain'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote(""), "''");
    }

    #[test]
    fn generated_secret_is_64_hex_chars() {
        let s = generate_secret().unwrap();
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn resolve_uses_default_when_no_value() {
        let mut declared = BTreeMap::new();
        declared.insert(
            "X".into(),
            Parameter {
                default: Some("from-default".into()),
                ..Default::default()
            },
        );
        // Unique spell name so we don't collide with any real service dir
        // a developer might have on disk.
        let name = format!("grimoire-test-{}", generate_secret().unwrap());
        let out = resolve_parameters(&name, &declared).unwrap();
        assert_eq!(out.get("X").unwrap(), "from-default");
    }

    #[test]
    fn resolve_generates_secret_when_missing() {
        let mut declared = BTreeMap::new();
        declared.insert(
            "S".into(),
            Parameter {
                kind: Some(ParameterKind::Secret),
                ..Default::default()
            },
        );
        let name = format!("grimoire-test-{}", generate_secret().unwrap());
        let out = resolve_parameters(&name, &declared).unwrap();
        let v = out.get("S").unwrap();
        assert_eq!(v.len(), 64);
    }

    #[test]
    fn resolve_fails_on_missing_required() {
        let mut declared = BTreeMap::new();
        declared.insert(
            "MUSTBESET".into(),
            Parameter {
                required: true,
                ..Default::default()
            },
        );
        let name = format!("grimoire-test-{}", generate_secret().unwrap());
        let err = resolve_parameters(&name, &declared).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("MUSTBESET"), "got: {msg}");
    }
}
