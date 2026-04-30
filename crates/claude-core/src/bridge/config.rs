//! Shared bridge auth and URL resolution.
//!
//! Mirrors TS `src/bridge/bridgeConfig.ts`: ant-only dev overrides are exposed
//! separately, and normal bridge callers fall through to Claude.ai OAuth tokens
//! plus the OAuth base API URL.

use anyhow::Result;

pub fn get_bridge_token_override() -> Option<String> {
    if !crate::user_type::is_ant() {
        return None;
    }
    std::env::var("CLAUDE_BRIDGE_OAUTH_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
}

pub fn get_bridge_base_url_override() -> Option<String> {
    if !crate::user_type::is_ant() {
        return None;
    }
    std::env::var("CLAUDE_BRIDGE_BASE_URL")
        .ok()
        .filter(|base_url| !base_url.trim().is_empty())
}

pub async fn get_bridge_access_token() -> Option<String> {
    if let Some(token) = get_bridge_token_override() {
        return Some(token);
    }
    crate::auth::storage::load_tokens()
        .await
        .ok()
        .flatten()
        .map(|tokens| tokens.access_token)
        .filter(|token| !token.trim().is_empty())
}

pub fn get_bridge_base_url() -> Result<String> {
    if let Some(base_url) = get_bridge_base_url_override() {
        return Ok(base_url);
    }
    Ok(crate::constants::oauth::get_oauth_config()?.base_api_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::ENV_LOCK;

    #[test]
    fn bridge_overrides_are_ant_only() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("USER_TYPE", "external");
        std::env::set_var("CLAUDE_BRIDGE_OAUTH_TOKEN", "tok");
        std::env::set_var("CLAUDE_BRIDGE_BASE_URL", "https://bridge.example");
        assert_eq!(get_bridge_token_override(), None);
        assert_eq!(get_bridge_base_url_override(), None);
        std::env::remove_var("CLAUDE_BRIDGE_OAUTH_TOKEN");
        std::env::remove_var("CLAUDE_BRIDGE_BASE_URL");
        std::env::remove_var("USER_TYPE");
    }

    #[test]
    fn bridge_overrides_read_when_ant() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("USER_TYPE", "ant");
        std::env::set_var("CLAUDE_BRIDGE_OAUTH_TOKEN", "tok");
        std::env::set_var("CLAUDE_BRIDGE_BASE_URL", "https://bridge.example");
        assert_eq!(get_bridge_token_override().as_deref(), Some("tok"));
        assert_eq!(
            get_bridge_base_url_override().as_deref(),
            Some("https://bridge.example")
        );
        std::env::remove_var("CLAUDE_BRIDGE_OAUTH_TOKEN");
        std::env::remove_var("CLAUDE_BRIDGE_BASE_URL");
        std::env::remove_var("USER_TYPE");
    }
}
