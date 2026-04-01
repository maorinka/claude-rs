#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::NamedTempFile;

    use crate::vcr::player::VcrPlayer;
    use crate::vcr::recorder::VcrRecorder;
    use crate::vcr::types::VcrFixture;

    fn make_recorder() -> VcrRecorder {
        VcrRecorder::new(true)
    }

    // ---- recorder tests --------------------------------------------------------

    #[test]
    fn test_record_and_count() {
        let mut recorder = make_recorder();
        assert_eq!(recorder.request_count(), 0);

        recorder.record(
            "POST",
            "https://api.anthropic.com/v1/messages",
            &json!({"model": "claude-opus-4-5"}),
            200,
            r#"{"id":"msg_01"}"#,
        );
        assert_eq!(recorder.request_count(), 1);

        recorder.record(
            "GET",
            "https://api.anthropic.com/v1/models",
            &json!(null),
            200,
            r#"{"models":[]}"#,
        );
        assert_eq!(recorder.request_count(), 2);
    }

    #[test]
    fn test_disabled_recorder_does_not_record() {
        let mut recorder = VcrRecorder::new(false);
        recorder.record("POST", "https://example.com", &json!({}), 200, "ok");
        assert_eq!(recorder.request_count(), 0);
    }

    #[test]
    fn test_save_creates_valid_json() {
        let mut recorder = make_recorder();
        recorder.record(
            "POST",
            "https://api.anthropic.com/v1/messages",
            &json!({"prompt": "hello"}),
            201,
            r#"{"text":"world"}"#,
        );

        let tmp = NamedTempFile::new().unwrap();
        recorder.save(tmp.path()).unwrap();

        let content = std::fs::read_to_string(tmp.path()).unwrap();
        let fixture: VcrFixture = serde_json::from_str(&content).unwrap();
        assert_eq!(fixture.version, 1);
        assert_eq!(fixture.requests.len(), 1);
        assert_eq!(fixture.requests[0].method, "POST");
        assert_eq!(fixture.requests[0].response_status, 201);
    }

    // ---- player tests ----------------------------------------------------------

    #[test]
    fn test_player_load_and_replay() {
        // Record two interactions and save.
        let mut recorder = make_recorder();
        recorder.record("POST", "https://api.example.com/a", &json!({"a":1}), 200, "resp-a");
        recorder.record("DELETE", "https://api.example.com/b", &json!({"b":2}), 204, "resp-b");

        let tmp = NamedTempFile::new().unwrap();
        recorder.save(tmp.path()).unwrap();

        // Replay from the saved fixture.
        let mut player = VcrPlayer::load(tmp.path()).unwrap();
        assert_eq!(player.remaining(), 2);

        let first = player.next_response().unwrap();
        assert_eq!(first.method, "POST");
        assert_eq!(first.response_body, "resp-a");
        assert_eq!(player.remaining(), 1);

        let second = player.next_response().unwrap();
        assert_eq!(second.method, "DELETE");
        assert_eq!(second.response_status, 204);
        assert_eq!(player.remaining(), 0);

        assert!(player.next_response().is_none());
    }

    #[test]
    fn test_player_reset() {
        let mut recorder = make_recorder();
        recorder.record("GET", "https://api.example.com/ping", &json!(null), 200, "pong");

        let tmp = NamedTempFile::new().unwrap();
        recorder.save(tmp.path()).unwrap();

        let mut player = VcrPlayer::load(tmp.path()).unwrap();
        assert!(player.next_response().is_some());
        assert_eq!(player.remaining(), 0);

        player.reset();
        assert_eq!(player.remaining(), 1);
        assert!(player.next_response().is_some());
    }

    #[test]
    fn test_empty_fixture_round_trip() {
        let recorder = VcrRecorder::new(true); // nothing recorded
        assert_eq!(recorder.request_count(), 0);

        let tmp = NamedTempFile::new().unwrap();
        recorder.save(tmp.path()).unwrap();

        let mut player = VcrPlayer::load(tmp.path()).unwrap();
        assert_eq!(player.remaining(), 0);
        assert!(player.next_response().is_none());
    }

    #[test]
    fn test_full_record_save_load_replay_cycle() {
        let method = "POST";
        let url = "https://api.anthropic.com/v1/messages";
        let req_body = json!({"model": "claude-opus-4-5", "messages": [{"role": "user", "content": "hi"}]});
        let resp_status: u16 = 200;
        let resp_body = r#"{"id":"msg_01","type":"message","content":[{"type":"text","text":"Hello!"}]}"#;

        // --- record ---
        let mut recorder = make_recorder();
        recorder.record(method, url, &req_body, resp_status, resp_body);
        assert_eq!(recorder.request_count(), 1);

        let tmp = NamedTempFile::new().unwrap();
        recorder.save(tmp.path()).unwrap();

        // --- replay ---
        let mut player = VcrPlayer::load(tmp.path()).unwrap();
        let req = player.next_response().unwrap();
        assert_eq!(req.method, method);
        assert_eq!(req.url, url);
        assert_eq!(req.request_body, req_body);
        assert_eq!(req.response_status, resp_status);
        assert_eq!(req.response_body, resp_body);
        assert!(req.timestamp > 0);
    }
}
