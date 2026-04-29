use claude_core::api::client::*;

#[test]
fn test_api_config_default() {
    let config = ApiConfig::default();
    assert_eq!(config.base_url, "https://api.anthropic.com");
    assert!(config.fallback_model.is_none());
    assert!(config.max_tokens > 0);
}

#[test]
fn test_auth_method_api_key_header() {
    let auth = AuthMethod::ApiKey("sk-ant-test123".into());
    let (header_name, header_value) = auth.to_header();
    assert_eq!(header_name, "x-api-key");
    assert_eq!(header_value, "sk-ant-test123");
}

#[test]
fn test_auth_method_oauth_header() {
    let auth = AuthMethod::OAuthToken("token123".into());
    let (header_name, header_value) = auth.to_header();
    assert_eq!(header_name, "authorization");
    assert_eq!(header_value, "Bearer token123");
}

#[test]
fn test_build_request_body() {
    let config = ApiConfig {
        model: "claude-sonnet-4-6".into(),
        max_tokens: 8000,
        thinking: ThinkingConfig::Adaptive,
        ..Default::default()
    };
    let body = build_request_body(&config, &[], &[], &[], false);
    assert_eq!(body["model"], "claude-sonnet-4-6");
    assert_eq!(body["max_tokens"], 8000);
    assert_eq!(body["stream"], true);
    assert_eq!(body["thinking"]["type"], "adaptive");
}

#[test]
fn test_build_request_body_thinking_enabled() {
    let config = ApiConfig {
        model: "claude-sonnet-4-6".into(),
        max_tokens: 8192,
        thinking: ThinkingConfig::Enabled {
            budget_tokens: 10000,
        },
        ..Default::default()
    };
    let body = build_request_body(&config, &[], &[], &[], false);
    assert_eq!(body["thinking"]["type"], "enabled");
    // budget_tokens is clamped to max_tokens - 1 (Issue 29 fix)
    assert_eq!(body["thinking"]["budget_tokens"], 8191);
}

#[test]
fn test_build_request_body_with_speed() {
    let config = ApiConfig {
        model: "claude-sonnet-4-6".into(),
        speed: Some(Speed::Fast),
        ..Default::default()
    };
    let body = build_request_body(&config, &[], &[], &[], false);
    assert_eq!(body["speed"], "fast");
}

#[test]
fn test_build_request_body_with_tools() {
    let config = ApiConfig::default();
    let tools = vec![ToolDefinition {
        name: "MyTool".to_string(),
        description: "A tool".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
        defer_loading: false,
    }];
    let body = build_request_body(&config, &[], &[], &tools, false);

    let tools_arr = body["tools"].as_array().expect("tools should be an array");
    assert!(tools_arr.iter().any(|t| t["name"] == "MyTool"));
}

#[test]
fn test_build_request_body_no_tools() {
    let config = ApiConfig::default();
    let body = build_request_body(&config, &[], &[], &[], false);
    // With no tools provided, tools field should be absent or empty
    let tools = &body["tools"];
    assert!(tools.is_null() || tools.as_array().is_none_or(|a| a.is_empty()));
}
