//! Bash output-length limits for truncation decisions.
//!
//! Port of TS `utils/shell/outputLimits.ts:1-14`.
//!
//! Controls how much of a Bash tool's stdout/stderr Claude ever sees. The
//! cap protects the token budget from a runaway command dumping MB of
//! logs into context. The default is the "everyday" ceiling; the upper
//! limit is an absolute hard stop that even an env-var override can't
//! exceed.

use crate::env_validation::validate_bounded_int_env_var;

/// Absolute ceiling — even `BASH_MAX_OUTPUT_LENGTH=1000000` is capped
/// at this many characters. Matches TS constant verbatim.
pub const BASH_MAX_OUTPUT_UPPER_LIMIT: u64 = 150_000;

/// Default when the env var is unset / empty / malformed. Matches TS
/// constant verbatim.
pub const BASH_MAX_OUTPUT_DEFAULT: u64 = 30_000;

/// Env var name the override is read from. Kept as a `const` so the test
/// in this module and call sites in `claude-tools` can share one source
/// of truth.
pub const BASH_MAX_OUTPUT_ENV: &str = "BASH_MAX_OUTPUT_LENGTH";

/// Resolve the effective Bash-output cap, consulting `BASH_MAX_OUTPUT_LENGTH`.
/// Delegates bounds/fallback handling to `validate_bounded_int_env_var`
/// (same helper TS's `validateBoundedIntEnvVar` maps onto), so the
/// "capped / invalid / valid" status stays consistent with other env-gated
/// limits in the crate.
pub fn get_max_output_length() -> u64 {
    let raw = std::env::var(BASH_MAX_OUTPUT_ENV).ok();
    validate_bounded_int_env_var(
        BASH_MAX_OUTPUT_ENV,
        raw.as_deref(),
        BASH_MAX_OUTPUT_DEFAULT,
        BASH_MAX_OUTPUT_UPPER_LIMIT,
    )
    .effective
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn constants_match_ts() {
        // Pins the two ceilings — TS has the same literals at
        // outputLimits.ts:3-4. Changing either would silently shift
        // truncation behaviour across the fleet.
        assert_eq!(BASH_MAX_OUTPUT_UPPER_LIMIT, 150_000);
        assert_eq!(BASH_MAX_OUTPUT_DEFAULT, 30_000);
    }

    #[test]
    fn default_when_env_unset() {
        let _g = lock_env();
        std::env::remove_var(BASH_MAX_OUTPUT_ENV);
        assert_eq!(get_max_output_length(), BASH_MAX_OUTPUT_DEFAULT);
    }

    #[test]
    fn default_when_env_empty() {
        let _g = lock_env();
        std::env::set_var(BASH_MAX_OUTPUT_ENV, "");
        assert_eq!(get_max_output_length(), BASH_MAX_OUTPUT_DEFAULT);
        std::env::remove_var(BASH_MAX_OUTPUT_ENV);
    }

    #[test]
    fn reads_valid_override() {
        let _g = lock_env();
        std::env::set_var(BASH_MAX_OUTPUT_ENV, "50000");
        assert_eq!(get_max_output_length(), 50_000);
        std::env::remove_var(BASH_MAX_OUTPUT_ENV);
    }

    #[test]
    fn caps_above_upper_limit() {
        let _g = lock_env();
        // 1M requested → clamped to the 150k ceiling.
        std::env::set_var(BASH_MAX_OUTPUT_ENV, "1000000");
        assert_eq!(get_max_output_length(), BASH_MAX_OUTPUT_UPPER_LIMIT);
        std::env::remove_var(BASH_MAX_OUTPUT_ENV);
    }

    #[test]
    fn falls_back_on_non_numeric() {
        let _g = lock_env();
        std::env::set_var(BASH_MAX_OUTPUT_ENV, "not-a-number");
        assert_eq!(get_max_output_length(), BASH_MAX_OUTPUT_DEFAULT);
        std::env::remove_var(BASH_MAX_OUTPUT_ENV);
    }

    #[test]
    fn falls_back_on_non_positive() {
        let _g = lock_env();
        std::env::set_var(BASH_MAX_OUTPUT_ENV, "0");
        assert_eq!(get_max_output_length(), BASH_MAX_OUTPUT_DEFAULT);
        std::env::set_var(BASH_MAX_OUTPUT_ENV, "-500");
        assert_eq!(get_max_output_length(), BASH_MAX_OUTPUT_DEFAULT);
        std::env::remove_var(BASH_MAX_OUTPUT_ENV);
    }

    #[test]
    fn accepts_value_at_upper_limit_exactly() {
        let _g = lock_env();
        std::env::set_var(BASH_MAX_OUTPUT_ENV, "150000");
        assert_eq!(get_max_output_length(), BASH_MAX_OUTPUT_UPPER_LIMIT);
        std::env::remove_var(BASH_MAX_OUTPUT_ENV);
    }
}
