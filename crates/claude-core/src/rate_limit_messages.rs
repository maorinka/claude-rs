//! Rate-limit user-facing messages — templates only.
//!
//! Port of TS `services/rateLimitMessages.ts`. The dynamic
//! computation side (fetching `used%`, formatting reset-time
//! durations, detecting overage state, ant-vs-external user
//! branching) is not yet ported — the API-layer rate-limit
//! handling in `api/retry.rs` + `api/client.rs` does not
//! currently surface user-facing messages.
//!
//! These helpers capture the exact template strings TS emits so
//! the caller wiring lands with verbatim parity when the
//! service is ported.

use crate::user_type::UserType;

/// "You've hit your {limit}" — emitted when the user has just
/// hit a rate-limit boundary. External-user variant.
///
/// Port of TS `rateLimitMessages.ts:343`.
pub fn limit_reached_external(limit: &str, reset_message: &str) -> String {
    format!("You've hit your {limit}{reset_message}")
}

/// Ant-user variant: appends the feedback-channel pointer and
/// `/reset-limits` escape hatch.
///
/// Port of TS `rateLimitMessages.ts:340`.
pub fn limit_reached_ant(limit: &str, reset_message: &str, feedback_channel: &str) -> String {
    format!(
        "You've hit your {limit}{reset_message}. If you have feedback about this limit, \
         post in {feedback_channel}. You can reset your limits with /reset-limits"
    )
}

/// Dispatch: pick the right limit-reached message for the
/// current `USER_TYPE`. Mirrors TS's `USER_TYPE === 'ant'`
/// branch at `rateLimitMessages.ts:339-343`.
pub fn limit_reached(
    user_type: UserType,
    limit: &str,
    reset_message: &str,
    feedback_channel: &str,
) -> String {
    match user_type {
        UserType::Ant => limit_reached_ant(limit, reset_message, feedback_channel),
        _ => limit_reached_external(limit, reset_message),
    }
}

/// "You've used {used}% of your {limit_name} · resets {reset_time}" —
/// early-warning variant with a percent used + reset time.
///
/// Port of TS `rateLimitMessages.ts:233`.
pub fn early_warning_with_reset(used: u32, limit_name: &str, reset_time: &str) -> String {
    format!("You've used {used}% of your {limit_name} · resets {reset_time}")
}

/// "You've used {used}% of your {limit_name}" — early-warning
/// without a known reset time.
///
/// Port of TS `rateLimitMessages.ts:238`.
pub fn early_warning_no_reset(used: u32, limit_name: &str) -> String {
    format!("You've used {used}% of your {limit_name}")
}

/// "Approaching {limit_name} · resets {reset_time}" — softer
/// warning when we only know the limit name + reset.
///
/// Port of TS `rateLimitMessages.ts:248`.
pub fn approaching_with_reset(limit_name: &str, reset_time: &str) -> String {
    format!("Approaching {limit_name} · resets {reset_time}")
}

/// "Approaching {limit_name}" — softest warning.
///
/// Port of TS `rateLimitMessages.ts:252`.
pub fn approaching_no_reset(limit_name: &str) -> String {
    format!("Approaching {limit_name}")
}

/// "You're now using extra usage" — emitted when the user
/// transitions into overage.
///
/// Port of TS `rateLimitMessages.ts:330`.
pub fn now_using_overage(reset_message: &str) -> String {
    format!("You're now using extra usage{reset_message}")
}

/// "You're close to your extra usage spending limit" — warn
/// before hitting overage limit.
///
/// Port of TS `rateLimitMessages.ts:~` (literal in
/// `rateLimitMessages.ts`).
pub const OVERAGE_CLOSE_TO_LIMIT: &str = "You're close to your extra usage spending limit";

/// "You're out of extra usage" — emitted when overage credits
/// are exhausted.
///
/// Port of TS `rateLimitMessages.ts:169`.
pub fn out_of_overage(overage_reset_message: &str) -> String {
    format!("You're out of extra usage{overage_reset_message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_reached_external_verbatim() {
        let s = limit_reached_external("5-hour limit", " (resets in 2h)");
        assert_eq!(s, "You've hit your 5-hour limit (resets in 2h)");
    }

    #[test]
    fn limit_reached_ant_includes_feedback_channel() {
        let s = limit_reached_ant("5-hour limit", "", "#cc-feedback");
        assert!(s.contains("You've hit your 5-hour limit"));
        assert!(s.contains("If you have feedback about this limit, post in #cc-feedback"));
        assert!(s.contains("You can reset your limits with /reset-limits"));
    }

    #[test]
    fn limit_reached_dispatch_by_user_type() {
        let ext = limit_reached(UserType::External, "L", "", "#chan");
        let ant = limit_reached(UserType::Ant, "L", "", "#chan");
        assert!(!ext.contains("feedback"));
        assert!(ant.contains("feedback"));
    }

    #[test]
    fn early_warning_shapes() {
        assert_eq!(
            early_warning_with_reset(80, "5-hour limit", "2pm"),
            "You've used 80% of your 5-hour limit · resets 2pm"
        );
        assert_eq!(
            early_warning_no_reset(80, "5-hour limit"),
            "You've used 80% of your 5-hour limit"
        );
    }

    #[test]
    fn approaching_shapes() {
        assert_eq!(
            approaching_with_reset("5-hour limit", "2pm"),
            "Approaching 5-hour limit · resets 2pm"
        );
        assert_eq!(
            approaching_no_reset("5-hour limit"),
            "Approaching 5-hour limit"
        );
    }

    #[test]
    fn overage_shapes() {
        assert_eq!(now_using_overage(""), "You're now using extra usage");
        assert_eq!(out_of_overage(""), "You're out of extra usage");
        assert_eq!(
            OVERAGE_CLOSE_TO_LIMIT,
            "You're close to your extra usage spending limit"
        );
    }
}
