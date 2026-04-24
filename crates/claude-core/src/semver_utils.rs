//! Semver comparison helpers matching TS `utils/semver.ts:1-60`.
//!
//! TS uses `Bun.semver.order` when available, falling back to the npm
//! `semver` package with `{ loose: true }`. Rust uses the `semver` crate
//! directly — `{ loose: true }` means "coerce" in the npm sense, which
//! the Rust `semver::Version::parse` does NOT do out of the box. This
//! module emulates coercion on the way in: extract the first `x.y.z[...]`
//! run from the input and parse that, matching what `semver.coerce`
//! does in npm.
//!
//! Named `semver_utils` rather than `semver` to avoid shadowing the
//! crate name at call sites.

use semver::{Version, VersionReq};

/// Extract the first `x.y.z` run from `input` and parse it as a Version.
/// Used to emulate npm `semver` loose mode, which TS relies on.
///
/// `coerce("v1.2.3-beta.4+build")` → `1.2.3-beta.4+build`
/// `coerce("1.2")` → `1.2.0`
/// `coerce("abc")` → `None`
fn coerce(input: &str) -> Option<Version> {
    // Direct parse first — fastest path for well-formed input like
    // `"1.2.3"`. Falls back to regex scan on the npm-ish inputs
    // (leading `v`, missing patch, embedded in a label, etc.).
    if let Ok(v) = Version::parse(input) {
        return Some(v);
    }

    let re = regex::Regex::new(r"(\d+)(?:\.(\d+))?(?:\.(\d+))?([.\-+][0-9A-Za-z.\-+]*)?").ok()?;
    let cap = re.captures(input)?;
    let major: u64 = cap.get(1)?.as_str().parse().ok()?;
    let minor: u64 = cap
        .get(2)
        .map(|m| m.as_str().parse().unwrap_or(0))
        .unwrap_or(0);
    let patch: u64 = cap
        .get(3)
        .map(|m| m.as_str().parse().unwrap_or(0))
        .unwrap_or(0);
    let suffix = cap.get(4).map(|m| m.as_str()).unwrap_or("");
    let canonical = format!("{major}.{minor}.{patch}{suffix}");
    Version::parse(&canonical).ok()
}

/// Matches TS `gt(a, b)`.
pub fn gt(a: &str, b: &str) -> bool {
    order(a, b).is_some_and(|o| o == std::cmp::Ordering::Greater)
}

/// Matches TS `gte(a, b)`.
pub fn gte(a: &str, b: &str) -> bool {
    order(a, b).is_some_and(|o| o != std::cmp::Ordering::Less)
}

/// Matches TS `lt(a, b)`.
pub fn lt(a: &str, b: &str) -> bool {
    order(a, b).is_some_and(|o| o == std::cmp::Ordering::Less)
}

/// Matches TS `lte(a, b)`.
pub fn lte(a: &str, b: &str) -> bool {
    order(a, b).is_some_and(|o| o != std::cmp::Ordering::Greater)
}

/// Matches TS `satisfies(version, range)` under `{ loose: true }`.
/// Returns `false` for any parse failure — TS throws in strict mode
/// but the npm loose parser swallows malformed input as "doesn't
/// match". Callers that need to distinguish malformed-input from
/// valid-no-match should parse up-front.
pub fn satisfies(version: &str, range: &str) -> bool {
    let Some(v) = coerce(version) else {
        return false;
    };
    // npm/Bun accept space-separated AND-ranges (`">=1.0.0 <2.0.0"`); the
    // Rust `semver` crate requires commas. Normalise here so callers that
    // ported range literals from TS keep working.
    let normalised = normalise_range_separators(range);
    let Ok(r) = VersionReq::parse(&normalised) else {
        return false;
    };
    r.matches(&v)
}

fn normalise_range_separators(range: &str) -> String {
    // Collapse runs of whitespace between tokens into `,`. Leading
    // space on operators like `>= 1.0.0` is fine — we only split on
    // whitespace that separates full comparators.
    range.split_whitespace().collect::<Vec<_>>().join(",")
}

/// Matches TS `order(a, b)` returning `-1 | 0 | 1`. Rust returns an
/// `Option<Ordering>` so callers can tell "unparseable" from "equal"
/// — TS ignores that distinction because Bun throws on invalid input.
pub fn order(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let va = coerce(a)?;
    let vb = coerce(b)?;
    Some(va.cmp(&vb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gt_basic() {
        assert!(gt("2.0.0", "1.9.9"));
        assert!(!gt("1.0.0", "1.0.0"));
        assert!(!gt("1.0.0", "2.0.0"));
    }

    #[test]
    fn gte_basic() {
        assert!(gte("2.0.0", "1.9.9"));
        assert!(gte("1.0.0", "1.0.0"));
        assert!(!gte("1.0.0", "2.0.0"));
    }

    #[test]
    fn lt_and_lte() {
        assert!(lt("1.0.0", "1.0.1"));
        assert!(!lt("1.0.0", "1.0.0"));
        assert!(lte("1.0.0", "1.0.0"));
        assert!(lte("1.0.0", "2.0.0"));
        assert!(!lte("2.0.0", "1.0.0"));
    }

    #[test]
    fn order_returns_orderings() {
        use std::cmp::Ordering;
        assert_eq!(order("1.0.0", "2.0.0"), Some(Ordering::Less));
        assert_eq!(order("1.0.0", "1.0.0"), Some(Ordering::Equal));
        assert_eq!(order("2.0.0", "1.0.0"), Some(Ordering::Greater));
    }

    #[test]
    fn prerelease_ordering() {
        // semver.org spec: prerelease < release.
        assert!(lt("1.0.0-alpha", "1.0.0"));
        assert!(lt("1.0.0-alpha", "1.0.0-beta"));
        assert!(gt("1.0.0-rc.2", "1.0.0-rc.1"));
    }

    #[test]
    fn satisfies_basic_ranges() {
        assert!(satisfies("1.2.3", "^1.0.0"));
        assert!(!satisfies("2.0.0", "^1.0.0"));
        assert!(satisfies("1.2.3", "~1.2.0"));
        assert!(!satisfies("1.3.0", "~1.2.0"));
        assert!(satisfies("1.2.3", ">=1.0.0 <2.0.0"));
    }

    #[test]
    fn coerce_accepts_leading_v() {
        // npm semver coerce strips a leading `v` — Bun and Node paths
        // both end up calling through it for `v1.2.3`-style strings.
        assert!(gt("v2.0.0", "v1.9.9"));
    }

    #[test]
    fn coerce_accepts_partial_version() {
        // `"1.2"` coerces to `1.2.0`. Matches npm semver loose.
        assert!(gte("1.2", "1.2.0"));
        assert!(lt("1", "2"));
    }

    #[test]
    fn coerce_accepts_version_with_label() {
        // Real Electron / macOS / Windows version strings tend to have
        // suffixes. Ensure the coerce extraction keeps the
        // prerelease+build tail so ordering is correct.
        assert!(gt("1.2.3-beta.4", "1.2.3-beta.3"));
    }

    #[test]
    fn malformed_input_returns_false_not_panic() {
        assert!(!gt("not-a-version", "1.0.0"));
        assert!(!gte("", "1.0.0"));
        assert!(!lt("1.0.0", "~~~"));
        assert!(order("abc", "def").is_none());
    }

    #[test]
    fn satisfies_rejects_unparseable() {
        // npm loose would swallow these into false; Rust port matches.
        assert!(!satisfies("abc", "^1.0.0"));
        assert!(!satisfies("1.0.0", "not a range"));
    }
}
