use anyhow::Result;
use std::path::PathBuf;

pub struct TeamMemorySync {
    team_dir: PathBuf,
}

impl TeamMemorySync {
    pub fn new(team_id: &str) -> Result<Self> {
        let dir = crate::config::paths::claude_dir()?
            .join("teams")
            .join(team_id)
            .join("memory");
        std::fs::create_dir_all(&dir)?;
        Ok(Self { team_dir: dir })
    }

    /// Create with an explicit directory (useful for testing).
    pub fn with_dir(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { team_dir: dir })
    }

    pub fn write_memory(&self, key: &str, value: &str) -> Result<()> {
        std::fs::write(self.team_dir.join(format!("{}.md", key)), value)?;
        Ok(())
    }

    pub fn read_memory(&self, key: &str) -> Result<Option<String>> {
        let path = self.team_dir.join(format!("{}.md", key));
        if path.exists() {
            Ok(Some(std::fs::read_to_string(path)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_memories(&self) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        if self.team_dir.exists() {
            for entry in std::fs::read_dir(&self.team_dir)? {
                let entry = entry?;
                if let Some(name) = entry.path().file_stem() {
                    keys.push(name.to_string_lossy().to_string());
                }
            }
        }
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_sync(tmp: &TempDir) -> TeamMemorySync {
        let dir = tmp.path().join("memory");
        TeamMemorySync::with_dir(dir).unwrap()
    }

    #[test]
    fn test_write_and_read_memory() {
        let tmp = TempDir::new().unwrap();
        let sync = make_sync(&tmp);

        sync.write_memory("goal", "Build something great").unwrap();
        let val = sync.read_memory("goal").unwrap();
        assert_eq!(val.as_deref(), Some("Build something great"));
    }

    #[test]
    fn test_read_nonexistent_memory() {
        let tmp = TempDir::new().unwrap();
        let sync = make_sync(&tmp);

        let val = sync.read_memory("does-not-exist").unwrap();
        assert!(val.is_none());
    }

    #[test]
    fn test_list_memories_empty() {
        let tmp = TempDir::new().unwrap();
        let sync = make_sync(&tmp);

        let keys = sync.list_memories().unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn test_list_memories_after_writes() {
        let tmp = TempDir::new().unwrap();
        let sync = make_sync(&tmp);

        sync.write_memory("context", "project context").unwrap();
        sync.write_memory("decisions", "decision log").unwrap();
        sync.write_memory("todos", "remaining items").unwrap();

        let mut keys = sync.list_memories().unwrap();
        keys.sort();
        assert_eq!(keys, vec!["context", "decisions", "todos"]);
    }

    #[test]
    fn test_overwrite_memory() {
        let tmp = TempDir::new().unwrap();
        let sync = make_sync(&tmp);

        sync.write_memory("note", "first version").unwrap();
        sync.write_memory("note", "second version").unwrap();

        let val = sync.read_memory("note").unwrap();
        assert_eq!(val.as_deref(), Some("second version"));
    }

    #[test]
    fn test_memory_persists_content() {
        let tmp = TempDir::new().unwrap();
        let sync = make_sync(&tmp);

        let content = "# Memory Entry\n\nThis is a multi-line\nmemory entry.";
        sync.write_memory("my-memory", content).unwrap();
        let read_back = sync.read_memory("my-memory").unwrap().unwrap();
        assert_eq!(read_back, content);
    }

    #[test]
    fn test_list_memories_counts() {
        let tmp = TempDir::new().unwrap();
        let sync = make_sync(&tmp);

        for i in 0..5 {
            sync.write_memory(&format!("key-{}", i), &format!("value {}", i))
                .unwrap();
        }

        let keys = sync.list_memories().unwrap();
        assert_eq!(keys.len(), 5);
    }
}
