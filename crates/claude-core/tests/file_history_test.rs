use claude_core::file_history::FileHistoryTracker;
use std::fs;

/// Helper: create a temp directory for a test session.
fn tmp_session() -> tempfile::TempDir {
    tempfile::tempdir().expect("failed to create temp dir")
}

#[test]
fn test_snapshot_of_nonexistent_file_is_noop() {
    let session = tmp_session();
    let mut tracker = FileHistoryTracker::new(session.path());

    let nonexistent = session.path().join("does_not_exist.txt");
    // Should not error and should not create snapshot
    let result = tracker.snapshot(&nonexistent);
    assert!(result.is_ok(), "snapshot of nonexistent file should be Ok");

    let history = tracker.history(&nonexistent);
    assert!(history.is_empty(), "history should be empty for nonexistent file");
}

#[test]
fn test_snapshot_captures_file_content() {
    let session = tmp_session();
    let mut tracker = FileHistoryTracker::new(session.path());

    let file = session.path().join("hello.txt");
    fs::write(&file, "original content").unwrap();

    tracker.snapshot(&file).expect("snapshot should succeed");

    let history = tracker.history(&file);
    assert_eq!(history.len(), 1, "should have one snapshot");

    let snapshot_content = fs::read_to_string(&history[0].snapshot_path)
        .expect("snapshot file should be readable");
    assert_eq!(snapshot_content, "original content");
}

#[test]
fn test_restore_reverts_to_snapshot() {
    let session = tmp_session();
    let mut tracker = FileHistoryTracker::new(session.path());

    let file = session.path().join("restore_me.txt");
    fs::write(&file, "before edit").unwrap();

    tracker.snapshot(&file).expect("snapshot should succeed");

    // Simulate an edit
    fs::write(&file, "after edit").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "after edit");

    // Restore
    let restored = tracker.restore(&file).expect("restore should succeed");
    assert!(restored, "restore should return true when snapshot exists");

    let restored_content = fs::read_to_string(&file).unwrap();
    assert_eq!(restored_content, "before edit", "content should be reverted");
}

#[test]
fn test_restore_returns_false_when_no_snapshot() {
    let session = tmp_session();
    let tracker = FileHistoryTracker::new(session.path());

    let file = session.path().join("no_snapshot.txt");
    fs::write(&file, "some content").unwrap();

    let restored = tracker.restore(&file).expect("restore should not error");
    assert!(!restored, "restore should return false when no snapshot exists");
}

#[test]
fn test_history_listing_multiple_snapshots() {
    let session = tmp_session();
    let mut tracker = FileHistoryTracker::new(session.path());

    let file = session.path().join("multi.txt");
    fs::write(&file, "v1").unwrap();
    tracker.snapshot(&file).expect("snapshot 1");

    fs::write(&file, "v2").unwrap();
    tracker.snapshot(&file).expect("snapshot 2");

    fs::write(&file, "v3").unwrap();
    tracker.snapshot(&file).expect("snapshot 3");

    let history = tracker.history(&file);
    assert_eq!(history.len(), 3, "should have three snapshots");

    // Snapshots should be in creation order (oldest first)
    assert!(
        history[0].sequence < history[1].sequence,
        "snapshots should be ordered oldest-first"
    );
    assert!(
        history[1].sequence < history[2].sequence,
        "snapshots should be ordered oldest-first"
    );

    // Content check: each snapshot holds the version at the time
    assert_eq!(fs::read_to_string(&history[0].snapshot_path).unwrap(), "v1");
    assert_eq!(fs::read_to_string(&history[1].snapshot_path).unwrap(), "v2");
    assert_eq!(fs::read_to_string(&history[2].snapshot_path).unwrap(), "v3");
}

#[test]
fn test_restore_uses_most_recent_snapshot() {
    let session = tmp_session();
    let mut tracker = FileHistoryTracker::new(session.path());

    let file = session.path().join("most_recent.txt");
    fs::write(&file, "first").unwrap();
    tracker.snapshot(&file).expect("snapshot 1");

    fs::write(&file, "second").unwrap();
    tracker.snapshot(&file).expect("snapshot 2");

    // Now overwrite with third content
    fs::write(&file, "third").unwrap();

    // Restore should revert to "second" (most recent snapshot)
    tracker.restore(&file).expect("restore should succeed");
    assert_eq!(fs::read_to_string(&file).unwrap(), "second");
}

#[test]
fn test_snapshot_creates_snapshot_directory() {
    let session = tmp_session();
    let mut tracker = FileHistoryTracker::new(session.path());

    let file = session.path().join("dir_test.txt");
    fs::write(&file, "content").unwrap();

    // Before snapshot the dir should not exist
    assert!(!tracker.snapshot_dir().exists() || tracker.snapshot_dir().read_dir().map(|mut d| d.next().is_none()).unwrap_or(true),
        "snapshot dir should be empty/absent before first snapshot");

    tracker.snapshot(&file).expect("snapshot should succeed");

    assert!(tracker.snapshot_dir().exists(), "snapshot dir should be created");
}
