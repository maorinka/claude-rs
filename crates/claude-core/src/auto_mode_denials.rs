//! Auto-mode denial ring buffer.
//!
//! Port of TS `src/utils/autoModeDenials.ts`. When the classifier
//! blocks a tool call, a record is appended here; the /permissions
//! RecentDenialsTab tails the list. Fixed cap at 20 entries —
//! anything older rolls off.
//!
//! Feature-gated at the TS site via `feature('TRANSCRIPT_CLASSIFIER')`;
//! on Rust the gate is the caller's responsibility (this module
//! always records; gate at the call site to match TS semantics).

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Hard cap on stored denials. Matches TS MAX_DENIALS.
pub const MAX_DENIALS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoModeDenial {
    pub tool_name: String,
    /// Human-readable description of the denied command (e.g. the
    /// bash command string).
    pub display: String,
    pub reason: String,
    /// Unix ms when the denial was recorded.
    pub timestamp_ms: i64,
}

impl AutoModeDenial {
    pub fn new(tool_name: &str, display: &str, reason: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self {
            tool_name: tool_name.to_string(),
            display: display.to_string(),
            reason: reason.to_string(),
            timestamp_ms: now,
        }
    }
}

static STORE: Mutex<Vec<AutoModeDenial>> = Mutex::new(Vec::new());

/// Prepend `denial`; evicts the oldest when the list exceeds
/// `MAX_DENIALS`. Matches the TS `[denial, ...rest.slice(0, MAX-1)]`
/// shape so /permissions sees the most recent denial first.
pub fn record_auto_mode_denial(denial: AutoModeDenial) {
    let mut guard = STORE.lock().expect("auto_mode_denials mutex poisoned");
    guard.insert(0, denial);
    if guard.len() > MAX_DENIALS {
        guard.truncate(MAX_DENIALS);
    }
}

/// Snapshot the current denial list (newest first).
pub fn get_auto_mode_denials() -> Vec<AutoModeDenial> {
    STORE
        .lock()
        .expect("auto_mode_denials mutex poisoned")
        .clone()
}

/// Test-only reset.
pub fn clear_auto_mode_denials() {
    STORE
        .lock()
        .expect("auto_mode_denials mutex poisoned")
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn records_and_snapshots_newest_first() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_auto_mode_denials();
        record_auto_mode_denial(AutoModeDenial::new("bash", "rm -rf /", "dangerous"));
        record_auto_mode_denial(AutoModeDenial::new("bash", "ls", "not-really"));
        let out = get_auto_mode_denials();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].display, "ls");
        assert_eq!(out[1].display, "rm -rf /");
        clear_auto_mode_denials();
    }

    #[test]
    fn caps_at_max_denials() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_auto_mode_denials();
        for i in 0..MAX_DENIALS + 5 {
            record_auto_mode_denial(AutoModeDenial::new("tool", &format!("cmd-{i}"), "reason"));
        }
        let out = get_auto_mode_denials();
        assert_eq!(out.len(), MAX_DENIALS);
        // Newest first: last inserted is cmd-24 (MAX_DENIALS + 5 - 1 = 24).
        assert_eq!(out[0].display, format!("cmd-{}", MAX_DENIALS + 4));
        // Oldest retained is cmd-5 (the first 5 rolled off).
        assert_eq!(out[MAX_DENIALS - 1].display, "cmd-5");
        clear_auto_mode_denials();
    }

    #[test]
    fn timestamp_populated() {
        let _g = TEST_LOCK.lock().unwrap();
        clear_auto_mode_denials();
        let d = AutoModeDenial::new("bash", "x", "y");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!((now - d.timestamp_ms).abs() < 5_000);
        clear_auto_mode_denials();
    }
}
