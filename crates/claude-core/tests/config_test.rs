use claude_core::config::paths::*;
use claude_core::config::settings::*;

#[test]
fn test_claude_dir() {
    let dir = claude_dir().unwrap();
    assert!(dir.ends_with(".claude"));
}

#[test]
fn test_detect_project_root_with_git() {
    let tmp = std::env::temp_dir().join("claude_test_git");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(tmp.join(".git")).unwrap();
    let sub = tmp.join("a/b/c");
    std::fs::create_dir_all(&sub).unwrap();
    let root = detect_project_root(&sub);
    assert_eq!(root, tmp);
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn test_detect_project_root_with_cargo_toml() {
    let tmp = std::env::temp_dir().join("claude_test_cargo");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("Cargo.toml"), "[package]").unwrap();
    let sub = tmp.join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();
    let root = detect_project_root(&sub);
    assert_eq!(root, tmp);
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn test_settings_deserialize() {
    let json = r#"{"model":"claude-opus-4-6","verbose":true,"permissions":{"allow":[{"tool":"Read"}],"deny":[]}}"#;
    let settings: Settings = serde_json::from_str(json).unwrap();
    assert_eq!(settings.model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(settings.verbose, Some(true));
    assert_eq!(settings.permissions.allow.len(), 1);
    assert_eq!(settings.permissions.allow[0].tool, "Read");
}

#[test]
fn test_settings_merge() {
    let base = Settings {
        model: Some("claude-sonnet-4-6".into()),
        verbose: Some(false),
        ..Default::default()
    };
    let overlay = Settings {
        model: Some("claude-opus-4-6".into()),
        ..Default::default()
    };
    let merged = base.merge(&overlay);
    assert_eq!(merged.model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(merged.verbose, Some(false));
}
