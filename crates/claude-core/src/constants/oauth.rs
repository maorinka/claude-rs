//! OAuth configuration constants + environment-aware config resolver.
//!
//! Port of `src/constants/oauth.ts`. Picks prod / staging / local /
//! custom (FedStart) OAuth config based on env vars; returns a config
//! struct the auth flow threads through its HTTP calls.

/// OAuth scopes used during authorize / token exchange.
pub const CLAUDE_AI_INFERENCE_SCOPE: &str = "user:inference";
pub const CLAUDE_AI_PROFILE_SCOPE: &str = "user:profile";
pub const CONSOLE_SCOPE: &str = "org:create_api_key";

pub const OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";

/// Console OAuth scopes — for API key creation via Console.
pub const CONSOLE_OAUTH_SCOPES: &[&str] = &[CONSOLE_SCOPE, CLAUDE_AI_PROFILE_SCOPE];

/// Claude.ai OAuth scopes — for Claude.ai subscribers (Pro/Max/Team/Enterprise).
pub const CLAUDE_AI_OAUTH_SCOPES: &[&str] = &[
    CLAUDE_AI_PROFILE_SCOPE,
    CLAUDE_AI_INFERENCE_SCOPE,
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
];

/// Client ID Metadata Document URL for MCP OAuth (CIMD / SEP-991).
/// Matches TS `MCP_CLIENT_METADATA_URL`.
pub const MCP_CLIENT_METADATA_URL: &str = "https://claude.ai/oauth/claude-code-client-metadata";

/// Allowed base URLs for the `CLAUDE_CODE_CUSTOM_OAUTH_URL` override.
/// Only FedStart / PubSec deployments are permitted to keep tokens
/// from going to arbitrary endpoints. Matches TS allowlist.
pub const ALLOWED_OAUTH_BASE_URLS: &[&str] = &[
    "https://beacon.claude-ai.staging.ant.dev",
    "https://claude.fedstart.com",
    "https://claude-staging.fedstart.com",
];

#[derive(Debug, Clone)]
pub struct OauthConfig {
    pub base_api_url: String,
    pub console_authorize_url: String,
    pub claude_ai_authorize_url: String,
    /// The claude.ai web origin. Separate from `claude_ai_authorize_url`
    /// because that routes through claude.com/cai/* for attribution —
    /// deriving origin from it would give claude.com.
    pub claude_ai_origin: String,
    pub token_url: String,
    pub api_key_url: String,
    pub roles_url: String,
    pub console_success_url: String,
    pub claudeai_success_url: String,
    pub manual_redirect_url: String,
    pub client_id: String,
    pub oauth_file_suffix: String,
    pub mcp_proxy_url: String,
    pub mcp_proxy_path: String,
}

fn is_env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref().map(|v| v.to_ascii_lowercase()),
        Some(s) if matches!(s.as_str(), "1" | "true" | "yes" | "on")
    )
}

fn is_ant_user() -> bool {
    std::env::var("USER_TYPE").map(|v| v == "ant").unwrap_or(false)
}

fn prod_oauth_config() -> OauthConfig {
    OauthConfig {
        base_api_url: "https://api.anthropic.com".into(),
        console_authorize_url: "https://platform.claude.com/oauth/authorize".into(),
        // Bounces through claude.com/cai/* for attribution.
        claude_ai_authorize_url: "https://claude.com/cai/oauth/authorize".into(),
        claude_ai_origin: "https://claude.ai".into(),
        token_url: "https://platform.claude.com/v1/oauth/token".into(),
        api_key_url: "https://api.anthropic.com/api/oauth/claude_cli/create_api_key".into(),
        roles_url: "https://api.anthropic.com/api/oauth/claude_cli/roles".into(),
        console_success_url:
            "https://platform.claude.com/buy_credits?returnUrl=/oauth/code/success%3Fapp%3Dclaude-code"
                .into(),
        claudeai_success_url: "https://platform.claude.com/oauth/code/success?app=claude-code".into(),
        manual_redirect_url: "https://platform.claude.com/oauth/code/callback".into(),
        client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".into(),
        oauth_file_suffix: String::new(),
        mcp_proxy_url: "https://mcp-proxy.anthropic.com".into(),
        mcp_proxy_path: "/v1/mcp/{server_id}".into(),
    }
}

fn staging_oauth_config() -> OauthConfig {
    OauthConfig {
        base_api_url: "https://api-staging.anthropic.com".into(),
        console_authorize_url: "https://platform.staging.ant.dev/oauth/authorize".into(),
        claude_ai_authorize_url: "https://claude-ai.staging.ant.dev/oauth/authorize".into(),
        claude_ai_origin: "https://claude-ai.staging.ant.dev".into(),
        token_url: "https://platform.staging.ant.dev/v1/oauth/token".into(),
        api_key_url:
            "https://api-staging.anthropic.com/api/oauth/claude_cli/create_api_key".into(),
        roles_url: "https://api-staging.anthropic.com/api/oauth/claude_cli/roles".into(),
        console_success_url:
            "https://platform.staging.ant.dev/buy_credits?returnUrl=/oauth/code/success%3Fapp%3Dclaude-code"
                .into(),
        claudeai_success_url:
            "https://platform.staging.ant.dev/oauth/code/success?app=claude-code".into(),
        manual_redirect_url: "https://platform.staging.ant.dev/oauth/code/callback".into(),
        client_id: "22422756-60c9-4084-8eb7-27705fd5cf9a".into(),
        oauth_file_suffix: "-staging-oauth".into(),
        mcp_proxy_url: "https://mcp-proxy-staging.anthropic.com".into(),
        mcp_proxy_path: "/v1/mcp/{server_id}".into(),
    }
}

fn local_oauth_config() -> OauthConfig {
    let api = std::env::var("CLAUDE_LOCAL_OAUTH_API_BASE")
        .unwrap_or_else(|_| "http://localhost:8000".into())
        .trim_end_matches('/')
        .to_string();
    let apps = std::env::var("CLAUDE_LOCAL_OAUTH_APPS_BASE")
        .unwrap_or_else(|_| "http://localhost:4000".into())
        .trim_end_matches('/')
        .to_string();
    let console_base = std::env::var("CLAUDE_LOCAL_OAUTH_CONSOLE_BASE")
        .unwrap_or_else(|_| "http://localhost:3000".into())
        .trim_end_matches('/')
        .to_string();

    OauthConfig {
        base_api_url: api.clone(),
        console_authorize_url: format!("{}/oauth/authorize", console_base),
        claude_ai_authorize_url: format!("{}/oauth/authorize", apps),
        claude_ai_origin: apps.clone(),
        token_url: format!("{}/v1/oauth/token", api),
        api_key_url: format!("{}/api/oauth/claude_cli/create_api_key", api),
        roles_url: format!("{}/api/oauth/claude_cli/roles", api),
        console_success_url: format!(
            "{}/buy_credits?returnUrl=/oauth/code/success%3Fapp%3Dclaude-code",
            console_base
        ),
        claudeai_success_url: format!("{}/oauth/code/success?app=claude-code", console_base),
        manual_redirect_url: format!("{}/oauth/code/callback", console_base),
        client_id: "22422756-60c9-4084-8eb7-27705fd5cf9a".into(),
        oauth_file_suffix: "-local-oauth".into(),
        mcp_proxy_url: "http://localhost:8205".into(),
        mcp_proxy_path: "/v1/toolbox/shttp/mcp/{server_id}".into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OauthConfigType {
    Prod,
    Staging,
    Local,
}

fn get_oauth_config_type() -> OauthConfigType {
    if is_ant_user() {
        if is_env_truthy("USE_LOCAL_OAUTH") {
            return OauthConfigType::Local;
        }
        if is_env_truthy("USE_STAGING_OAUTH") {
            return OauthConfigType::Staging;
        }
    }
    OauthConfigType::Prod
}

/// Filename suffix based on which OAuth env the user is in — used to
/// keep prod/staging/local token files separate so a dev with staging
/// tokens doesn't accidentally ship them to prod.
pub fn file_suffix_for_oauth_config() -> &'static str {
    if std::env::var("CLAUDE_CODE_CUSTOM_OAUTH_URL").is_ok() {
        return "-custom-oauth";
    }
    match get_oauth_config_type() {
        OauthConfigType::Local => "-local-oauth",
        OauthConfigType::Staging => "-staging-oauth",
        OauthConfigType::Prod => "",
    }
}

/// Result of `get_oauth_config` — either a valid config or a rejection
/// error when the custom-URL override points at a non-allowlisted
/// endpoint.
#[derive(Debug)]
pub enum OauthConfigError {
    DisallowedCustomUrl,
}

impl std::fmt::Display for OauthConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OauthConfigError::DisallowedCustomUrl => {
                write!(f, "CLAUDE_CODE_CUSTOM_OAUTH_URL is not an approved endpoint.")
            }
        }
    }
}

impl std::error::Error for OauthConfigError {}

/// Return the OAuth config for the current environment. Errors only
/// when the custom-URL override is set to a non-allowlisted base.
pub fn get_oauth_config() -> Result<OauthConfig, OauthConfigError> {
    let mut config = match get_oauth_config_type() {
        OauthConfigType::Local => local_oauth_config(),
        OauthConfigType::Staging => staging_oauth_config(),
        OauthConfigType::Prod => prod_oauth_config(),
    };

    if let Ok(raw) = std::env::var("CLAUDE_CODE_CUSTOM_OAUTH_URL") {
        let base = raw.trim_end_matches('/').to_string();
        if !ALLOWED_OAUTH_BASE_URLS.iter().any(|u| *u == base) {
            return Err(OauthConfigError::DisallowedCustomUrl);
        }
        config = OauthConfig {
            base_api_url: base.clone(),
            console_authorize_url: format!("{}/oauth/authorize", base),
            claude_ai_authorize_url: format!("{}/oauth/authorize", base),
            claude_ai_origin: base.clone(),
            token_url: format!("{}/v1/oauth/token", base),
            api_key_url: format!("{}/api/oauth/claude_cli/create_api_key", base),
            roles_url: format!("{}/api/oauth/claude_cli/roles", base),
            console_success_url: format!("{}/oauth/code/success?app=claude-code", base),
            claudeai_success_url: format!("{}/oauth/code/success?app=claude-code", base),
            manual_redirect_url: format!("{}/oauth/code/callback", base),
            oauth_file_suffix: "-custom-oauth".into(),
            ..config
        };
    }

    if let Ok(client_id) = std::env::var("CLAUDE_CODE_OAUTH_CLIENT_ID") {
        config.client_id = client_id;
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ENV_LOCK;

    fn clear_env() {
        for k in &[
            "USER_TYPE",
            "USE_LOCAL_OAUTH",
            "USE_STAGING_OAUTH",
            "CLAUDE_CODE_CUSTOM_OAUTH_URL",
            "CLAUDE_CODE_OAUTH_CLIENT_ID",
            "CLAUDE_LOCAL_OAUTH_API_BASE",
            "CLAUDE_LOCAL_OAUTH_APPS_BASE",
            "CLAUDE_LOCAL_OAUTH_CONSOLE_BASE",
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn prod_is_default() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        let cfg = get_oauth_config().unwrap();
        assert_eq!(cfg.base_api_url, "https://api.anthropic.com");
        assert_eq!(cfg.oauth_file_suffix, "");
    }

    #[test]
    fn staging_when_ant_with_staging_flag() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var("USE_STAGING_OAUTH", "1");
        let cfg = get_oauth_config().unwrap();
        assert!(cfg.base_api_url.contains("staging"));
        assert_eq!(cfg.oauth_file_suffix, "-staging-oauth");
        clear_env();
    }

    #[test]
    fn local_when_ant_with_local_flag() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var("USE_LOCAL_OAUTH", "1");
        let cfg = get_oauth_config().unwrap();
        assert!(cfg.base_api_url.starts_with("http://localhost"));
        assert_eq!(cfg.oauth_file_suffix, "-local-oauth");
        clear_env();
    }

    #[test]
    fn file_suffix_for_custom_wins() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(
            "CLAUDE_CODE_CUSTOM_OAUTH_URL",
            "https://claude.fedstart.com",
        );
        assert_eq!(file_suffix_for_oauth_config(), "-custom-oauth");
        clear_env();
    }

    #[test]
    fn custom_url_allowlisted() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(
            "CLAUDE_CODE_CUSTOM_OAUTH_URL",
            "https://claude.fedstart.com",
        );
        let cfg = get_oauth_config().unwrap();
        assert_eq!(cfg.base_api_url, "https://claude.fedstart.com");
        assert_eq!(cfg.oauth_file_suffix, "-custom-oauth");
        clear_env();
    }

    #[test]
    fn custom_url_rejects_unknown_base() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var(
            "CLAUDE_CODE_CUSTOM_OAUTH_URL",
            "https://evil.example/",
        );
        let err = get_oauth_config().unwrap_err();
        assert!(matches!(err, OauthConfigError::DisallowedCustomUrl));
        clear_env();
    }

    #[test]
    fn client_id_override_applied() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("CLAUDE_CODE_OAUTH_CLIENT_ID", "my-custom-id");
        let cfg = get_oauth_config().unwrap();
        assert_eq!(cfg.client_id, "my-custom-id");
        clear_env();
    }

    #[test]
    fn oauth_scopes_include_session_and_profile() {
        assert!(CLAUDE_AI_OAUTH_SCOPES.contains(&CLAUDE_AI_INFERENCE_SCOPE));
        assert!(CLAUDE_AI_OAUTH_SCOPES.contains(&CLAUDE_AI_PROFILE_SCOPE));
    }
}
