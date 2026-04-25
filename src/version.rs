use std::cmp::Ordering;

/// Compare two version strings.
///
/// Splits on `.` and `-`. Numeric components compare numerically; non-numeric
/// compare lexically. A trailing pre-release component (after `-`) sorts
/// lower than the same version without one (e.g. `1.0-rc1` < `1.0`).
pub fn compare(a: &str, b: &str) -> Ordering {
    let (a_main, a_pre) = split_pre(a);
    let (b_main, b_pre) = split_pre(b);

    match cmp_components(a_main, b_main) {
        Ordering::Equal => match (a_pre, b_pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater, // 1.0 > 1.0-rc1
            (Some(_), None) => Ordering::Less,
            (Some(x), Some(y)) => cmp_components(x, y),
        },
        other => other,
    }
}

fn split_pre(v: &str) -> (&str, Option<&str>) {
    match v.split_once('-') {
        Some((main, pre)) => (main, Some(pre)),
        None => (v, None),
    }
}

fn cmp_components(a: &str, b: &str) -> Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => match cmp_one(x, y) {
                Ordering::Equal => continue,
                other => return other,
            },
        }
    }
}

fn cmp_one(a: &str, b: &str) -> Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn semver_basic() {
        assert_eq!(compare("1.95.0", "1.95.0"), Equal);
        assert_eq!(compare("1.95.0", "1.96.0"), Less);
        assert_eq!(compare("2.0.0", "1.99.99"), Greater);
    }

    #[test]
    fn unequal_lengths() {
        assert_eq!(compare("1.95", "1.95.0"), Less);
        assert_eq!(compare("1.95.0", "1.95"), Greater);
    }

    #[test]
    fn dotted_dates() {
        assert_eq!(compare("2025.1.2.12", "2025.1.2.13"), Less);
        assert_eq!(compare("2025.10.0", "2025.2.0"), Greater);
    }

    #[test]
    fn pre_release_lower() {
        assert_eq!(compare("1.0.0-rc1", "1.0.0"), Less);
        assert_eq!(compare("1.0.0", "1.0.0-rc1"), Greater);
        assert_eq!(compare("1.0.0-rc1", "1.0.0-rc2"), Less);
    }

    #[test]
    fn numeric_beats_lexical() {
        assert_eq!(compare("1.10.0", "1.9.0"), Greater); // numeric, not lex
    }
}
