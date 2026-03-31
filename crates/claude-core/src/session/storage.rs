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
}
