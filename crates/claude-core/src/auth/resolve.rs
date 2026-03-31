use anyhow::Result;
use crate::api::client::AuthMethod;

/// Resolve authentication.
/// For OAuth: refreshes the token to get a fresh access token.
pub async fn resolve_auth() -> Result<AuthMethod> {
    // 1. Check env var (direct API key)
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return Ok(AuthMethod::ApiKey(key));
        }
    }

    // 2. Read stored OAuth tokens from keychain and refresh
    if let Some(tokens) = super::storage::load_tokens().await? {
        if let Some(refresh_token) = &tokens.refresh_token {
            match refresh_oauth_token(refresh_token).await {
                Ok(fresh_token) => return Ok(AuthMethod::OAuthToken(fresh_token)),
                Err(e) => {
                    tracing::warn!("Token refresh failed ({}), using stored token", e);
                    return Ok(AuthMethod::OAuthToken(tokens.access_token));
                }
            }
        }
        return Ok(AuthMethod::OAuthToken(tokens.access_token));
    }

    anyhow::bail!("No authentication found. Run `claude login` or set ANTHROPIC_API_KEY")
}

const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

async fn refresh_oauth_token(refresh_token: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
            "scope": "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload",
        }))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("refresh failed: {}", body);
    }

    let data: serde_json::Value = resp.json().await?;
    let access_token = data["access_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No access_token in response"))?
        .to_string();

    // Store refreshed tokens back to keychain
    let new_tokens = super::storage::OAuthStoredTokens {
        access_token: access_token.clone(),
        refresh_token: data["refresh_token"].as_str().map(|s| s.to_string()),
        expires_at: data["expires_in"]
            .as_u64()
            .map(|secs| {
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
        subscription_type: None,
        rate_limit_tier: None,
    };
    let _ = super::storage::store_tokens(&new_tokens).await;

    Ok(access_token)
}
