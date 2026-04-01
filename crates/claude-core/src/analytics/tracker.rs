use super::types::*;
use std::sync::Mutex;
use once_cell::sync::Lazy;

static EVENTS: Lazy<Mutex<Vec<AnalyticsEvent>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub struct AnalyticsTracker {
    enabled: bool,
    metadata: SessionMetadata,
}

impl AnalyticsTracker {
    pub fn new(metadata: SessionMetadata, enabled: bool) -> Self {
        Self { enabled, metadata }
    }

    pub fn log_event(&self, event_name: &str, properties: serde_json::Value) {
        if !self.enabled {
            return;
        }
        let event = AnalyticsEvent {
            event_name: event_name.to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            properties,
            session_id: self.metadata.session_id.clone(),
            device_id: self.metadata.device_id.clone(),
        };
        if let Ok(mut events) = EVENTS.lock() {
            events.push(event);
        }
    }

    pub fn flush() -> Vec<AnalyticsEvent> {
        EVENTS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
    }

    pub fn event_count() -> usize {
        EVENTS.lock().map(|v| v.len()).unwrap_or(0)
    }

    /// Count only events for the given session_id (non-destructive).
    pub fn session_event_count(session_id: &str) -> usize {
        EVENTS
            .lock()
            .map(|v| v.iter().filter(|e| e.session_id == session_id).count())
            .unwrap_or(0)
    }

    /// Drain only events that belong to the given session_id.
    /// Other sessions' events remain in the store.
    /// Useful for isolating parallel tests that share global state.
    pub fn drain_session(session_id: &str) -> Vec<AnalyticsEvent> {
        if let Ok(mut events) = EVENTS.lock() {
            let mut mine = Vec::new();
            let mut remaining = Vec::new();
            for event in events.drain(..) {
                if event.session_id == session_id {
                    mine.push(event);
                } else {
                    remaining.push(event);
                }
            }
            *events = remaining;
            mine
        } else {
            Vec::new()
        }
    }
}
