use anyhow::Result;
use std::path::PathBuf;

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
        storage.append_transcript(&serde_json::to_string(&msg1).unwrap()).unwrap();
        storage.append_transcript(&serde_json::to_string(&msg2).unwrap()).unwrap();

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
        storage.append_transcript(&serde_json::to_string(&msg1).unwrap()).unwrap();
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
        storage.append_transcript(&serde_json::to_string(&msg1).unwrap()).unwrap();
        storage.append_transcript(&serde_json::to_string(&msg2).unwrap()).unwrap();
        storage.append_transcript(&serde_json::to_string(&msg3).unwrap()).unwrap();

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
}
