//! Bounded-integer env var validation.
//!
//! Port of TS `src/utils/envValidation.ts`. Lets callers parse
//! numeric env vars (retry counts, timeouts, rate caps, etc.) with
//! three-way outcome reporting so misconfigurations surface as
//! warnings instead of silent defaults.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvVarStatus {
    /// Value parsed and fell within range.
    Valid,
    /// Value parsed but exceeded the upper limit; reduced to the cap.
    Capped,
    /// Value failed to parse or was non-positive; default used.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVarValidationResult {
    pub effective: u64,
    pub status: EnvVarStatus,
    /// Human-readable explanation; `None` on the success path.
    pub message: Option<String>,
}

/// Parse an env var into a positive integer bounded by
/// `upper_limit`. Falls back to `default_value` when the var is
/// missing, empty, non-numeric, or non-positive.
///
/// Matches TS semantics: `None` / empty → Valid default; non-
/// numeric or ≤ 0 → Invalid with a message; above the cap → Capped
/// to `upper_limit`; otherwise → Valid with the parsed value.
pub fn validate_bounded_int_env_var(
    _name: &str,
    value: Option<&str>,
    default_value: u64,
    upper_limit: u64,
) -> EnvVarValidationResult {
    let Some(value) = value else {
        return EnvVarValidationResult {
            effective: default_value,
            status: EnvVarStatus::Valid,
            message: None,
        };
    };
    if value.is_empty() {
        return EnvVarValidationResult {
            effective: default_value,
            status: EnvVarStatus::Valid,
            message: None,
        };
    }

    let parsed: Option<i64> = value.parse().ok();
    match parsed {
        None => EnvVarValidationResult {
            effective: default_value,
            status: EnvVarStatus::Invalid,
            message: Some(format!(
                "Invalid value \"{value}\" (using default: {default_value})"
            )),
        },
        Some(n) if n <= 0 => EnvVarValidationResult {
            effective: default_value,
            status: EnvVarStatus::Invalid,
            message: Some(format!(
                "Invalid value \"{value}\" (using default: {default_value})"
            )),
        },
        Some(n) => {
            let n = n as u64;
            if n > upper_limit {
                EnvVarValidationResult {
                    effective: upper_limit,
                    status: EnvVarStatus::Capped,
                    message: Some(format!(
                        "Capped from {n} to {upper_limit}"
                    )),
                }
            } else {
                EnvVarValidationResult {
                    effective: n,
                    status: EnvVarStatus::Valid,
                    message: None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_returns_default_valid() {
        let r = validate_bounded_int_env_var("X", None, 10, 100);
        assert_eq!(r.effective, 10);
        assert_eq!(r.status, EnvVarStatus::Valid);
        assert!(r.message.is_none());
    }

    #[test]
    fn empty_returns_default_valid() {
        let r = validate_bounded_int_env_var("X", Some(""), 10, 100);
        assert_eq!(r.effective, 10);
        assert_eq!(r.status, EnvVarStatus::Valid);
    }

    #[test]
    fn in_range_returns_parsed_valid() {
        let r = validate_bounded_int_env_var("X", Some("42"), 10, 100);
        assert_eq!(r.effective, 42);
        assert_eq!(r.status, EnvVarStatus::Valid);
    }

    #[test]
    fn above_limit_caps() {
        let r = validate_bounded_int_env_var("X", Some("500"), 10, 100);
        assert_eq!(r.effective, 100);
        assert_eq!(r.status, EnvVarStatus::Capped);
        assert_eq!(r.message.as_deref(), Some("Capped from 500 to 100"));
    }

    #[test]
    fn non_numeric_is_invalid() {
        let r = validate_bounded_int_env_var("X", Some("not a number"), 10, 100);
        assert_eq!(r.effective, 10);
        assert_eq!(r.status, EnvVarStatus::Invalid);
        assert!(r.message.as_deref().unwrap().contains("not a number"));
    }

    #[test]
    fn zero_is_invalid() {
        let r = validate_bounded_int_env_var("X", Some("0"), 10, 100);
        assert_eq!(r.effective, 10);
        assert_eq!(r.status, EnvVarStatus::Invalid);
    }

    #[test]
    fn negative_is_invalid() {
        let r = validate_bounded_int_env_var("X", Some("-5"), 10, 100);
        assert_eq!(r.effective, 10);
        assert_eq!(r.status, EnvVarStatus::Invalid);
    }

    #[test]
    fn exact_upper_limit_is_valid_not_capped() {
        let r = validate_bounded_int_env_var("X", Some("100"), 10, 100);
        assert_eq!(r.effective, 100);
        assert_eq!(r.status, EnvVarStatus::Valid);
    }
}
