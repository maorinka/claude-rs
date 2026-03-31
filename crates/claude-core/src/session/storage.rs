use anyhow::Result;
use std::path::PathBuf;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

pub struct SessionStorage {
    session_dir: PathBuf,
}

impl SessionStorage {
    pub fn new(session_id: &str) -> Result<Self> {
        let dir = crate::config::paths::sessions_dir()?.join(session_id);
        std::fs::create_dir_all(&dir)?;
        Ok(Self { session_dir: dir })
    }

    /// Append messages to transcript (JSONL format)
    pub async fn write_transcript(&self, messages: &[Value]) -> Result<()> {
        let path = self.session_dir.join("transcript.jsonl");
        let mut content = String::new();
        for msg in messages {
            content.push_str(&serde_json::to_string(msg)?);
            content.push('\n');
        }
        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?
            .write_all(content.as_bytes())
            .await?;
        Ok(())
    }

    /// Load session transcript
    pub async fn load_transcript(&self) -> Result<Vec<Value>> {
        let path = self.session_dir.join("transcript.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        content
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| serde_json::from_str(line).map_err(Into::into))
            .collect()
    }
}
