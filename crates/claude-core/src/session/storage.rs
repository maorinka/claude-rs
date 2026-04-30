use anyhow::Result;
use std::path::PathBuf;
use std::sync::OnceLock;

static INTERNAL_EVENT_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<InternalTranscriptEvent>> =
    OnceLock::new();

#[derive(Clone, Debug, PartialEq)]
pub struct InternalTranscriptEvent {
    pub payload: serde_json::Value,
    pub is_compaction: bool,
    pub agent_id: Option<String>,
}

pub fn set_internal_event_sender(
    sender: tokio::sync::mpsc::UnboundedSender<InternalTranscriptEvent>,
) {
    let _ = INTERNAL_EVENT_TX.set(sender);
}

pub fn internal_event_from_transcript_line(line: &str) -> Option<InternalTranscriptEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut payload = serde_json::from_str::<serde_json::Value>(trimmed).ok()?;
    if !payload.is_object() {
        return None;
    }
    if payload
        .get("uuid")
        .and_then(|value| value.as_str())
        .is_none()
    {
        payload["uuid"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
    }
    let is_compaction = payload
        .get("type")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value == "compact_boundary")
        || payload
            .get("subtype")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == "compact_boundary");
    let agent_id = payload
        .get("agentId")
        .or_else(|| payload.get("agent_id"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    Some(InternalTranscriptEvent {
        payload,
        is_compaction,
        agent_id,
    })
}

pub struct SessionStorage {
    pub session_dir: PathBuf,
}

impl SessionStorage {
    pub fn new(session_id: &str) -> Result<Self> {
        let sessions_dir = crate::config::paths::sessions_dir()?;
        let session_dir = sessions_dir.join(session_id);
        std::fs::create_dir_all(&session_dir)?;
        Ok(Self { session_dir })
    }

    pub fn transcript_path(&self) -> PathBuf {
        self.session_dir.join("transcript.jsonl")
    }

    pub fn append_transcript(&self, line: &str) -> Result<()> {
        use std::io::Write;
        let path = self.transcript_path();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(file, "{}", line)?;
        if let (Some(sender), Some(event)) = (
            INTERNAL_EVENT_TX.get(),
            internal_event_from_transcript_line(line),
        ) {
            let _ = sender.send(event);
        }
        Ok(())
    }

    pub fn replace_transcript(&self, entries: &[serde_json::Value]) -> Result<()> {
        use std::io::Write;
        let path = self.transcript_path();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        for entry in entries {
            writeln!(file, "{}", serde_json::to_string(entry)?)?;
        }
        Ok(())
    }

    /// Load the transcript as a list of JSON values (one per line).
    /// Each line is expected to be a JSON object representing a message.
    pub fn load_transcript(&self) -> Result<Vec<serde_json::Value>> {
        let path = self.transcript_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&path)?;
        let mut messages = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(val) => messages.push(val),
                Err(e) => {
                    tracing::warn!("skipping malformed transcript line: {}", e);
                }
            }
        }
        Ok(messages)
    }

    /// Create a `SessionStorage` rooted at a specific directory (for testing).
    #[cfg(test)]
    pub fn from_dir(session_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&session_dir).ok();
        Self { session_dir }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_load_empty_transcript() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = SessionStorage::from_dir(tmp.path().to_path_buf());
        let messages = storage.load_transcript().unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_load_transcript_with_messages() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = SessionStorage::from_dir(tmp.path().to_path_buf());

        // Write some transcript lines
        let msg1 = json!({"role": "user", "content": [{"type": "text", "text": "hello"}]});
        let msg2 = json!({"role": "assistant", "content": [{"type": "text", "text": "hi there"}]});
        storage
            .append_transcript(&serde_json::to_string(&msg1).unwrap())
            .unwrap();
        storage
            .append_transcript(&serde_json::to_string(&msg2).unwrap())
            .unwrap();

        let messages = storage.load_transcript().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
    }

    #[test]
    fn test_load_transcript_skips_malformed_lines() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = SessionStorage::from_dir(tmp.path().to_path_buf());

        let msg1 = json!({"role": "user", "content": "hello"});
        storage
            .append_transcript(&serde_json::to_string(&msg1).unwrap())
            .unwrap();
        storage.append_transcript("not valid json!!!").unwrap();
        storage.append_transcript("").unwrap(); // empty line

        let messages = storage.load_transcript().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn test_load_transcript_populates_messages() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = SessionStorage::from_dir(tmp.path().to_path_buf());

        let msg1 = json!({"role": "user", "content": [{"type": "text", "text": "what is 2+2?"}]});
        let msg2 = json!({"role": "assistant", "content": [{"type": "text", "text": "4"}]});
        let msg3 = json!({"role": "user", "content": [{"type": "text", "text": "thanks"}]});
        storage
            .append_transcript(&serde_json::to_string(&msg1).unwrap())
            .unwrap();
        storage
            .append_transcript(&serde_json::to_string(&msg2).unwrap())
            .unwrap();
        storage
            .append_transcript(&serde_json::to_string(&msg3).unwrap())
            .unwrap();

        let messages = storage.load_transcript().unwrap();
        assert_eq!(messages.len(), 3);

        // Verify messages alternate user/assistant/user
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "user");

        // Verify content is preserved
        let first_text = messages[0]["content"][0]["text"].as_str().unwrap();
        assert_eq!(first_text, "what is 2+2?");
    }

    #[test]
    fn replace_transcript_overwrites_existing_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = SessionStorage::from_dir(tmp.path().to_path_buf());

        storage
            .append_transcript(r#"{"role":"user","content":"old"}"#)
            .unwrap();
        storage
            .replace_transcript(&[
                json!({"role": "user", "content": "new"}),
                json!({"role": "assistant", "content": "fresh"}),
            ])
            .unwrap();

        let messages = storage.load_transcript().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], "new");
        assert_eq!(messages[1]["content"], "fresh");
    }

    #[test]
    fn internal_event_from_transcript_line_preserves_payload_type() {
        let event = internal_event_from_transcript_line(
            r#"{"type":"assistant","message":{"content":"hi"},"agentId":"agent-1"}"#,
        )
        .unwrap();

        assert_eq!(event.payload["type"], "assistant");
        assert_eq!(event.agent_id.as_deref(), Some("agent-1"));
        assert!(!event.is_compaction);
        assert!(event.payload["uuid"].as_str().is_some());
    }

    #[test]
    fn internal_event_from_transcript_line_marks_compaction() {
        let event =
            internal_event_from_transcript_line(r#"{"type":"compact_boundary","uuid":"u1"}"#)
                .unwrap();

        assert_eq!(event.payload["uuid"], "u1");
        assert!(event.is_compaction);
    }

    #[test]
    fn internal_event_from_transcript_line_skips_invalid_entries() {
        assert!(internal_event_from_transcript_line("").is_none());
        assert!(internal_event_from_transcript_line("not json").is_none());
        assert!(internal_event_from_transcript_line(r#""string""#).is_none());
    }
}
