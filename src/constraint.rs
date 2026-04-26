use crate::version;
use anyhow::{Context, Result, anyhow, bail};
use std::cmp::Ordering;

/// One requirement on a spell: name plus an optional list of version
/// comparators that must all hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub name: String,
    pub constraints: Vec<Constraint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    pub op: Op,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
    Eq,
    Ne,
    Ge,
    Gt,
}

impl Op {
    fn from_str(s: &str) -> Option<Op> {
        match s {
            "<" => Some(Op::Lt),
            "<=" => Some(Op::Le),
            "==" => Some(Op::Eq),
            "!=" => Some(Op::Ne),
            ">=" => Some(Op::Ge),
            ">" => Some(Op::Gt),
            _ => None,
        }
    }
}

impl Constraint {
    pub fn satisfied_by(&self, observed: &str) -> bool {
        let cmp = version::compare(observed, &self.version);
        match self.op {
            Op::Lt => cmp == Ordering::Less,
            Op::Le => cmp != Ordering::Greater,
            Op::Eq => cmp == Ordering::Equal,
            Op::Ne => cmp != Ordering::Equal,
            Op::Ge => cmp != Ordering::Less,
            Op::Gt => cmp == Ordering::Greater,
        }
    }
}

impl Requirement {
    pub fn satisfied_by(&self, observed: &str) -> bool {
        self.constraints.iter().all(|c| c.satisfied_by(observed))
    }
}

/// Parse a manifest entry like:
///   "rust"
///   "rust >= 1.95"
///   "java >= 18, < 20"
///   "android-studio == 2025.1.2.12"
pub fn parse(s: &str) -> Result<Requirement> {
    let s = s.trim();
    if s.is_empty() {
        bail!("empty requirement");
    }

    // Split off the spell name: first run of non-space, non-operator characters.
    let split = s
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '=' | '!' | ','))
        .unwrap_or(s.len());
    let (name, rest) = s.split_at(split);
    if name.is_empty() {
        bail!("requirement {s:?} is missing a spell name");
    }
    validate_name(name)?;

    let constraints = if rest.trim().is_empty() {
        Vec::new()
    } else {
        rest.split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(parse_constraint)
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("parsing requirement {s:?}"))?
    };

    Ok(Requirement {
        name: name.to_string(),
        constraints,
    })
}

fn parse_constraint(s: &str) -> Result<Constraint> {
    let s = s.trim();
    // Try two-char operators first, then one-char.
    for len in [2, 1] {
        if s.len() >= len {
            let (op_str, rest) = s.split_at(len);
            if let Some(op) = Op::from_str(op_str) {
                let version = rest.trim();
                if version.is_empty() {
                    bail!("constraint {s:?} is missing a version");
                }
                if version
                    .chars()
                    .next()
                    .is_some_and(|c| matches!(c, '<' | '>' | '=' | '!'))
                {
                    bail!("constraint {s:?} has an unrecognized operator");
                }
                return Ok(Constraint {
                    op,
                    version: version.to_string(),
                });
            }
        }
    }
    Err(anyhow!(
        "constraint {s:?} must start with one of: <, <=, ==, !=, >=, >"
    ))
}

fn validate_name(name: &str) -> Result<()> {
    if !name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        bail!("spell name {name:?} must start with a lowercase letter");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("spell name {name:?} must match [a-z][a-z0-9-]*");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_name() {
        let r = parse("rust").unwrap();
        assert_eq!(r.name, "rust");
        assert!(r.constraints.is_empty());
        assert!(r.satisfied_by("0.0.1")); // anything satisfies an empty constraint set
    }

    #[test]
    fn single_min() {
        let r = parse("rust >= 1.95").unwrap();
        assert_eq!(r.name, "rust");
        assert_eq!(r.constraints.len(), 1);
        assert!(r.satisfied_by("1.95.0"));
        assert!(r.satisfied_by("1.96.0"));
        assert!(!r.satisfied_by("1.94.9"));
    }

    #[test]
    fn range() {
        let r = parse("java >= 18, < 20").unwrap();
        assert_eq!(r.constraints.len(), 2);
        assert!(r.satisfied_by("18.0.1"));
        assert!(r.satisfied_by("19.0.0"));
        assert!(!r.satisfied_by("20.0.0"));
        assert!(!r.satisfied_by("17.9.9"));
    }

    #[test]
    fn equality_dotted_date() {
        let r = parse("android-studio == 2025.1.2.12").unwrap();
        assert!(r.satisfied_by("2025.1.2.12"));
        assert!(!r.satisfied_by("2025.1.2.13"));
    }

    #[test]
    fn inequality() {
        let r = parse("foo != 2.0").unwrap();
        assert!(r.satisfied_by("1.9"));
        assert!(!r.satisfied_by("2.0"));
        assert!(r.satisfied_by("2.0.1"));
    }

    #[test]
    fn whitespace_tolerance() {
        let r = parse("  rust   >=1.95  ").unwrap();
        assert_eq!(r.name, "rust");
        assert!(r.satisfied_by("1.95"));
    }

    #[test]
    fn rejects_bad_name() {
        assert!(parse("Rust-Dev >= 1.0").is_err());
        assert!(parse("1foo").is_err());
        assert!(parse("foo_bar").is_err());
    }

    #[test]
    fn rejects_bad_op() {
        assert!(parse("foo ~ 1.0").is_err());
        assert!(parse("foo === 1.0").is_err());
    }

    #[test]
    fn rejects_missing_version() {
        assert!(parse("foo >=").is_err());
        assert!(parse("foo >= , < 2").is_err());
    }
}
