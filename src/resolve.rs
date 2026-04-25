//! Dependency resolution for spells.
//!
//! Given a set of seed requirements, expand transitively through each spell's
//! `requires:` list, merge constraints (AND), and produce a topologically
//! sorted plan. Dependencies come before dependents.

use crate::constraint::{Constraint, Requirement};
use crate::grimoire::Grimoire;
use crate::spell::Spell;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, HashMap, VecDeque};

/// One step in a cast plan: a spell, its accumulated constraint, and whether
/// the user explicitly asked for it (`true`) or it was pulled in by another
/// spell's `requires:` (`false`).
#[derive(Debug, Clone)]
pub struct PlanStep<'g> {
    pub spell: &'g Spell,
    pub requirement: Requirement,
    pub is_seed: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedPlan<'g> {
    pub steps: Vec<PlanStep<'g>>,
}

/// Resolve a list of seed requirements into a topologically-sorted plan.
///
/// Errors:
/// - A referenced spell is not in the grimoire.
/// - A `requires:` entry fails to parse.
/// - The dependency graph contains a cycle.
pub fn resolve<'g>(grimoire: &'g Grimoire, seeds: &[Requirement]) -> Result<ResolvedPlan<'g>> {
    // Track which seed names came directly from the caller (vs pulled in
    // transitively).
    let seed_names: std::collections::HashSet<&str> =
        seeds.iter().map(|r| r.name.as_str()).collect();

    // 1. Expand: BFS through `requires`, accumulating per-spell constraints.
    let mut accumulated: BTreeMap<String, Vec<Constraint>> = BTreeMap::new();
    let mut to_visit: VecDeque<Requirement> = seeds.iter().cloned().collect();

    while let Some(req) = to_visit.pop_front() {
        let already_seen = accumulated.contains_key(&req.name);
        accumulated
            .entry(req.name.clone())
            .or_default()
            .extend(req.constraints.iter().cloned());

        if already_seen {
            continue;
        }

        let entry = grimoire.get(&req.name).with_context(|| {
            format!("spell {:?} is referenced but not in the grimoire", req.name)
        })?;

        for req_str in &entry.spell.requires {
            let parsed = crate::constraint::parse(req_str).with_context(|| {
                format!("spell {:?}: invalid requires entry {:?}", req.name, req_str)
            })?;
            to_visit.push_back(parsed);
        }
    }

    // 2. Topological sort (Kahn's algorithm).
    //    Edges go from dependency → dependent (so dependencies come first).
    let mut in_degree: HashMap<String, usize> =
        accumulated.keys().map(|n| (n.clone(), 0)).collect();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();

    for name in accumulated.keys() {
        let entry = grimoire.get(name).expect("expanded; lookup must succeed");
        for req_str in &entry.spell.requires {
            let parsed = crate::constraint::parse(req_str)?;
            // edge: parsed.name → name (parsed.name must be cast before name)
            adj.entry(parsed.name.clone())
                .or_default()
                .push(name.clone());
            *in_degree.get_mut(name).expect("name in_degree present") += 1;
        }
    }

    let mut queue: VecDeque<String> = in_degree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| n.clone())
        .collect();
    // Stable order: sort the initial queue alphabetically so plans are
    // deterministic across runs.
    {
        let mut v: Vec<String> = queue.drain(..).collect();
        v.sort();
        queue.extend(v);
    }

    let mut order: Vec<String> = Vec::with_capacity(accumulated.len());
    while let Some(n) = queue.pop_front() {
        order.push(n.clone());
        if let Some(children) = adj.get(&n) {
            // Sort children alphabetically before queueing for determinism.
            let mut sorted: Vec<&String> = children.iter().collect();
            sorted.sort();
            for child in sorted {
                let d = in_degree.get_mut(child).expect("child in_degree present");
                *d -= 1;
                if *d == 0 {
                    queue.push_back(child.clone());
                }
            }
        }
    }

    if order.len() != accumulated.len() {
        // Cycle: list the spells still with in_degree > 0.
        let stuck: Vec<String> = in_degree
            .iter()
            .filter(|(_, d)| **d > 0)
            .map(|(n, _)| n.clone())
            .collect();
        bail!("dependency cycle detected involving: {}", stuck.join(", "));
    }

    // 3. Build steps in topological order.
    let steps = order
        .into_iter()
        .map(|name| {
            let entry = grimoire.get(&name).expect("name in grimoire");
            let constraints = accumulated.remove(&name).unwrap_or_default();
            PlanStep {
                spell: &entry.spell,
                requirement: Requirement {
                    name: name.clone(),
                    constraints,
                },
                is_seed: seed_names.contains(name.as_str()),
            }
        })
        .collect();

    Ok(ResolvedPlan { steps })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraint;

    fn mk_spell(name: &str, requires: &[&str]) -> Spell {
        let yaml = format!(
            r#"
name: {name}
summary: test
verify: "true"
cast:
  default: shell
  channels:
    shell:
      type: shell
      summary: ""
      run: "true"
requires: {requires:?}
"#,
            requires = requires
        );
        Spell::from_yaml(&yaml).unwrap()
    }

    fn mk_grimoire(spells: Vec<Spell>) -> Grimoire {
        Grimoire::from_spells(spells)
    }

    fn req(s: &str) -> Requirement {
        constraint::parse(s).unwrap()
    }

    fn names<'a>(p: &'a ResolvedPlan<'_>) -> Vec<&'a str> {
        p.steps.iter().map(|s| s.spell.name.as_str()).collect()
    }

    #[test]
    fn no_requires_single_step() {
        let g = mk_grimoire(vec![mk_spell("a", &[])]);
        let plan = resolve(&g, &[req("a")]).unwrap();
        assert_eq!(names(&plan), vec!["a"]);
        assert!(plan.steps[0].is_seed);
    }

    #[test]
    fn single_requires_expansion() {
        let g = mk_grimoire(vec![mk_spell("a", &["b"]), mk_spell("b", &[])]);
        let plan = resolve(&g, &[req("a")]).unwrap();
        assert_eq!(names(&plan), vec!["b", "a"]);
        assert!(!plan.steps[0].is_seed); // b was pulled in
        assert!(plan.steps[1].is_seed); // a was the seed
    }

    #[test]
    fn two_level_requires() {
        let g = mk_grimoire(vec![
            mk_spell("a", &["b"]),
            mk_spell("b", &["c"]),
            mk_spell("c", &[]),
        ]);
        let plan = resolve(&g, &[req("a")]).unwrap();
        assert_eq!(names(&plan), vec!["c", "b", "a"]);
    }

    #[test]
    fn diamond_dependency() {
        // a → b, a → c, b → d, c → d
        let g = mk_grimoire(vec![
            mk_spell("a", &["b", "c"]),
            mk_spell("b", &["d"]),
            mk_spell("c", &["d"]),
            mk_spell("d", &[]),
        ]);
        let plan = resolve(&g, &[req("a")]).unwrap();
        let n = names(&plan);
        // d must come first; a must come last; b and c order between is implementation-detail.
        assert_eq!(n[0], "d");
        assert_eq!(n[3], "a");
        assert!(n[1..3].contains(&"b") && n[1..3].contains(&"c"));
    }

    #[test]
    fn cycle_detected() {
        let g = mk_grimoire(vec![mk_spell("a", &["b"]), mk_spell("b", &["a"])]);
        let err = resolve(&g, &[req("a")]).unwrap_err();
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn missing_spell_errors() {
        let g = mk_grimoire(vec![mk_spell("a", &["b"])]);
        let err = resolve(&g, &[req("a")]).unwrap_err();
        assert!(err.to_string().contains("not in the grimoire"));
    }

    #[test]
    fn constraint_merge_from_dependents() {
        // Two seeds both require b, but with different constraints.
        let g = mk_grimoire(vec![
            mk_spell("a", &["b >= 1.0"]),
            mk_spell("c", &["b < 2.0"]),
            mk_spell("b", &[]),
        ]);
        let plan = resolve(&g, &[req("a"), req("c")]).unwrap();
        let b_step = plan
            .steps
            .iter()
            .find(|s| s.spell.name == "b")
            .expect("b in plan");
        // Both >= 1.0 and < 2.0 must be present (AND).
        assert_eq!(b_step.requirement.constraints.len(), 2);
        assert!(b_step.requirement.satisfied_by("1.5"));
        assert!(!b_step.requirement.satisfied_by("0.9"));
        assert!(!b_step.requirement.satisfied_by("2.1"));
    }

    #[test]
    fn seed_constraint_propagates() {
        let g = mk_grimoire(vec![mk_spell("a", &[])]);
        let plan = resolve(&g, &[req("a >= 1.0")]).unwrap();
        assert_eq!(plan.steps[0].requirement.constraints.len(), 1);
    }

    #[test]
    fn deterministic_order_for_independent_seeds() {
        let g = mk_grimoire(vec![
            mk_spell("alpha", &[]),
            mk_spell("beta", &[]),
            mk_spell("gamma", &[]),
        ]);
        let plan1 = resolve(&g, &[req("gamma"), req("alpha"), req("beta")]).unwrap();
        let plan2 = resolve(&g, &[req("alpha"), req("beta"), req("gamma")]).unwrap();
        assert_eq!(names(&plan1), names(&plan2));
        assert_eq!(names(&plan1), vec!["alpha", "beta", "gamma"]);
    }
}
