#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::analytics::tracker::AnalyticsTracker;
    use crate::analytics::types::{AnalyticsEvent, SessionMetadata};

    fn make_metadata(suffix: &str) -> SessionMetadata {
        SessionMetadata {
            session_id: format!("session-{suffix}"),
            device_id: format!("device-{suffix}"),
            model: "claude-opus-4-5".to_string(),
            subscription_type: Some("pro".to_string()),
            platform: "darwin".to_string(),
            cli_version: "1.0.0".to_string(),
        }
    }

    /// Drain only events for this session from the global store.
    /// Other sessions' events remain so parallel tests don't bleed into each other.
    fn drain_for(suffix: &str) -> Vec<AnalyticsEvent> {
        AnalyticsTracker::drain_session(&format!("session-{suffix}"))
    }

    // ---- basic logging ---------------------------------------------------------

    #[test]
    fn test_log_event_stores_event() {
        let id = "log_event";
        let tracker = AnalyticsTracker::new(make_metadata(id), true);
        tracker.log_event("test_action", json!({"key": "value"}));

        let events = drain_for(id);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name, "test_action");
        assert_eq!(events[0].properties["key"], "value");
        assert_eq!(events[0].session_id, format!("session-{id}"));
        assert_eq!(events[0].device_id, format!("device-{id}"));
        assert!(events[0].timestamp > 0);
    }

    #[test]
    fn test_multiple_events_accumulate() {
        let id = "multi_events";
        let tracker = AnalyticsTracker::new(make_metadata(id), true);
        tracker.log_event("event_one", json!({"n": 1}));
        tracker.log_event("event_two", json!({"n": 2}));
        tracker.log_event("event_three", json!({"n": 3}));

        let events = drain_for(id);
        assert_eq!(events.len(), 3);
        let names: Vec<&str> = events.iter().map(|e| e.event_name.as_str()).collect();
        assert!(names.contains(&"event_one"));
        assert!(names.contains(&"event_two"));
        assert!(names.contains(&"event_three"));
    }

    // ---- disabled tracker ------------------------------------------------------

    #[test]
    fn test_disabled_tracker_does_not_log() {
        let id = "disabled";
        // Flush any residual events first.
        drain_for(id);

        let tracker = AnalyticsTracker::new(make_metadata(id), false);
        tracker.log_event("should_not_appear", json!({}));

        let events = drain_for(id);
        assert_eq!(events.len(), 0, "disabled tracker should not log events");
    }

    // ---- flush -----------------------------------------------------------------

    #[test]
    fn test_flush_clears_global_store() {
        let id = "flush_clears";
        // Make sure the store is empty for our session.
        drain_for(id);

        let tracker = AnalyticsTracker::new(make_metadata(id), true);
        tracker.log_event("before_flush", json!({}));

        // Drain our session events (session-scoped so parallel tests don't interfere).
        let drained = drain_for(id);
        assert_eq!(drained.len(), 1, "should drain 1 event for our session");

        // Now our session should have 0 events.
        let session_id = format!("session-{id}");
        let count_after = AnalyticsTracker::session_event_count(&session_id);
        assert_eq!(count_after, 0, "flush should clear our session's stored events");
    }

    #[test]
    fn test_flush_returns_logged_events() {
        let id = "flush_returns";
        // Clear any leftover state for our session.
        drain_for(id);

        let tracker = AnalyticsTracker::new(make_metadata(id), true);
        tracker.log_event("ev_a", json!({"x": 1}));
        tracker.log_event("ev_b", json!({"x": 2}));

        // Use drain_for to filter by our session_id, tolerating parallel tests
        // that may have added events for other sessions.
        let mine = drain_for(id);
        assert_eq!(mine.len(), 2, "expected 2 events for session-{id}");
    }

    // ---- event_count -----------------------------------------------------------

    #[test]
    fn test_event_count_reflects_logged_events() {
        let id = "count_test";
        // Clear our session's events first.
        drain_for(id);

        let tracker = AnalyticsTracker::new(make_metadata(id), true);
        // Our session should have 0 events (other sessions may exist but we
        // test per-session count to avoid parallel-test interference).
        let session_id = format!("session-{id}");
        assert_eq!(AnalyticsTracker::session_event_count(&session_id), 0);

        tracker.log_event("one", json!({}));
        assert_eq!(AnalyticsTracker::session_event_count(&session_id), 1);

        tracker.log_event("two", json!({}));
        assert_eq!(AnalyticsTracker::session_event_count(&session_id), 2);

        drain_for(id);
        assert_eq!(AnalyticsTracker::session_event_count(&session_id), 0);
    }

    // ---- metadata serialization ------------------------------------------------

    #[test]
    fn test_metadata_serialization_round_trip() {
        let meta = SessionMetadata {
            session_id: "sess-abc".to_string(),
            device_id: "dev-xyz".to_string(),
            model: "claude-opus-4-5".to_string(),
            subscription_type: Some("enterprise".to_string()),
            platform: "linux".to_string(),
            cli_version: "2.3.1".to_string(),
        };

        let json_str = serde_json::to_string(&meta).unwrap();
        let round_tripped: SessionMetadata = serde_json::from_str(&json_str).unwrap();

        assert_eq!(round_tripped.session_id, meta.session_id);
        assert_eq!(round_tripped.device_id, meta.device_id);
        assert_eq!(round_tripped.model, meta.model);
        assert_eq!(round_tripped.subscription_type, meta.subscription_type);
        assert_eq!(round_tripped.platform, meta.platform);
        assert_eq!(round_tripped.cli_version, meta.cli_version);
    }

    #[test]
    fn test_metadata_optional_subscription_type_none() {
        let meta = SessionMetadata {
            session_id: "s1".to_string(),
            device_id: "d1".to_string(),
            model: "claude-haiku".to_string(),
            subscription_type: None,
            platform: "windows".to_string(),
            cli_version: "0.1.0".to_string(),
        };

        let json_str = serde_json::to_string(&meta).unwrap();
        let round_tripped: SessionMetadata = serde_json::from_str(&json_str).unwrap();
        assert!(round_tripped.subscription_type.is_none());
    }

    #[test]
    fn test_analytics_event_serialization() {
        let event = AnalyticsEvent {
            event_name: "button_clicked".to_string(),
            timestamp: 1_700_000_000_000,
            properties: json!({"button": "submit", "page": "checkout"}),
            session_id: "sess-1".to_string(),
            device_id: "dev-1".to_string(),
        };

        let json_str = serde_json::to_string(&event).unwrap();
        let parsed: AnalyticsEvent = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.event_name, "button_clicked");
        assert_eq!(parsed.timestamp, 1_700_000_000_000);
        assert_eq!(parsed.properties["button"], "submit");
        assert_eq!(parsed.session_id, "sess-1");
        assert_eq!(parsed.device_id, "dev-1");
    }
}
