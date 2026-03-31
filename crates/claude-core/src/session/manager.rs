use anyhow::Result;
use std::path::PathBuf;
use uuid::Uuid;
use super::storage::SessionStorage;

pub struct SessionManager {
    session_id: String,
    storage: SessionStorage,
}

impl SessionManager {
    pub fn new() -> Result<Self> {
        let session_id = Uuid::new_v4().to_string();
        let storage = SessionStorage::new(&session_id)?;
        Ok(Self { session_id, storage })
    }

    pub fn resume(session_id: &str) -> Result<Self> {
        let storage = SessionStorage::new(session_id)?;
        Ok(Self { session_id: session_id.to_string(), storage })
    }

    pub fn session_id(&self) -> &str { &self.session_id }
    pub fn storage(&self) -> &SessionStorage { &self.storage }

    /// List recent sessions
    pub fn list_sessions() -> Result<Vec<SessionInfo>> {
        let sessions_dir = crate::config::paths::sessions_dir()?;
        if !sessions_dir.exists() { return Ok(Vec::new()); }
        let mut sessions = Vec::new();
        for entry in std::fs::read_dir(&sessions_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let id = entry.file_name().to_string_lossy().to_string();
                let transcript = entry.path().join("transcript.jsonl");
                let modified = transcript.metadata().ok()
                    .and_then(|m| m.modified().ok());
                sessions.push(SessionInfo {
                    id,
                    path: entry.path(),
                    last_modified: modified,
                });
            }
        }
        sessions.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
        Ok(sessions)
    }
}

pub struct SessionInfo {
    pub id: String,
    pub path: PathBuf,
    pub last_modified: Option<std::time::SystemTime>,
}
