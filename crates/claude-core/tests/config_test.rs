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
    let json = r#"{"model":"claude-opus-4-6","verbose":true,"permissions":{"allow":[{"tool":"Read"}],"deny":[]},"allowedHttpHookUrls":["https://hooks.example.com/*"],"httpHookAllowedEnvVars":["TOKEN"]}"#;
    let settings: Settings = serde_json::from_str(json).unwrap();
    assert_eq!(settings.model.as_deref(), Some("claude-opus-4-6"));
    assert_eq!(settings.verbose, Some(true));
    assert_eq!(settings.permissions.allow.len(), 1);
    assert_eq!(settings.permissions.allow[0].tool, "Read");
    assert_eq!(
        settings.allowed_http_hook_urls.as_ref().unwrap(),
        &vec!["https://hooks.example.com/*".to_string()]
    );
    assert_eq!(
        settings.http_hook_allowed_env_vars.as_ref().unwrap(),
        &vec!["TOKEN".to_string()]
    );
}

#[test]
fn test_settings_mcp_http_server_deserialize() {
    let json = r#"{
        "mcpServers": {
            "docs": {
                "type": "http",
                "url": "https://example.com/mcp",
                "headers": {"Authorization": "Bearer token"}
            }
        }
    }"#;
    let settings: Settings = serde_json::from_str(json).unwrap();
    let entry = settings.mcp_servers.get("docs").unwrap();
    let config = entry.to_mcp_server_config().unwrap();
    match config {
        claude_core::mcp::types::McpServerConfig::Http(http) => {
            assert_eq!(http.url, "https://example.com/mcp");
            assert_eq!(
                http.headers
                    .as_ref()
                    .unwrap()
                    .get("Authorization")
                    .map(String::as_str),
                Some("Bearer token")
            );
        }
        other => panic!("expected http config, got {other:?}"),
    }
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

#[test]
fn test_settings_merge_http_hook_policy_lists() {
    let base = Settings {
        allowed_http_hook_urls: Some(vec!["https://base.example.com/*".into()]),
        http_hook_allowed_env_vars: Some(vec!["BASE_TOKEN".into()]),
        ..Default::default()
    };
    let overlay = Settings {
        allowed_http_hook_urls: Some(vec!["https://overlay.example.com/*".into()]),
        http_hook_allowed_env_vars: Some(vec!["OVERLAY_TOKEN".into()]),
        ..Default::default()
    };
    let merged = base.merge(&overlay);
    assert_eq!(
        merged.allowed_http_hook_urls.unwrap(),
        vec![
            "https://base.example.com/*".to_string(),
            "https://overlay.example.com/*".to_string()
        ]
    );
    assert_eq!(
        merged.http_hook_allowed_env_vars.unwrap(),
        vec!["BASE_TOKEN".to_string(), "OVERLAY_TOKEN".to_string()]
    );
}
