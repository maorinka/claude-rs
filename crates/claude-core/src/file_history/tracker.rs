use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A single snapshot of a file at a point in time.
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// Monotonically increasing sequence number for ordering.
    pub sequence: u64,
    /// Timestamp when the snapshot was taken.
    pub timestamp: SystemTime,
    /// Path to the snapshot file on disk.
    pub snapshot_path: PathBuf,
}

/// Tracks file snapshots for a session so edits can be undone.
///
/// Before each write/edit a snapshot of the original file is stored under
/// `<session_dir>/file_snapshots/`.  The snapshot file name encodes the
/// sequence number so snapshots sort in creation order.
pub struct FileHistoryTracker {
    /// Directory where snapshot files are written.
    snapshot_dir: PathBuf,
    /// Map from original file path → list of snapshots in creation order.
    snapshots: HashMap<PathBuf, Vec<SnapshotEntry>>,
    /// Monotonically increasing counter.
    sequence: u64,
}

impl FileHistoryTracker {
    /// Create a new tracker rooted at `session_dir/file_snapshots/`.
    /// The directory is created lazily when the first snapshot is taken.
    pub fn new(session_dir: &Path) -> Self {
        let snapshot_dir = session_dir.join("file_snapshots");
        Self {
            snapshot_dir,
            snapshots: HashMap::new(),
            sequence: 0,
        }
    }

    /// Take a snapshot of `file_path` before it is modified.
    ///
    /// If the file does not exist (new file creation) this is a no-op and
    /// returns `Ok(())`.
    pub fn snapshot(&mut self, file_path: &Path) -> Result<()> {
        if !file_path.exists() {
            return Ok(());
        }

        std::fs::create_dir_all(&self.snapshot_dir)?;

        self.sequence += 1;
        let base_name = file_path.file_name().unwrap_or_default().to_string_lossy();
        let snapshot_name = format!("{}_{}", self.sequence, base_name);
        let snapshot_path = self.snapshot_dir.join(&snapshot_name);

        std::fs::copy(file_path, &snapshot_path)?;

        self.snapshots
            .entry(file_path.to_path_buf())
            .or_default()
            .push(SnapshotEntry {
                sequence: self.sequence,
                timestamp: SystemTime::now(),
                snapshot_path,
            });

        Ok(())
    }

    /// Restore `file_path` to the content of its most recent snapshot.
    ///
    /// Returns `Ok(true)` if a snapshot was found and restored, `Ok(false)`
    /// if no snapshots exist for this file.
    pub fn restore(&self, file_path: &Path) -> Result<bool> {
        if let Some(entries) = self.snapshots.get(file_path) {
            if let Some(last) = entries.last() {
                std::fs::copy(&last.snapshot_path, file_path)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Return all snapshots recorded for `file_path`, oldest first.
    pub fn history(&self, file_path: &Path) -> Vec<&SnapshotEntry> {
        self.snapshots
            .get(file_path)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Return the snapshot directory path (for inspection / testing).
    pub fn snapshot_dir(&self) -> &Path {
        &self.snapshot_dir
    }
}
