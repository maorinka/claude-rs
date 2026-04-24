//! Port of `migrateReplBridgeEnabledToRemoteControlAtStartup.ts`.
//!
//! Simple key rename in GlobalConfig. Our GlobalConfig stores legacy/extra
//! fields in `extra: serde_json::Map<String, Value>`, so this migration
//! operates on that.

use crate::config::global::GlobalConfig;
use serde_json::Value;

/// If `replBridgeEnabled` exists in extras and `remoteControlAtStartup` does
/// not, copy the value (as bool) to the new key and remove the old one.
/// Idempotent.
///
/// Returns true if state changed.
pub fn migrate_repl_bridge_to_remote_control(cfg: &mut GlobalConfig) -> bool {
    let Some(old_value) = cfg.extra.get("replBridgeEnabled").cloned() else {
        return false;
    };
    if cfg.extra.contains_key("remoteControlAtStartup") {
        // Already migrated (or user set the new key manually). Drop the
        // legacy key anyway so we don't keep re-checking.
        cfg.extra.remove("replBridgeEnabled");
        return true;
    }
    let as_bool = matches!(old_value, Value::Bool(true))
        || matches!(old_value, Value::String(ref s) if s == "true");
    cfg.extra
        .insert("remoteControlAtStartup".to_string(), Value::Bool(as_bool));
    cfg.extra.remove("replBridgeEnabled");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(extra: serde_json::Map<String, Value>) -> GlobalConfig {
        GlobalConfig {
            extra,
            ..Default::default()
        }
    }

    #[test]
    fn copies_bool_true_to_new_key() {
        let mut map = serde_json::Map::new();
        map.insert("replBridgeEnabled".into(), Value::Bool(true));
        let mut cfg = cfg_with(map);
        assert!(migrate_repl_bridge_to_remote_control(&mut cfg));
        assert_eq!(
            cfg.extra.get("remoteControlAtStartup"),
            Some(&Value::Bool(true))
        );
        assert!(!cfg.extra.contains_key("replBridgeEnabled"));
    }

    #[test]
    fn noop_when_old_key_absent() {
        let mut cfg = cfg_with(serde_json::Map::new());
        assert!(!migrate_repl_bridge_to_remote_control(&mut cfg));
    }

    #[test]
    fn drops_legacy_key_when_new_already_set() {
        let mut map = serde_json::Map::new();
        map.insert("replBridgeEnabled".into(), Value::Bool(true));
        map.insert("remoteControlAtStartup".into(), Value::Bool(false));
        let mut cfg = cfg_with(map);
        assert!(migrate_repl_bridge_to_remote_control(&mut cfg));
        assert_eq!(
            cfg.extra.get("remoteControlAtStartup"),
            Some(&Value::Bool(false))
        );
        assert!(!cfg.extra.contains_key("replBridgeEnabled"));
    }

    #[test]
    fn is_idempotent() {
        let mut map = serde_json::Map::new();
        map.insert("replBridgeEnabled".into(), Value::Bool(true));
        let mut cfg = cfg_with(map);
        assert!(migrate_repl_bridge_to_remote_control(&mut cfg));
        assert!(!migrate_repl_bridge_to_remote_control(&mut cfg));
    }
}
