//! Minimal GrowthBook feature-value bridge.
//!
//! TS resolves `getFeatureValue_CACHED_MAY_BE_STALE(name, defaultValue)` from
//! the initialized GrowthBook client. The Rust port does not yet have the full
//! client/runtime refresh path, so callers can inject the same cached feature
//! view through environment variables while preserving TS's default-value
//! semantics.

use serde_json::Value;

/// Return a cached boolean feature value, falling back to `default_value`.
///
/// Supported injection points:
/// - `CLAUDE_CODE_GROWTHBOOK_FEATURES`: JSON object, for example
///   `{"tengu_quartz_lantern": true}`.
/// - `CLAUDE_CODE_GROWTHBOOK_<FEATURE>`: per-feature override where
///   non-alphanumeric characters in `FEATURE` become `_` and letters uppercase.
pub fn get_feature_value_cached_may_be_stale_bool(name: &str, default_value: bool) -> bool {
    let env_name = format!("CLAUDE_CODE_GROWTHBOOK_{}", env_feature_name(name));
    if let Ok(raw) = std::env::var(&env_name) {
        if let Some(value) = parse_bool(&raw) {
            return value;
        }
    }

    if let Ok(raw) = std::env::var("CLAUDE_CODE_GROWTHBOOK_FEATURES") {
        if let Ok(Value::Object(features)) = serde_json::from_str::<Value>(&raw) {
            if let Some(value) = features.get(name).and_then(Value::as_bool) {
                return value;
            }
        }
    }

    default_value
}

/// Return a cached JSON feature value, falling back to `default_value`.
///
/// This mirrors the same env-backed injection path as the boolean helper so
/// typed feature-specific modules can preserve TS default-value semantics.
pub fn get_feature_value_cached_may_be_stale_json(name: &str, default_value: Value) -> Value {
    let env_name = format!("CLAUDE_CODE_GROWTHBOOK_{}", env_feature_name(name));
    if let Ok(raw) = std::env::var(&env_name) {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            return value;
        }
    }

    if let Ok(raw) = std::env::var("CLAUDE_CODE_GROWTHBOOK_FEATURES") {
        if let Ok(Value::Object(features)) = serde_json::from_str::<Value>(&raw) {
            if let Some(value) = features.get(name) {
                return value.clone();
            }
        }
    }

    default_value
}

fn env_feature_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::ENV_LOCK;

    #[test]
    fn defaults_when_missing() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_FEATURES");
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_TENGU_QUARTZ_LANTERN");
        assert!(get_feature_value_cached_may_be_stale_bool(
            "tengu_quartz_lantern",
            true
        ));
        assert!(!get_feature_value_cached_may_be_stale_bool(
            "tengu_quartz_lantern",
            false
        ));
    }

    #[test]
    fn reads_json_feature_map() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(
            "CLAUDE_CODE_GROWTHBOOK_FEATURES",
            r#"{"tengu_quartz_lantern":true}"#,
        );
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_TENGU_QUARTZ_LANTERN");
        assert!(get_feature_value_cached_may_be_stale_bool(
            "tengu_quartz_lantern",
            false
        ));
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_FEATURES");
    }

    #[test]
    fn per_feature_env_overrides_json() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(
            "CLAUDE_CODE_GROWTHBOOK_FEATURES",
            r#"{"tengu_quartz_lantern":true}"#,
        );
        std::env::set_var("CLAUDE_CODE_GROWTHBOOK_TENGU_QUARTZ_LANTERN", "false");
        assert!(!get_feature_value_cached_may_be_stale_bool(
            "tengu_quartz_lantern",
            true
        ));
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_FEATURES");
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_TENGU_QUARTZ_LANTERN");
    }

    #[test]
    fn reads_json_feature_value() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(
            "CLAUDE_CODE_GROWTHBOOK_FEATURES",
            r#"{"tengu_poll":{"interval":2000}}"#,
        );
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_TENGU_POLL");
        assert_eq!(
            get_feature_value_cached_may_be_stale_json(
                "tengu_poll",
                serde_json::json!({"interval": 100})
            ),
            serde_json::json!({"interval": 2000})
        );
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_FEATURES");
    }

    #[test]
    fn per_feature_json_env_overrides_map() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(
            "CLAUDE_CODE_GROWTHBOOK_FEATURES",
            r#"{"tengu_poll":{"interval":2000}}"#,
        );
        std::env::set_var("CLAUDE_CODE_GROWTHBOOK_TENGU_POLL", r#"{"interval":3000}"#);
        assert_eq!(
            get_feature_value_cached_may_be_stale_json(
                "tengu_poll",
                serde_json::json!({"interval": 100})
            ),
            serde_json::json!({"interval": 3000})
        );
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_FEATURES");
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_TENGU_POLL");
    }
}
