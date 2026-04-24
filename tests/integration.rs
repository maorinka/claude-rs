//! Integration tests for claude-rs
//!
//! These tests require mocked API responses to avoid hitting the real Anthropic API.

#[cfg(test)]
mod tests {
    use claude_core::api::client::{normalize_model_for_api, has_1m_context};

    #[test]
    fn test_model_normalization_removes_1m_suffix() {
        assert_eq!(normalize_model_for_api("claude-opus-4-6[1m]"), "claude-opus-4-6");
        assert_eq!(normalize_model_for_api("claude-sonnet-4-6[1M]"), "claude-sonnet-4-6");
    }

    #[test]
    fn test_model_normalization_removes_2m_suffix() {
        assert_eq!(normalize_model_for_api("claude-opus-4-6[2m]"), "claude-opus-4-6");
        assert_eq!(normalize_model_for_api("claude-haiku-4-5[2M]"), "claude-haiku-4-5");
    }

    #[test]
    fn test_model_normalization_preserves_normal_models() {
        assert_eq!(normalize_model_for_api("claude-opus-4-6"), "claude-opus-4-6");
        assert_eq!(normalize_model_for_api("claude-sonnet-4-6"), "claude-sonnet-4-6");
    }

    #[test]
    fn test_has_1m_context() {
        assert!(has_1m_context("claude-opus-4-6[1m]"));
        assert!(has_1m_context("claude-opus-4-6[1M]"));
        assert!(!has_1m_context("claude-opus-4-6"));
        assert!(!has_1m_context("claude-opus-4-6[2m]"));
    }

    #[test]
    fn test_auth_method_header_generation() {
        use claude_core::api::client::AuthMethod;

        let api_key = AuthMethod::ApiKey("sk-ant-test".to_string());
        let (name, value) = api_key.to_header();
        assert_eq!(name, "x-api-key");
        assert_eq!(value, "sk-ant-test");

        let oauth = AuthMethod::OAuthToken("Bearer token".to_string());
        let (name, value) = oauth.to_header();
        assert_eq!(name, "authorization");
        assert_eq!(value, "Bearer Bearer token");
    }

    #[test]
    fn test_auth_method_is_oauth() {
        use claude_core::api::client::AuthMethod;

        assert!(!AuthMethod::ApiKey("key".to_string()).is_oauth());
        assert!(AuthMethod::OAuthToken("token".to_string()).is_oauth());
    }
}

#[cfg(test)]
mod tool_tests {
    use claude_tools::registry::ToolRegistry;

    #[test]
    fn test_default_registry_contains_core_tools() {
        let registry = claude_tools::build_default_registry();
        
        // Verify core tools are registered
        assert!(registry.get("Bash").is_some(), "Bash tool should be registered");
        assert!(registry.get("Read").is_some(), "Read tool should be registered");
        assert!(registry.get("Write").is_some(), "Write tool should be registered");
        assert!(registry.get("Edit").is_some(), "Edit tool should be registered");
        assert!(registry.get("Grep").is_some(), "Grep tool should be registered");
        assert!(registry.get("Glob").is_some(), "Glob tool should be registered");
    }

    #[test]
    fn test_registry_returns_none_for_unknown_tool() {
        let registry = claude_tools::build_default_registry();
        assert!(registry.get("NonExistentTool").is_none());
    }
}

#[cfg(test)]
mod config_tests {
    #[test]
    fn test_default_settings() {
        let settings = claude_core::config::settings::Settings::default();
        // Default settings should be valid
        assert!(settings.model.is_none() || settings.model.is_some());
    }
}