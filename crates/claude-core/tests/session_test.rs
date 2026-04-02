use claude_core::session::manager::SessionManager;

#[test]
fn test_session_manager_new_creates_unique_ids() {
    let s1 = SessionManager::new().expect("new session");
    let s2 = SessionManager::new().expect("new session");
    assert_ne!(s1.session_id(), s2.session_id());
    assert!(!s1.session_id().is_empty());
}

#[test]
fn test_session_manager_resume() {
    let original = SessionManager::new().expect("new session");
    let id = original.session_id().to_string();

    let resumed = SessionManager::resume(&id).expect("resume session");
    assert_eq!(resumed.session_id(), id);
}

#[test]
fn test_session_manager_session_dir_created() {
    let mgr = SessionManager::new().expect("new session");
    assert!(mgr.storage().session_dir.exists());
}

#[test]
fn test_list_sessions_returns_vec() {
    // Just verify the API works without error (real sessions dir may or may not exist)
    let result = SessionManager::list_sessions();
    assert!(result.is_ok());
}

#[test]
fn test_list_sessions_includes_created_session() {
    let mgr = SessionManager::new().expect("new session");
    let id = mgr.session_id().to_string();

    // Write a transcript so last_modified is populated
    mgr.storage()
        .append_transcript(r#"{"role":"user"}"#)
        .expect("write transcript");

    let sessions = SessionManager::list_sessions().expect("list sessions");
    let found = sessions.iter().any(|s| s.id == id);
    assert!(found, "newly created session should appear in list");
}
