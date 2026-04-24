//! Session-scoped env vars set via `/env`.
//!
//! Port of TS `src/utils/sessionEnvVars.ts`. Stored in a global
//! `Mutex<BTreeMap<String, String>>` so all callers see the same
//! map without plumbing it through every struct literal. Applied
//! only to spawned child processes (bash provider env overrides),
//! never to the REPL process itself.
//!
//! `BTreeMap` gives deterministic iteration order (useful for
//! stable `/env` listing output and stable child-process
//! environments).

use std::collections::BTreeMap;
use std::sync::Mutex;

static STORE: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

/// Snapshot the current session env-var map.
pub fn get_session_env_vars() -> BTreeMap<String, String> {
    STORE.lock().expect("session env mutex poisoned").clone()
}

/// Set `name=value`, replacing any previous entry.
pub fn set_session_env_var(name: &str, value: &str) {
    let mut guard = STORE.lock().expect("session env mutex poisoned");
    guard.insert(name.to_string(), value.to_string());
}

/// Remove `name` from the session env-var map. No-op if absent.
pub fn delete_session_env_var(name: &str) {
    let mut guard = STORE.lock().expect("session env mutex poisoned");
    guard.remove(name);
}

/// Drop every session env-var entry.
pub fn clear_session_env_vars() {
    let mut guard = STORE.lock().expect("session env mutex poisoned");
    guard.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests touch the global store, so serialise them to avoid
    /// cross-test interference.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn set_then_get() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_session_env_vars();
        set_session_env_var("FOO", "bar");
        let snap = get_session_env_vars();
        assert_eq!(snap.get("FOO").map(String::as_str), Some("bar"));
        clear_session_env_vars();
    }

    #[test]
    fn set_replaces_previous_value() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_session_env_vars();
        set_session_env_var("FOO", "one");
        set_session_env_var("FOO", "two");
        assert_eq!(
            get_session_env_vars().get("FOO").map(String::as_str),
            Some("two")
        );
        clear_session_env_vars();
    }

    #[test]
    fn delete_removes_entry() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_session_env_vars();
        set_session_env_var("K", "v");
        delete_session_env_var("K");
        assert!(!get_session_env_vars().contains_key("K"));
    }

    #[test]
    fn clear_empties_map() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_session_env_vars();
        set_session_env_var("A", "1");
        set_session_env_var("B", "2");
        clear_session_env_vars();
        assert!(get_session_env_vars().is_empty());
    }

    #[test]
    fn snapshot_is_deterministic_ordering() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_session_env_vars();
        set_session_env_var("Z", "1");
        set_session_env_var("A", "2");
        set_session_env_var("M", "3");
        let snap = get_session_env_vars();
        let keys: Vec<&str> = snap.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["A", "M", "Z"]);
        clear_session_env_vars();
    }
}
