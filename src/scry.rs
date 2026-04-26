use crate::constraint::Requirement;
use crate::shell;
use crate::spell::Spell;
use crate::state::CastLog;
use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// `verify` passes; if a Requirement was supplied, its constraints are met.
    Cast,
    /// `verify` passes but the constraint isn't satisfied by `version_check`.
    Outdated,
    /// `verify` fails and there's no record of a previous successful cast.
    Missing,
    /// `verify` fails but the cast log shows a previous successful cast — the
    /// state has drifted (manual uninstall, broken upgrade, etc.).
    Drifted,
    /// `verify` exited with an unexpected error (script crash, command-not-found
    /// when the script itself is buggy). Reserved for future heuristics; not
    /// emitted today since we treat any non-zero exit as state-not-reached.
    Invalid,
    /// `verify` fails AND no `cast.channels[*]` applies on this distro. The
    /// spell can't be installed here without a new channel being added.
    Unsupported,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Cast => "cast",
            Status::Outdated => "outdated",
            Status::Missing => "missing",
            Status::Drifted => "drifted",
            Status::Invalid => "invalid",
            Status::Unsupported => "unsupported",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Status::Cast => "✓",
            Status::Outdated => "↑",
            Status::Missing => "·",
            Status::Drifted => "⚠",
            Status::Invalid => "?",
            Status::Unsupported => "✗",
        }
    }
}

#[derive(Debug, Clone)]
pub struct State {
    pub status: Status,
    pub version: Option<String>,
    pub last_cast_channel: Option<String>,
}

/// Inspect a single spell against the live system. Read-only.
///
/// If `requirement` is supplied, its constraints are checked against the
/// spell's `version_check` output. A passing `verify` plus an unsatisfied
/// constraint yields [`Status::Outdated`].
pub fn observe(
    spell: &Spell,
    requirement: Option<&Requirement>,
    log: Option<&CastLog>,
) -> Result<State> {
    let verify = shell::check("verify", &spell.verify)
        .with_context(|| format!("running `verify` for spell {}", spell.name))?;

    if verify.ok() {
        let version = if let Some(snippet) = &spell.version_check {
            let out = shell::check("version_check", snippet)
                .with_context(|| format!("running `version_check` for spell {}", spell.name))?;
            if out.ok() && !out.trimmed_stdout().is_empty() {
                Some(out.trimmed_stdout().to_string())
            } else {
                None
            }
        } else {
            None
        };

        let status = match (requirement, version.as_deref()) {
            (Some(req), Some(v)) if !req.satisfied_by(v) => Status::Outdated,
            (Some(req), None) if !req.constraints.is_empty() => {
                // Manifest expects a version, but version_check didn't yield one.
                // Treat as outdated so the user gets a re-cast nudge.
                Status::Outdated
            }
            _ => Status::Cast,
        };

        return Ok(State {
            status,
            version,
            last_cast_channel: None,
        });
    }

    // verify failed → distinguish missing vs drifted vs unsupported.
    if !spell.cast.any_applicable(crate::distro::current()) {
        return Ok(State {
            status: Status::Unsupported,
            version: None,
            last_cast_channel: None,
        });
    }
    let prior = match log {
        Some(l) => l.last_successful_cast(&spell.name)?,
        None => None,
    };
    Ok(State {
        status: if prior.is_some() {
            Status::Drifted
        } else {
            Status::Missing
        },
        version: None,
        last_cast_channel: prior.map(|r| r.channel),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraint;
    use crate::spell::Spell;

    fn echo_spell(version_output: &str) -> Spell {
        // A spell that always verifies and prints `version_output` from
        // version_check. No filesystem dependencies.
        let yaml = format!(
            r#"
name: echo-spell
summary: Test fixture.
verify: "true"
version_check: |
  echo "{version_output}"
cast:
  default: shell
  channels:
    shell:
      type: shell
      summary: noop
      run: "true"
"#
        );
        Spell::from_yaml(&yaml).unwrap()
    }

    #[test]
    fn cast_when_no_constraint() {
        let s = echo_spell("1.95.0");
        let st = observe(&s, None, None).unwrap();
        assert_eq!(st.status, Status::Cast);
        assert_eq!(st.version.as_deref(), Some("1.95.0"));
    }

    #[test]
    fn cast_when_constraint_satisfied() {
        let s = echo_spell("1.95.0");
        let req = constraint::parse("echo-spell >= 1.95").unwrap();
        let st = observe(&s, Some(&req), None).unwrap();
        assert_eq!(st.status, Status::Cast);
    }

    #[test]
    fn outdated_when_constraint_unsatisfied() {
        let s = echo_spell("1.94.0");
        let req = constraint::parse("echo-spell >= 1.95").unwrap();
        let st = observe(&s, Some(&req), None).unwrap();
        assert_eq!(st.status, Status::Outdated);
        assert_eq!(st.version.as_deref(), Some("1.94.0"));
    }

    #[test]
    fn outdated_in_range_check() {
        let s = echo_spell("21.0.10");
        let req = constraint::parse("echo-spell >= 18, < 20").unwrap();
        let st = observe(&s, Some(&req), None).unwrap();
        assert_eq!(st.status, Status::Outdated);
    }
}
