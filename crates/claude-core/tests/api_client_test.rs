use claude_core::api::client::*;

#[test]
fn test_api_config_default() {
    let config = ApiConfig::default();
    assert_eq!(config.base_url, "https://api.anthropic.com");
    assert_eq!(config.max_tokens, 64000);
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
    let body = build_request_body(&config, &[], &[], &[]);
    assert_eq!(body["model"], "claude-sonnet-4-6");
    assert_eq!(body["max_tokens"], 8000);
    assert_eq!(body["stream"], true);
    assert_eq!(body["thinking"]["type"], "adaptive");
}

#[test]
fn test_build_request_body_thinking_enabled() {
    let config = ApiConfig {
        model: "claude-sonnet-4-6".into(),
        thinking: ThinkingConfig::Enabled { budget_tokens: 10000 },
        ..Default::default()
    };
    let body = build_request_body(&config, &[], &[], &[]);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 10000);
}

#[test]
fn test_build_request_body_with_speed() {
    let config = ApiConfig {
        model: "claude-sonnet-4-6".into(),
        speed: Some(Speed::Fast),
        ..Default::default()
    };
    let body = build_request_body(&config, &[], &[], &[]);
    assert_eq!(body["speed"], "fast");
}

#[test]
fn test_build_request_body_includes_web_search_server_tool() {
    let config = ApiConfig::default();
    let body = build_request_body(&config, &[], &[], &[]);

    // The tools array should contain the web_search server tool definition
    let tools = body["tools"].as_array().expect("tools should be an array");
    let web_search = tools.iter().find(|t| t["name"] == "web_search");
    assert!(web_search.is_some(), "web_search server tool should be present in tools");

    let ws = web_search.unwrap();
    assert_eq!(ws["type"], "web_search_20250305");
    assert_eq!(ws["name"], "web_search");
    assert_eq!(ws["max_uses"], 8);
}

#[test]
fn test_build_request_body_web_search_appended_to_existing_tools() {
    let config = ApiConfig::default();
    let tools = vec![ToolDefinition {
        name: "MyTool".to_string(),
        description: "A tool".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let body = build_request_body(&config, &[], &[], &tools);

    let tools_arr = body["tools"].as_array().expect("tools should be an array");
    // Should have the regular tool + web_search server tool
    assert_eq!(tools_arr.len(), 2);
    assert_eq!(tools_arr[0]["name"], "MyTool");
    assert_eq!(tools_arr[1]["name"], "web_search");
    assert_eq!(tools_arr[1]["type"], "web_search_20250305");
}
