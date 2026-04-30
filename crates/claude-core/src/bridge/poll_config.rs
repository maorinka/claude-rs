//! Bridge poll interval defaults and validation.
//!
//! Ports the validation contract from TS `src/bridge/pollConfig.ts` and
//! `pollConfigDefaults.ts`, including TS's default-value fallback semantics
//! when GrowthBook has no value or serves malformed JSON.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollIntervalConfig {
    pub poll_interval_ms_not_at_capacity: u64,
    pub poll_interval_ms_at_capacity: u64,
    pub non_exclusive_heartbeat_interval_ms: u64,
    pub multisession_poll_interval_ms_not_at_capacity: u64,
    pub multisession_poll_interval_ms_partial_capacity: u64,
    pub multisession_poll_interval_ms_at_capacity: u64,
    pub reclaim_older_than_ms: u64,
    pub session_keepalive_interval_v2_ms: u64,
}

pub const DEFAULT_POLL_CONFIG: PollIntervalConfig = PollIntervalConfig {
    poll_interval_ms_not_at_capacity: 2_000,
    poll_interval_ms_at_capacity: 600_000,
    non_exclusive_heartbeat_interval_ms: 0,
    multisession_poll_interval_ms_not_at_capacity: 2_000,
    multisession_poll_interval_ms_partial_capacity: 2_000,
    multisession_poll_interval_ms_at_capacity: 600_000,
    reclaim_older_than_ms: 5_000,
    session_keepalive_interval_v2_ms: 120_000,
};

pub fn get_poll_interval_config() -> PollIntervalConfig {
    let raw = crate::growthbook::get_feature_value_cached_may_be_stale_json(
        "tengu_bridge_poll_interval_config",
        serde_json::to_value(&DEFAULT_POLL_CONFIG).unwrap_or(Value::Null),
    );
    parse_poll_interval_config(&raw)
}

pub fn parse_poll_interval_config(raw: &Value) -> PollIntervalConfig {
    let Some(obj) = raw.as_object() else {
        return DEFAULT_POLL_CONFIG;
    };
    let Some(poll_interval_ms_not_at_capacity) =
        integer_at_least(obj.get("poll_interval_ms_not_at_capacity"), 100)
    else {
        return DEFAULT_POLL_CONFIG;
    };
    let Some(poll_interval_ms_at_capacity) =
        zero_or_at_least_100(obj.get("poll_interval_ms_at_capacity"))
    else {
        return DEFAULT_POLL_CONFIG;
    };
    let Some(non_exclusive_heartbeat_interval_ms) =
        optional_integer_at_least(obj.get("non_exclusive_heartbeat_interval_ms"), 0, 0)
    else {
        return DEFAULT_POLL_CONFIG;
    };
    let Some(multisession_poll_interval_ms_not_at_capacity) = optional_integer_at_least(
        obj.get("multisession_poll_interval_ms_not_at_capacity"),
        100,
        DEFAULT_POLL_CONFIG.multisession_poll_interval_ms_not_at_capacity,
    ) else {
        return DEFAULT_POLL_CONFIG;
    };
    let Some(multisession_poll_interval_ms_partial_capacity) = optional_integer_at_least(
        obj.get("multisession_poll_interval_ms_partial_capacity"),
        100,
        DEFAULT_POLL_CONFIG.multisession_poll_interval_ms_partial_capacity,
    ) else {
        return DEFAULT_POLL_CONFIG;
    };
    let Some(multisession_poll_interval_ms_at_capacity) = optional_zero_or_at_least_100(
        obj.get("multisession_poll_interval_ms_at_capacity"),
        DEFAULT_POLL_CONFIG.multisession_poll_interval_ms_at_capacity,
    ) else {
        return DEFAULT_POLL_CONFIG;
    };
    let Some(reclaim_older_than_ms) =
        optional_integer_at_least(obj.get("reclaim_older_than_ms"), 1, 5_000)
    else {
        return DEFAULT_POLL_CONFIG;
    };
    let Some(session_keepalive_interval_v2_ms) =
        optional_integer_at_least(obj.get("session_keepalive_interval_v2_ms"), 0, 120_000)
    else {
        return DEFAULT_POLL_CONFIG;
    };

    if non_exclusive_heartbeat_interval_ms == 0 && poll_interval_ms_at_capacity == 0 {
        return DEFAULT_POLL_CONFIG;
    }
    if non_exclusive_heartbeat_interval_ms == 0 && multisession_poll_interval_ms_at_capacity == 0 {
        return DEFAULT_POLL_CONFIG;
    }

    PollIntervalConfig {
        poll_interval_ms_not_at_capacity,
        poll_interval_ms_at_capacity,
        non_exclusive_heartbeat_interval_ms,
        multisession_poll_interval_ms_not_at_capacity,
        multisession_poll_interval_ms_partial_capacity,
        multisession_poll_interval_ms_at_capacity,
        reclaim_older_than_ms,
        session_keepalive_interval_v2_ms,
    }
}

fn integer_at_least(value: Option<&Value>, min: u64) -> Option<u64> {
    let value = value?;
    let int = value.as_u64()?;
    (int >= min).then_some(int)
}

fn optional_integer_at_least(value: Option<&Value>, min: u64, default: u64) -> Option<u64> {
    match value {
        Some(value) => integer_at_least(Some(value), min),
        None => Some(default),
    }
}

fn zero_or_at_least_100(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    let int = value.as_u64()?;
    (int == 0 || int >= 100).then_some(int)
}

fn optional_zero_or_at_least_100(value: Option<&Value>, default: u64) -> Option<u64> {
    match value {
        Some(value) => zero_or_at_least_100(Some(value)),
        None => Some(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_valid() -> Value {
        json!({
            "poll_interval_ms_not_at_capacity": 2000,
            "poll_interval_ms_at_capacity": 600000,
        })
    }

    #[test]
    fn defaults_match_ts_poll_config_defaults() {
        assert_eq!(DEFAULT_POLL_CONFIG.poll_interval_ms_not_at_capacity, 2_000);
        assert_eq!(DEFAULT_POLL_CONFIG.poll_interval_ms_at_capacity, 600_000);
        assert_eq!(DEFAULT_POLL_CONFIG.non_exclusive_heartbeat_interval_ms, 0);
        assert_eq!(
            DEFAULT_POLL_CONFIG.session_keepalive_interval_v2_ms,
            120_000
        );
    }

    #[test]
    fn get_poll_interval_config_reads_growthbook_feature_value() {
        let _guard = crate::constants::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var(
            "CLAUDE_CODE_GROWTHBOOK_FEATURES",
            r#"{"tengu_bridge_poll_interval_config":{"poll_interval_ms_not_at_capacity":2500,"poll_interval_ms_at_capacity":600000,"multisession_poll_interval_ms_partial_capacity":3000}}"#,
        );
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_TENGU_BRIDGE_POLL_INTERVAL_CONFIG");

        let parsed = get_poll_interval_config();

        assert_eq!(parsed.poll_interval_ms_not_at_capacity, 2_500);
        assert_eq!(parsed.multisession_poll_interval_ms_partial_capacity, 3_000);
        assert_eq!(parsed.reclaim_older_than_ms, 5_000);
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_FEATURES");
    }

    #[test]
    fn valid_partial_config_gets_ts_defaults_for_new_fields() {
        let parsed = parse_poll_interval_config(&minimal_valid());
        assert_eq!(parsed.poll_interval_ms_not_at_capacity, 2_000);
        assert_eq!(
            parsed.multisession_poll_interval_ms_at_capacity,
            DEFAULT_POLL_CONFIG.multisession_poll_interval_ms_at_capacity
        );
        assert_eq!(parsed.reclaim_older_than_ms, 5_000);
    }

    #[test]
    fn rejects_bad_intervals_by_falling_back_to_defaults() {
        let bad = json!({
            "poll_interval_ms_not_at_capacity": 99,
            "poll_interval_ms_at_capacity": 600000,
        });
        assert_eq!(parse_poll_interval_config(&bad), DEFAULT_POLL_CONFIG);

        let bad_at_cap = json!({
            "poll_interval_ms_not_at_capacity": 2000,
            "poll_interval_ms_at_capacity": 10,
        });
        assert_eq!(parse_poll_interval_config(&bad_at_cap), DEFAULT_POLL_CONFIG);
    }

    #[test]
    fn requires_at_capacity_liveness_for_single_and_multisession() {
        let no_liveness = json!({
            "poll_interval_ms_not_at_capacity": 2000,
            "poll_interval_ms_at_capacity": 0,
            "multisession_poll_interval_ms_at_capacity": 0,
            "non_exclusive_heartbeat_interval_ms": 0,
        });
        assert_eq!(
            parse_poll_interval_config(&no_liveness),
            DEFAULT_POLL_CONFIG
        );

        let heartbeat = json!({
            "poll_interval_ms_not_at_capacity": 2000,
            "poll_interval_ms_at_capacity": 0,
            "multisession_poll_interval_ms_at_capacity": 0,
            "non_exclusive_heartbeat_interval_ms": 60_000,
        });
        let parsed = parse_poll_interval_config(&heartbeat);
        assert_eq!(parsed.non_exclusive_heartbeat_interval_ms, 60_000);
        assert_eq!(parsed.poll_interval_ms_at_capacity, 0);
    }
}
