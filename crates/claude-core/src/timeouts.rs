//! Bash execution timeout envelope.
//!
//! Port of TS `src/utils/timeouts.ts`. Two env vars drive the
//! shell-exec timeouts:
//! - `BASH_DEFAULT_TIMEOUT_MS` → default per-call timeout
//!   (floor: `DEFAULT_TIMEOUT_MS` = 120_000 ms).
//! - `BASH_MAX_TIMEOUT_MS` → ceiling (floor: `MAX_TIMEOUT_MS` =
//!   600_000 ms). The ceiling is also clamped to never be below
//!   the default — callers that bump `BASH_DEFAULT_TIMEOUT_MS`
//!   past the static max still get a sensible window.

use std::collections::HashMap;

/// Default bash operation timeout (2 minutes).
pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
/// Ceiling for any explicit per-call timeout (10 minutes).
pub const MAX_TIMEOUT_MS: u64 = 600_000;

/// Borrow an env lookup; `None` falls back to `std::env::var`.
pub type EnvLookup<'a> = Option<&'a HashMap<String, String>>;

fn env_get(env: EnvLookup, key: &str) -> Option<String> {
    match env {
        Some(map) => map.get(key).cloned(),
        None => std::env::var(key).ok(),
    }
}

fn parse_positive_ms(s: &str) -> Option<u64> {
    let n: i64 = s.parse().ok()?;
    if n > 0 {
        Some(n as u64)
    } else {
        None
    }
}

/// Resolve the bash default timeout. Reads `BASH_DEFAULT_TIMEOUT_MS`
/// if set to a positive integer, else returns `DEFAULT_TIMEOUT_MS`.
pub fn get_default_bash_timeout_ms(env: EnvLookup) -> u64 {
    env_get(env, "BASH_DEFAULT_TIMEOUT_MS")
        .as_deref()
        .and_then(parse_positive_ms)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
}

/// Resolve the bash maximum timeout. Reads `BASH_MAX_TIMEOUT_MS` if
/// set to a positive integer, else `MAX_TIMEOUT_MS`; the result is
/// always ≥ `get_default_bash_timeout_ms(env)` so callers that bump
/// the default above the cap still get a window that accommodates
/// it.
pub fn get_max_bash_timeout_ms(env: EnvLookup) -> u64 {
    let default_ms = get_default_bash_timeout_ms(env);
    let configured = env_get(env, "BASH_MAX_TIMEOUT_MS")
        .as_deref()
        .and_then(parse_positive_ms)
        .unwrap_or(MAX_TIMEOUT_MS);
    configured.max(default_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn default_when_unset() {
        let e = env(&[]);
        assert_eq!(get_default_bash_timeout_ms(Some(&e)), DEFAULT_TIMEOUT_MS);
    }

    #[test]
    fn default_env_override_applies() {
        let e = env(&[("BASH_DEFAULT_TIMEOUT_MS", "30000")]);
        assert_eq!(get_default_bash_timeout_ms(Some(&e)), 30_000);
    }

    #[test]
    fn default_non_positive_falls_back() {
        for v in ["0", "-5", "not-a-number"] {
            let e = env(&[("BASH_DEFAULT_TIMEOUT_MS", v)]);
            assert_eq!(
                get_default_bash_timeout_ms(Some(&e)),
                DEFAULT_TIMEOUT_MS
            );
        }
    }

    #[test]
    fn max_when_unset() {
        let e = env(&[]);
        assert_eq!(get_max_bash_timeout_ms(Some(&e)), MAX_TIMEOUT_MS);
    }

    #[test]
    fn max_env_override_applies() {
        let e = env(&[("BASH_MAX_TIMEOUT_MS", "90000")]);
        // override < default (120_000) → clamped to default.
        assert_eq!(
            get_max_bash_timeout_ms(Some(&e)),
            DEFAULT_TIMEOUT_MS
        );
    }

    #[test]
    fn max_stays_at_least_default() {
        let e = env(&[
            ("BASH_DEFAULT_TIMEOUT_MS", "200000"),
            // Unset BASH_MAX_TIMEOUT_MS → MAX_TIMEOUT_MS (600_000)
        ]);
        assert!(get_max_bash_timeout_ms(Some(&e)) >= 200_000);
    }

    #[test]
    fn max_respects_above_static() {
        let e = env(&[("BASH_MAX_TIMEOUT_MS", "1200000")]);
        assert_eq!(get_max_bash_timeout_ms(Some(&e)), 1_200_000);
    }
}
