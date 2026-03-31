use anyhow::Result;
use crate::api::client::AuthMethod;

/// Authentication resolution result
pub enum AuthResolution {
    /// Direct API key — can call the API directly
    ApiKey(AuthMethod),
    /// OAuth token found — need to use claude binary proxy
    OAuthProxy,
    /// No auth found
    None,
}

/// Resolve authentication method by priority (matches real Claude Code):
/// 1. ANTHROPIC_API_KEY env var → direct API
/// 2. Stored OAuth tokens (keychain) → proxy through claude binary
/// 3. Claude binary available (already logged in) → proxy
/// 4. None
pub async fn resolve_auth() -> Result<AuthResolution> {
    // 1. Check env var
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return Ok(AuthResolution::ApiKey(AuthMethod::ApiKey(key)));
        }
    }

    // 2. Check stored OAuth tokens (from real Claude Code login)
    if let Some(_tokens) = super::storage::load_tokens().await? {
        // OAuth tokens require Anthropic's internal SDK — proxy through claude binary
        if crate::api::claude_proxy::is_claude_available() {
            return Ok(AuthResolution::OAuthProxy);
        }
        // Have tokens but no claude binary — can't use them
        tracing::warn!("Found OAuth tokens but `claude` binary not available for proxy");
    }

    // 3. Check if claude binary is available (user may be logged in)
    if crate::api::claude_proxy::is_claude_available() {
        return Ok(AuthResolution::OAuthProxy);
    }

    Ok(AuthResolution::None)
}
