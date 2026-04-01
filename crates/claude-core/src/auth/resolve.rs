use anyhow::Result;
use crate::api::client::AuthMethod;
use super::login::{
    TOKEN_URL, is_claude_ai_auth, build_token_refresh_body,
};

/// Check if an OAuth token has expired (with 5 minute buffer, matching TS).
fn is_token_expired(expires_at: Option<u64>) -> bool {
    match expires_at {
        None => false,
        Some(exp) => {
            let buffer_ms = 5 * 60 * 1000; // 5 minutes, matching TS
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            now_ms + buffer_ms >= exp
        }
    }
}

/// Resolve authentication.
///
/// Checks in order (matching the TS `getAuthTokenSource` / `getAnthropicClient`):
/// 1. ANTHROPIC_API_KEY env var -> ApiKey
/// 2. CLAUDE_CODE_OAUTH_TOKEN env var -> OAuthToken (inference-only)
/// 3. Stored OAuth tokens from keychain/file -> OAuthToken (with refresh if expired)
///
/// For Claude.ai subscribers (scopes include `user:inference`):
///   - Uses `authToken` (sent as `Authorization: Bearer <token>`)
///
/// For Console users (no `user:inference` scope):
///   - The stored API key is used instead (from the create_api_key flow)
pub async fn resolve_auth() -> Result<AuthMethod> {
    // 1. Check env var (direct API key)
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return Ok(AuthMethod::ApiKey(key));
        }
    }

    // 2. Check for OAuth token from env var (inference-only, e.g. from CCD/CCR)
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        if !token.is_empty() {
            return Ok(AuthMethod::OAuthToken(token));
        }
    }

    // 3. Read stored OAuth tokens from keychain and refresh if needed
    if let Some(tokens) = super::storage::load_tokens().await? {
        // Check if this is a Claude.ai subscriber
        if is_claude_ai_auth(&tokens.scopes) {
            // Claude.ai subscriber — use authToken (Bearer)
            if let Some(refresh_token) = &tokens.refresh_token {
                if is_token_expired(tokens.expires_at) {
                    // Token expired — refresh it
                    match refresh_oauth_token(refresh_token, &tokens.scopes).await {
                        Ok(fresh) => return Ok(AuthMethod::OAuthToken(fresh)),
                        Err(e) => {
                            tracing::warn!("Token refresh failed ({}), using stored token", e);
                            return Ok(AuthMethod::OAuthToken(tokens.access_token));
                        }
                    }
                }
            }
            return Ok(AuthMethod::OAuthToken(tokens.access_token));
        } else {
            // Console user — they should have an API key stored
            // (created during login via create_api_key endpoint)
            // Try to use the OAuth token to create/get an API key,
            // but fall through to the OAuth token as fallback
            return Ok(AuthMethod::OAuthToken(tokens.access_token));
        }
    }

    anyhow::bail!("No authentication found. Run `claude login` or set ANTHROPIC_API_KEY")
}

/// Refresh an OAuth token via the token endpoint.
///
/// For Claude.ai subscribers, uses `CLAUDE_AI_OAUTH_SCOPES` as default
/// (allows scope expansion on refresh without re-login, matching TS).
/// For Console users, uses the original scopes.
async fn refresh_oauth_token(refresh_token: &str, stored_scopes: &[String]) -> Result<String> {
    // For Claude.ai subscribers, omit scopes so the default CLAUDE_AI_OAUTH_SCOPES
    // applies — this allows scope expansion on refresh (matching TS).
    let scopes = if is_claude_ai_auth(stored_scopes) {
        None
    } else {
        Some(stored_scopes)
    };

    let body = build_token_refresh_body(refresh_token, scopes);

    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;

    if !resp.status().is_success() {
        let err_body = resp.text().await.unwrap_or_default();
        anyhow::bail!("refresh failed: {}", err_body);
    }

    let data: serde_json::Value = resp.json().await?;
    let access_token = data["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No access_token in response"))?
        .to_string();

    // Store refreshed tokens back
    let new_tokens = super::storage::OAuthStoredTokens {
        access_token: access_token.clone(),
        refresh_token: data["refresh_token"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| Some(refresh_token.to_string())), // Keep original if not returned
        expires_at: data["expires_in"].as_u64().map(|secs| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + secs * 1000
        }),
        scopes: data["scope"]
            .as_str()
            .map(|s| s.split(' ').map(|x| x.to_string()).collect())
            .unwrap_or_default(),
        // Preserve existing subscription type on refresh (matching TS logic)
        subscription_type: None,
        rate_limit_tier: None,
    };
    let _ = super::storage::store_tokens(&new_tokens).await;

    Ok(access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_token_expired_none_means_not_expired() {
        assert!(!is_token_expired(None));
    }

    #[test]
    fn test_is_token_expired_far_future() {
        // Token expires in year 2099
        let far_future = 4_102_444_800_000u64; // ~2099
        assert!(!is_token_expired(Some(far_future)));
    }

    #[test]
    fn test_is_token_expired_past() {
        assert!(is_token_expired(Some(0)));
        assert!(is_token_expired(Some(1000)));
    }

    #[test]
    fn test_is_token_expired_within_buffer() {
        // Token expires in 3 minutes — within the 5 minute buffer — should be "expired"
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let three_min = 3 * 60 * 1000;
        assert!(is_token_expired(Some(now_ms + three_min)));
    }

    #[test]
    fn test_is_token_expired_outside_buffer() {
        // Token expires in 10 minutes — outside the 5 minute buffer — should NOT be expired
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let ten_min = 10 * 60 * 1000;
        assert!(!is_token_expired(Some(now_ms + ten_min)));
    }
}
