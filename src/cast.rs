use crate::shell::{self, CheckOutput};
use crate::spell::Spell;
use crate::state::{CastEvent, CastLog};
use anyhow::{Context, Result, bail};

pub struct CastOptions<'a> {
    pub via: Option<&'a str>,
    pub recast: bool,
    pub dry_run: bool,
}

pub enum Outcome {
    /// Verify already passed; nothing to do.
    AlreadyCast,
    /// `--dry-run`: plan printed, nothing executed.
    DryRun,
    /// All steps ran and verify now passes.
    Cast,
    /// Channel ran but verify still fails after the run.
    VerifyStillFails,
}

pub fn cast(spell: &Spell, opts: CastOptions<'_>) -> Result<Outcome> {
    let distro = crate::distro::current();
    let (channel_name, channel) = spell
        .cast
        .pick_channel(distro, opts.via)
        .with_context(|| format!("spell {:?}: choosing channel", spell.name))?;
    let channel_name = channel_name.to_string();

    // 1. Initial verify.
    let initial = run_verify(spell)?;
    let version_before = if initial.ok() {
        run_version_check(spell)?.map(|c| c.trimmed_stdout().to_string())
    } else {
        None
    };

    if initial.ok() && !opts.recast {
        eprintln!(
            "✓ {} already cast{}",
            spell.name,
            version_before
                .as_deref()
                .map(|v| format!(" (version: {v})"))
                .unwrap_or_default()
        );
        return Ok(Outcome::AlreadyCast);
    }

    // 2. Print the plan.
    eprintln!("→ Casting {} via {channel_name}", spell.name);
    eprintln!("  {}", channel.summary);
    if channel.requires_sudo {
        eprintln!("  (this channel uses sudo; you may be prompted)");
    }

    if opts.dry_run {
        eprintln!("\n[dry-run] would run channel `{channel_name}`:");
        eprintln!("---");
        eprintln!("{}", channel.run.trim_end());
        eprintln!("---");
        return Ok(Outcome::DryRun);
    }

    // 3. before hook.
    if let Some(before) = &spell.before {
        shell::banner("before", &channel_name)?;
        shell::run_or_fail("before hook", before)
            .with_context(|| format!("spell {} before hook", spell.name))?;
    }

    // 4. Channel run.
    shell::banner(&channel_name, "cast")?;
    let exit = shell::run("cast", &channel.run)
        .with_context(|| format!("spell {} cast via {channel_name}", spell.name))?;
    if exit != 0 {
        log_cast(
            spell,
            &channel_name,
            exit,
            version_before.clone(),
            None,
            Some(format!("channel exited with code {exit}")),
        );
        bail!("channel `{channel_name}` exited with code {exit}");
    }

    // 5. after hook.
    if let Some(after) = &spell.after {
        shell::banner("after", &channel_name)?;
        if let Err(e) = shell::run_or_fail("after hook", after) {
            // Don't fail the cast if `after` fails — log and warn. The cast
            // itself succeeded; `after` is best-effort post-config.
            eprintln!("⚠ after hook failed: {e:#}");
        }
    }

    // 6. Re-verify and capture version_after.
    let post = run_verify(spell)?;
    let version_after = if post.ok() {
        run_version_check(spell)?.map(|c| c.trimmed_stdout().to_string())
    } else {
        None
    };

    log_cast(
        spell,
        &channel_name,
        exit,
        version_before,
        version_after.clone(),
        if post.ok() {
            None
        } else {
            Some("verify still fails after cast".into())
        },
    );

    if post.ok() {
        eprintln!(
            "✓ {} cast{}",
            spell.name,
            version_after
                .as_deref()
                .map(|v| format!(" (version: {v})"))
                .unwrap_or_default()
        );
        Ok(Outcome::Cast)
    } else {
        eprintln!("✗ {} cast ran but verify still fails", spell.name);
        Ok(Outcome::VerifyStillFails)
    }
}

fn run_verify(spell: &Spell) -> Result<CheckOutput> {
    shell::check("verify", &spell.verify)
        .with_context(|| format!("running `verify` for spell {}", spell.name))
}

fn run_version_check(spell: &Spell) -> Result<Option<CheckOutput>> {
    let Some(snippet) = &spell.version_check else {
        return Ok(None);
    };
    let out = shell::check("version_check", snippet)
        .with_context(|| format!("running `version_check` for spell {}", spell.name))?;
    if out.ok() && !out.trimmed_stdout().is_empty() {
        Ok(Some(out))
    } else {
        Ok(None)
    }
}

fn log_cast(
    spell: &Spell,
    channel: &str,
    exit_code: i32,
    version_before: Option<String>,
    version_after: Option<String>,
    error: Option<String>,
) {
    // Logging is best-effort — never abort the user's cast because we couldn't
    // open SQLite.
    if let Err(e) = (|| -> Result<()> {
        let log = CastLog::open()?;
        log.record(&CastEvent {
            spell: spell.name.clone(),
            channel: channel.to_string(),
            exit_code,
            version_before,
            version_after,
            error,
        })?;
        Ok(())
    })() {
        eprintln!("⚠ could not write cast log: {e:#}");
    }
}
