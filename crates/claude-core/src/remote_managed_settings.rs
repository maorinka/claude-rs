//! Policy-managed settings (read + apply layer).
//!
//! Port of the decision + cache half of `src/services/remoteManagedSettings/`.
//! TS ships 877 LOC covering ETag-cached HTTP fetch, OAuth eligibility,
//! zod validation, session cache, and a `settings.ts` merge integration.
//! This module ports the pieces that don't depend on the Rust HTTP /
//! OAuth / settings-pipeline that we haven't finished porting:
//!
//!   - a session cache backed by `~/.claude/remote-settings.json` (or
//!     `$CLAUDE_CONFIG_DIR/remote-settings.json`)
//!   - `apply_policy_overlay` that merges a policy-settings JSON Value
//!     on top of a user-settings JSON Value with POLICY WINS semantics
//!     (policy is an override, not a default)
//!   - load / save helpers so the future HTTP layer has a stable
//!     on-disk format
//!
//! Callers that later wire in the fetch/poll layer only need to call
//! `save_to_disk` with the freshly fetched Value.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use serde_json::Value;

const SETTINGS_FILENAME: &str = "remote-settings.json";

// ── Session cache ──────────────────────────────────────────────────────────

#[derive(Default)]
pub struct RemoteManagedSettingsCache {
    inner: RwLock<Option<Value>>,
}

impl RemoteManagedSettingsCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install a freshly-fetched settings value in the session cache.
    pub fn set(&self, settings: Value) {
        if let Ok(mut g) = self.inner.write() {
            *g = Some(settings);
        }
    }

    /// Clear the session cache. Matches TS `resetSyncCache`.
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.write() {
            *g = None;
        }
    }

    /// Current cached value (cloned), or None if not yet populated.
    pub fn get(&self) -> Option<Value> {
        self.inner.read().ok().and_then(|g| g.clone())
    }
}

static GLOBAL: OnceLock<Arc<RemoteManagedSettingsCache>> = OnceLock::new();

pub fn global() -> Arc<RemoteManagedSettingsCache> {
    GLOBAL
        .get_or_init(|| Arc::new(RemoteManagedSettingsCache::new()))
        .clone()
}

// ── Disk I/O ───────────────────────────────────────────────────────────────

pub fn get_settings_path() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(dir).join(SETTINGS_FILENAME);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude")
        .join(SETTINGS_FILENAME)
}

/// Load the policy-settings JSON from disk. Returns None on missing
/// file, malformed JSON, or any IO error — identical to TS catch-all.
pub fn load_from_disk() -> Option<Value> {
    let path = get_settings_path();
    let contents = std::fs::read_to_string(&path).ok()?;
    let stripped = contents.strip_prefix('\u{FEFF}').unwrap_or(&contents);
    let v: Value = serde_json::from_str(stripped).ok()?;
    // Guard: TS treats non-objects and arrays as null.
    if v.is_object() {
        Some(v)
    } else {
        None
    }
}

/// Save the policy-settings JSON to disk. Creates the parent directory
/// if it doesn't already exist. Returns Err on any IO / serialisation
/// failure so callers can decide whether to surface or silently retry.
pub fn save_to_disk(value: &Value) -> std::io::Result<()> {
    let path = get_settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pretty = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&path, pretty)
}

// ── Merge semantics ────────────────────────────────────────────────────────

/// Deep-merge `policy` on top of `user` with **policy-wins** semantics:
///   - for keys present in both where both values are objects: recurse.
///   - for keys present in both where values aren't both objects:
///     policy value overrides.
///   - for keys present only in policy: take policy value.
///   - for keys present only in user: take user value.
///
/// Arrays are treated as atomic values (policy replaces user's array).
///
/// Returns a new Value; inputs are not mutated.
pub fn apply_policy_overlay(user: &Value, policy: &Value) -> Value {
    match (user, policy) {
        (Value::Object(u), Value::Object(p)) => {
            let mut out = serde_json::Map::new();
            // Start with user's keys as a base so user-only keys are preserved.
            for (k, v) in u {
                if let Some(pv) = p.get(k) {
                    out.insert(k.clone(), apply_policy_overlay(v, pv));
                } else {
                    out.insert(k.clone(), v.clone());
                }
            }
            // Add policy-only keys.
            for (k, v) in p {
                if !u.contains_key(k) {
                    out.insert(k.clone(), v.clone());
                }
            }
            Value::Object(out)
        },
        // Non-object combinations: policy wins, full stop.
        (_, policy) => policy.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cache_install_and_get() {
        let c = RemoteManagedSettingsCache::new();
        assert!(c.get().is_none());
        c.set(json!({"model": "opus"}));
        let v = c.get().unwrap();
        assert_eq!(v["model"], "opus");
        c.clear();
        assert!(c.get().is_none());
    }

    #[test]
    fn policy_wins_on_scalar_conflict() {
        let user = json!({"model": "sonnet", "verbose": true});
        let policy = json!({"model": "opus"});
        let merged = apply_policy_overlay(&user, &policy);
        assert_eq!(merged["model"], "opus");
        assert_eq!(merged["verbose"], true);
    }

    #[test]
    fn merge_is_recursive_for_objects() {
        let user = json!({"permissions": {"allow": ["read"], "deny": []}, "misc": true});
        let policy = json!({"permissions": {"deny": ["write"]}});
        let merged = apply_policy_overlay(&user, &policy);
        assert_eq!(merged["permissions"]["allow"], json!(["read"]));
        assert_eq!(merged["permissions"]["deny"], json!(["write"]));
        assert_eq!(merged["misc"], true);
    }

    #[test]
    fn policy_only_keys_are_added() {
        let user = json!({"a": 1});
        let policy = json!({"b": 2});
        let merged = apply_policy_overlay(&user, &policy);
        assert_eq!(merged["a"], 1);
        assert_eq!(merged["b"], 2);
    }

    #[test]
    fn arrays_replaced_atomically() {
        let user = json!({"list": [1, 2, 3]});
        let policy = json!({"list": [9]});
        let merged = apply_policy_overlay(&user, &policy);
        assert_eq!(merged["list"], json!([9]));
    }

    #[test]
    fn non_object_user_is_overridden_wholesale() {
        let user = json!(42);
        let policy = json!({"model": "opus"});
        let merged = apply_policy_overlay(&user, &policy);
        assert_eq!(merged, json!({"model": "opus"}));
    }

    #[test]
    fn save_then_load_roundtrip() {
        // Use a scratch path via CLAUDE_CONFIG_DIR for isolation.
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path());
        let payload = json!({"model": "opus[1m]", "verbose": false});
        save_to_disk(&payload).unwrap();
        let loaded = load_from_disk().unwrap();
        assert_eq!(loaded, payload);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn load_missing_returns_none() {
        std::env::set_var(
            "CLAUDE_CONFIG_DIR",
            "/nonexistent-dir-for-remote-settings-test-xyz",
        );
        assert!(load_from_disk().is_none());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
}
