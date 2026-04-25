use crate::cast::{self, CastOptions};
use crate::constraint::Requirement;
use crate::grimoire::{Grimoire, Source};
use crate::manifest::{self, Discovered, ManifestSource};
use crate::resolve;
use crate::scry;
use crate::spell::Spell;
use crate::state::CastLog;
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::collections::HashMap;

#[derive(Parser)]
#[command(
    name = "grimoire",
    version,
    about = "Declarative state for Linux desktops"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List spells available in the grimoire.
    Ls(LsArgs),
    /// Render a spell's full schema.
    Show(ShowArgs),
    /// Report state without changing anything (verify + version_check).
    Scry(ScryArgs),
    /// Reach a declared state (idempotent — no-op if verify already passes).
    Cast(CastArgs),
    /// Scaffold a new personal spell. (Not yet implemented.)
    Scribe(ScribeArgs),
}

#[derive(clap::Args)]
pub struct LsArgs {
    /// Show only personal spells (in ~/.config/grimoire/spells/).
    #[arg(long)]
    pub personal: bool,
    /// Show only embedded (shareable) spells.
    #[arg(long, conflicts_with = "personal")]
    pub embedded: bool,
    /// Filter by category.
    #[arg(long)]
    pub category: Option<String>,
}

#[derive(clap::Args)]
pub struct ShowArgs {
    pub spell: String,
}

#[derive(clap::Args)]
pub struct ScryArgs {
    /// Spell to inspect. Omit to use the active manifest, or list all spells
    /// if no manifest is found.
    pub spell: Option<String>,
    /// Use a named profile from the manifest.
    #[arg(long)]
    pub profile: Option<String>,
}

#[derive(clap::Args)]
pub struct CastArgs {
    /// Spell to cast. Omit to cast everything declared by the active manifest.
    pub spell: Option<String>,
    /// Override which channel to use (must exist on the spell).
    /// Only valid with a single named spell.
    #[arg(long)]
    pub via: Option<String>,
    /// Print the plan, don't execute.
    #[arg(long)]
    pub dry_run: bool,
    /// Recast even if `verify` already passes.
    #[arg(long)]
    pub recast: bool,
    /// Use a named profile from the manifest.
    #[arg(long)]
    pub profile: Option<String>,
}

#[derive(clap::Args)]
pub struct ScribeArgs {
    pub name: String,
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Ls(args) => ls(args),
        Command::Show(args) => show(args),
        Command::Cast(args) => cast_cmd(args),
        Command::Scry(args) => scry_cmd(args),
        Command::Scribe(_) => stub("scribe"),
    }
}

fn ls(args: LsArgs) -> Result<()> {
    let g = Grimoire::load()?;
    let mut rows: Vec<(String, &str, String)> = g
        .iter()
        .filter(|(_, e)| match (args.personal, args.embedded) {
            (true, _) => e.source == Source::Personal,
            (_, true) => e.source == Source::Embedded,
            _ => true,
        })
        .filter(|(_, e)| match &args.category {
            Some(c) => e.spell.category.as_deref() == Some(c.as_str()),
            None => true,
        })
        .map(|(name, e)| {
            let src = match e.source {
                Source::Embedded => "embedded",
                Source::Personal => "personal",
            };
            (name.clone(), src, e.spell.summary.clone())
        })
        .collect();

    rows.sort_by(|a, b| a.0.cmp(&b.0));

    if rows.is_empty() {
        println!("(no spells)");
        return Ok(());
    }

    let name_width = rows.iter().map(|(n, _, _)| n.len()).max().unwrap_or(0);
    for (name, src, summary) in rows {
        println!(
            "  {:<width$}  [{}]  {}",
            name,
            src,
            summary,
            width = name_width
        );
    }
    Ok(())
}

fn show(args: ShowArgs) -> Result<()> {
    let g = Grimoire::load()?;
    let entry = g
        .get(&args.spell)
        .with_context(|| format!("spell {:?} not found", args.spell))?;
    let yaml = serde_yml::to_string(&entry.spell).context("re-serializing spell to YAML")?;
    println!("# source: {:?}", entry.source);
    print!("{yaml}");
    Ok(())
}

fn scry_cmd(args: ScryArgs) -> Result<()> {
    let g = Grimoire::load()?;
    let log = CastLog::open().ok();

    // Build (name, spell, optional requirement, is_seed) list. Three flavors:
    //   - explicit spell arg → resolve seed = [that one], shows transitive deps
    //   - manifest found      → resolve seeds from manifest
    //   - no manifest         → list every spell in the grimoire (no deps view)
    let rows: Vec<(String, &Spell, Option<Requirement>, bool)> = match &args.spell {
        Some(name) => {
            // Confirm the spell exists for a friendlier error than the resolver's.
            g.get(name)
                .with_context(|| format!("spell {name:?} not found"))?;
            let plan = resolve::resolve(
                &g,
                &[Requirement {
                    name: name.clone(),
                    constraints: Vec::new(),
                }],
            )?;
            plan.steps
                .into_iter()
                .map(|s| {
                    (
                        s.spell.name.clone(),
                        s.spell,
                        Some(s.requirement),
                        s.is_seed,
                    )
                })
                .collect()
        }
        None => match manifest::discover(&cwd()?)? {
            Some(d) => {
                announce_manifest(&d);
                let seeds = d.manifest.requirements(args.profile.as_deref())?;
                let plan = resolve::resolve(&g, &seeds)?;
                plan.steps
                    .into_iter()
                    .map(|s| {
                        (
                            s.spell.name.clone(),
                            s.spell,
                            Some(s.requirement),
                            s.is_seed,
                        )
                    })
                    .collect()
            }
            None => g
                .iter()
                .map(|(n, e)| (n.clone(), &e.spell, None, true))
                .collect(),
        },
    };

    let name_width = rows.iter().map(|(n, _, _, _)| n.len()).max().unwrap_or(0);

    for (name, spell, req, is_seed) in rows {
        let state = scry::observe(spell, req.as_ref(), log.as_ref())?;
        let mut suffix = String::new();
        if let Some(v) = &state.version {
            suffix.push_str(&format!(" (version: {v})"));
        }
        if !is_seed {
            suffix.push_str(" (dependency)");
        }
        if state.status == scry::Status::Drifted
            && let Some(ch) = &state.last_cast_channel
        {
            suffix.push_str(&format!(" (last cast via {ch})"));
        }
        println!(
            "  {} {:<width$}  {}{}",
            state.status.icon(),
            name,
            state.status.label(),
            suffix,
            width = name_width
        );
    }
    Ok(())
}

fn cast_cmd(args: CastArgs) -> Result<()> {
    let g = Grimoire::load()?;
    let log = CastLog::open().ok();

    // Build seed requirements + per-spell channel overrides.
    let (seeds, channel_overrides) = if let Some(name) = &args.spell {
        let seed = Requirement {
            name: name.clone(),
            constraints: Vec::new(),
        };
        let mut overrides: HashMap<String, String> = HashMap::new();
        if let Some(via) = &args.via {
            overrides.insert(name.clone(), via.clone());
        }
        (vec![seed], overrides)
    } else {
        if args.via.is_some() {
            bail!("--via requires a single spell name");
        }
        let discovered = manifest::discover(&cwd()?)?.context(
            "no manifest found (.grimoire.toml in cwd or ancestors, or ~/.config/grimoire/manifest.toml)",
        )?;
        announce_manifest(&discovered);
        let seeds = discovered.manifest.requirements(args.profile.as_deref())?;
        let overrides: HashMap<String, String> = discovered
            .manifest
            .overrides
            .iter()
            .filter_map(|(k, v)| v.channel.clone().map(|c| (k.clone(), c)))
            .collect();
        (seeds, overrides)
    };

    // Resolve: expand transitively, topo sort, detect cycles, merge constraints.
    let plan = resolve::resolve(&g, &seeds)?;

    let mut summary = CastSummary::default();
    for step in &plan.steps {
        let state = scry::observe(step.spell, Some(&step.requirement), log.as_ref())?;
        let via = channel_overrides.get(&step.spell.name).map(|s| s.as_str());
        let force_recast_seed = step.is_seed && args.recast;
        let dep_marker = if step.is_seed { "" } else { " (dependency)" };

        match state.status {
            scry::Status::Cast => {
                if force_recast_seed {
                    do_cast_step(step, via, args.dry_run, true, &mut summary)?;
                } else {
                    summary.already_cast += 1;
                    eprintln!(
                        "✓ {} already cast{}{}",
                        step.spell.name,
                        state
                            .version
                            .as_deref()
                            .map(|v| format!(" (version: {v})"))
                            .unwrap_or_default(),
                        dep_marker,
                    );
                }
            }
            scry::Status::Missing | scry::Status::Outdated => {
                if !step.is_seed {
                    eprintln!("→ pulling in {}{}", step.spell.name, dep_marker);
                }
                do_cast_step(
                    step,
                    via,
                    args.dry_run,
                    state.status == scry::Status::Outdated,
                    &mut summary,
                )?;
            }
            scry::Status::Drifted => {
                summary.skipped_drifted += 1;
                eprintln!(
                    "⚠ {} drifted (last cast via {}); use `grimoire cast {} --recast` to force",
                    step.spell.name,
                    state.last_cast_channel.as_deref().unwrap_or("?"),
                    step.spell.name,
                );
            }
            scry::Status::Invalid => {
                summary.skipped_invalid += 1;
                eprintln!("? {} invalid (review verify script)", step.spell.name);
            }
        }
    }

    if plan.steps.len() > 1 {
        eprintln!("\n{}", summary.line());
    }
    if summary.verify_still_fails > 0 {
        bail!(
            "{} spells failed to reach state",
            summary.verify_still_fails
        );
    }
    Ok(())
}

fn do_cast_step(
    step: &resolve::PlanStep<'_>,
    via: Option<&str>,
    dry_run: bool,
    recast: bool,
    summary: &mut CastSummary,
) -> Result<()> {
    let outcome = cast::cast(
        step.spell,
        CastOptions {
            via,
            recast,
            dry_run,
        },
    )?;
    match outcome {
        cast::Outcome::AlreadyCast => summary.already_cast += 1,
        cast::Outcome::Cast => summary.cast += 1,
        cast::Outcome::DryRun => summary.dry_run += 1,
        cast::Outcome::VerifyStillFails => summary.verify_still_fails += 1,
    }
    Ok(())
}

#[derive(Default)]
struct CastSummary {
    cast: usize,
    already_cast: usize,
    dry_run: usize,
    skipped_drifted: usize,
    skipped_invalid: usize,
    verify_still_fails: usize,
}

impl CastSummary {
    fn line(&self) -> String {
        let mut parts = Vec::new();
        if self.cast > 0 {
            parts.push(format!("{} cast", self.cast));
        }
        if self.already_cast > 0 {
            parts.push(format!("{} already cast", self.already_cast));
        }
        if self.dry_run > 0 {
            parts.push(format!("{} planned", self.dry_run));
        }
        if self.skipped_drifted > 0 {
            parts.push(format!("{} drifted (skipped)", self.skipped_drifted));
        }
        if self.skipped_invalid > 0 {
            parts.push(format!("{} invalid (skipped)", self.skipped_invalid));
        }
        if self.verify_still_fails > 0 {
            parts.push(format!("{} failed", self.verify_still_fails));
        }
        if parts.is_empty() {
            "(no spells)".into()
        } else {
            parts.join(", ")
        }
    }
}

fn announce_manifest(d: &Discovered) {
    let kind = match d.source {
        ManifestSource::Project => "project manifest",
        ManifestSource::User => "user manifest",
    };
    eprintln!("⌥ {kind}: {}", d.path.display());
}

fn cwd() -> Result<std::path::PathBuf> {
    std::env::current_dir().context("getting current directory")
}

fn stub(cmd: &str) -> Result<()> {
    bail!("`grimoire {cmd}` is not yet implemented in this build");
}
