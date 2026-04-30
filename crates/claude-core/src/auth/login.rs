use super::pkce::*;
use super::profile::{
    fetch_and_store_user_roles, fetch_profile_info, store_oauth_account_info, OAuthProfileResponse,
};
use super::storage::{
    delete_managed_api_key, delete_tokens, store_managed_api_key, store_tokens, OAuthStoredTokens,
};
use crate::config::global::{save_global_config, AccountInfo};
use anyhow::{Context, Result};
use tokio::io::AsyncBufReadExt;

// ── OAuth constants matching the real Claude Code (src/constants/oauth.ts) ────
//
// All URL constants support env var overrides via CLAUDE_DEBUG_PROXY_BASE.
// When set, all URLs are rewritten to go through the proxy:
//   CLAUDE_DEBUG_PROXY_BASE=http://localhost:8888
//   platform.claude.com/v1/oauth/token → localhost:8888/platform/v1/oauth/token
//   api.anthropic.com/api/oauth/profile → localhost:8888/api/api/oauth/profile

/// Console (API) authorization URL.
pub const CONSOLE_AUTHORIZE_URL: &str = "https://platform.claude.com/oauth/authorize";

/// Claude.ai authorization URL — bounces through claude.com/cai/* for attribution.
pub const CLAUDE_AI_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";

/// Token exchange URL.
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";

/// API key creation endpoint (for Console users).
pub const API_KEY_URL: &str = "https://api.anthropic.com/api/oauth/claude_cli/create_api_key";

/// Success redirect for Console users.
pub const CONSOLE_SUCCESS_URL: &str =
    "https://platform.claude.com/buy_credits?returnUrl=/oauth/code/success%3Fapp%3Dclaude-code";

/// Success redirect for Claude.ai subscribers.
pub const CLAUDEAI_SUCCESS_URL: &str =
    "https://platform.claude.com/oauth/code/success?app=claude-code";

/// Manual redirect URL (for copy-paste auth code flow).
pub const MANUAL_REDIRECT_URL: &str = "https://platform.claude.com/oauth/code/callback";

/// OAuth Client ID.
pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// OAuth beta header value — required for Bearer OAuth on API routes.
pub const OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";

/// Rewrite a URL through the debug proxy if CLAUDE_DEBUG_PROXY_BASE is set.
///
/// Maps:
///   https://platform.claude.com/path → {base}/platform/path
///   https://api.anthropic.com/path   → {base}/api/path
///   https://claude.com/cai/path      → {base}/cai/path
/// Returns true when traffic is being routed through the debug proxy.
pub fn is_debug_proxy_active() -> bool {
    std::env::var("CLAUDE_DEBUG_PROXY_BASE")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Rewrite a URL through the debug proxy if CLAUDE_DEBUG_PROXY_BASE is set.
///
/// Maps:
///   https://platform.claude.com/path → {base}/platform/path
///   https://api.anthropic.com/path   → {base}/api/path
///   https://claude.com/cai/path      → {base}/cai/path
pub fn proxy_url(original: &str) -> String {
    let base = match std::env::var("CLAUDE_DEBUG_PROXY_BASE") {
        Ok(b) if !b.is_empty() => b.trim_end_matches('/').to_string(),
        _ => return original.to_string(),
    };

    if let Some(rest) = original.strip_prefix("https://platform.claude.com") {
        return format!("{}/platform{}", base, rest);
    }
    if let Some(rest) = original.strip_prefix("https://api.anthropic.com") {
        return format!("{}/api{}", base, rest);
    }
    if let Some(rest) = original.strip_prefix("https://claude.com/cai") {
        return format!("{}/cai{}", base, rest);
    }
    original.to_string()
}

/// Build a reqwest::Client that adds `x-client-tag: RS` when the debug proxy
/// is active, so the proxy can distinguish Rust traffic from TS traffic.
pub fn debug_http_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(300)); // 5 min max per request
    if is_debug_proxy_active() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-client-tag", "RS".parse().unwrap());
        builder = builder.default_headers(headers);
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

// ── OAuth scope constants matching TS (src/constants/oauth.ts) ───────────────

pub const SCOPE_USER_INFERENCE: &str = "user:inference";
pub const SCOPE_USER_PROFILE: &str = "user:profile";
pub const SCOPE_ORG_CREATE_API_KEY: &str = "org:create_api_key";

/// Console OAuth scopes (for API key creation via Console).
pub const CONSOLE_OAUTH_SCOPES: &[&str] = &[SCOPE_ORG_CREATE_API_KEY, SCOPE_USER_PROFILE];

/// Claude.ai OAuth scopes (for Claude.ai subscribers: Pro/Max/Team/Enterprise).
pub const CLAUDE_AI_OAUTH_SCOPES: &[&str] = &[
    SCOPE_USER_PROFILE,
    SCOPE_USER_INFERENCE,
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
];

/// Build the deduplicated union of all scopes (matching TS ALL_OAUTH_SCOPES).
pub fn all_oauth_scopes() -> Vec<&'static str> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for &scope in CONSOLE_OAUTH_SCOPES
        .iter()
        .chain(CLAUDE_AI_OAUTH_SCOPES.iter())
    {
        if seen.insert(scope) {
            result.push(scope);
        }
    }
    result
}

/// Check if the given scopes indicate a Claude.ai subscriber (has user:inference).
pub fn is_claude_ai_auth(scopes: &[String]) -> bool {
    scopes.iter().any(|s| s == SCOPE_USER_INFERENCE)
}

// ── Login options ────────────────────────────────────────────────────────────

/// CLI login options, matching TS `authLogin()` params in `src/cli/handlers/auth.ts`.
#[derive(Debug, Default)]
pub struct LoginOptions {
    /// Pre-populate email on the login form (--email).
    pub email: Option<String>,
    /// Force SSO login method (--sso).
    pub sso: bool,
    /// Use Console auth flow (--console).
    pub use_console: bool,
    /// Use Claude.ai auth flow (--claudeai). Default when no flags given.
    pub use_claude_ai: bool,
}

// ── Auth URL builder ─────────────────────────────────────────────────────────

/// Options for building the authorization URL.
pub struct AuthUrlOptions<'a> {
    pub code_challenge: &'a str,
    pub state: &'a str,
    pub port: u16,
    pub is_manual: bool,
    pub login_with_claude_ai: bool,
    pub org_uuid: Option<&'a str>,
    pub login_hint: Option<&'a str>,
    pub login_method: Option<&'a str>,
}

/// Build the OAuth authorization URL with PKCE parameters.
///
/// Matches the TS `buildAuthUrl()` in `src/services/oauth/client.ts`.
pub fn build_auth_url(opts: &AuthUrlOptions) -> String {
    let base = if opts.login_with_claude_ai {
        CLAUDE_AI_AUTHORIZE_URL
    } else {
        CONSOLE_AUTHORIZE_URL
    };

    let redirect_uri = if opts.is_manual {
        MANUAL_REDIRECT_URL.to_string()
    } else {
        format!("http://localhost:{}/callback", opts.port)
    };

    let scopes = all_oauth_scopes().join(" ");

    let mut url = url::Url::parse(base).expect("invalid base authorize URL");
    // code=true tells the login page to show Claude Max upsell
    url.query_pairs_mut()
        .append_pair("code", "true")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &scopes)
        .append_pair("code_challenge", opts.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", opts.state);

    if let Some(org) = opts.org_uuid {
        url.query_pairs_mut().append_pair("orgUUID", org);
    }
    if let Some(hint) = opts.login_hint {
        url.query_pairs_mut().append_pair("login_hint", hint);
    }
    if let Some(method) = opts.login_method {
        url.query_pairs_mut().append_pair("login_method", method);
    }

    url.to_string()
}

/// Build the JSON request body for the token exchange POST.
///
/// Matches the TS `exchangeCodeForTokens()` in `src/services/oauth/client.ts`.
pub fn build_token_exchange_body(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    state: &str,
) -> serde_json::Value {
    serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": redirect_uri,
        "client_id": CLIENT_ID,
        "code_verifier": code_verifier,
        "state": state,
    })
}

/// Build the JSON body for a token refresh.
///
/// For Claude.ai subscribers, uses CLAUDE_AI_OAUTH_SCOPES (allows scope expansion).
/// For Console users, uses the original scopes.
pub fn build_token_refresh_body(
    refresh_token: &str,
    scopes: Option<&[String]>,
) -> serde_json::Value {
    let scope_str = match scopes {
        Some(s) if !s.is_empty() => s.join(" "),
        _ => CLAUDE_AI_OAUTH_SCOPES.join(" "),
    };

    serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
        "scope": scope_str,
    })
}

/// Parse an HTTP request line from the callback to extract `code` and `state`
/// query parameters.
///
/// The browser sends something like:
///   GET /callback?code=AUTH_CODE&state=STATE HTTP/1.1
///
/// Returns `(code, state)` on success.
pub fn parse_callback_params(request_line: &str) -> Result<(String, String)> {
    // Extract the path+query portion from the request line
    let path = request_line
        .split_whitespace()
        .nth(1)
        .context("invalid HTTP request line")?;

    let query = path
        .split_once('?')
        .map(|(_, q)| q)
        .context("no query string in callback URL")?;

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;

    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            match key {
                "code" => code = Some(urlencoding::decode(value)?.into_owned()),
                "state" => state = Some(urlencoding::decode(value)?.into_owned()),
                _ => {}
            }
        }
    }

    let code = code.context("no 'code' parameter in callback")?;
    let state = state.context("no 'state' parameter in callback")?;
    Ok((code, state))
}

fn parse_manual_callback_input(input: &str) -> Result<(String, String)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty manual authorization input");
    }

    if let Ok(url) = url::Url::parse(trimmed) {
        let code = url
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.into_owned())
            .context("manual callback URL missing code parameter")?;
        let state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .context("manual callback URL missing state parameter")?;
        return Ok((code, state));
    }

    if let Some((code, state)) = trimmed.split_once('#') {
        if !code.is_empty() && !state.is_empty() {
            return Ok((code.to_string(), state.to_string()));
        }
    }

    anyhow::bail!("paste the full callback URL or code#state")
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Write all bytes to a tokio TcpStream, handling WouldBlock.
async fn write_all_to_stream(stream: &tokio::net::TcpStream, data: &[u8]) {
    let mut written = 0;
    while written < data.len() {
        match stream.writable().await {
            Ok(()) => match stream.try_write(&data[written..]) {
                Ok(n) => written += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => break,
            },
            Err(_) => break,
        }
    }
}

// ── Token exchange ───────────────────────────────────────────────────────────

/// Exchange an authorization code for OAuth tokens via POST to the token endpoint.
async fn exchange_code_for_tokens(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    state: &str,
) -> Result<(OAuthStoredTokens, Option<TokenAccountInfo>)> {
    let body = build_token_exchange_body(code, redirect_uri, code_verifier, state);

    let token_url = proxy_url(TOKEN_URL);
    tracing::debug!("Token exchange POST → {}", token_url);

    let client = debug_http_client();
    let resp = client
        .post(&token_url)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .context("token exchange request failed")?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        anyhow::bail!("Authentication failed: Invalid authorization code");
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Token exchange failed ({}): {}", status, text);
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .context("failed to parse token response")?;

    let access_token = data["access_token"]
        .as_str()
        .context("missing access_token in response")?
        .to_string();
    let refresh_token = data["refresh_token"].as_str().map(|s| s.to_string());
    let expires_in = data["expires_in"].as_u64().unwrap_or(3600);
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
        + expires_in * 1000;
    let scopes: Vec<String> = data["scope"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    // Extract account info from token response (fallback when profile fetch fails)
    let token_account = data["account"].as_object().map(|acc| TokenAccountInfo {
        uuid: acc
            .get("uuid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        email_address: acc
            .get("email_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        organization_uuid: data["organization"]["uuid"].as_str().map(|s| s.to_string()),
    });

    let tokens = OAuthStoredTokens {
        access_token,
        refresh_token,
        expires_at: Some(expires_at),
        scopes,
        subscription_type: None,
        rate_limit_tier: None,
    };

    Ok((tokens, token_account))
}

/// Account info from the token exchange response (fallback for profile fetch).
struct TokenAccountInfo {
    uuid: String,
    email_address: String,
    organization_uuid: Option<String>,
}

// ── Refresh token exchange (for env var fast-path) ───────────────────────────

/// Refresh an OAuth token and return full OAuthTokens with profile info.
///
/// Matches the TS `refreshOAuthToken()` in `src/services/oauth/client.ts`.
async fn refresh_for_login(
    refresh_token: &str,
    scopes: &[String],
) -> Result<(
    OAuthStoredTokens,
    Option<OAuthProfileResponse>,
    Option<TokenAccountInfo>,
)> {
    let body = build_token_refresh_body(refresh_token, Some(scopes));

    let token_url = proxy_url(TOKEN_URL);
    let client = debug_http_client();
    let resp = client
        .post(&token_url)
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .context("token refresh request failed")?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Token refresh failed: {}", text);
    }

    let data: serde_json::Value = resp.json().await?;
    let access_token = data["access_token"]
        .as_str()
        .context("No access_token in refresh response")?
        .to_string();
    let new_refresh = data["refresh_token"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| refresh_token.to_string());
    let expires_in = data["expires_in"].as_u64().unwrap_or(3600);
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
        + expires_in * 1000;
    let new_scopes: Vec<String> = data["scope"]
        .as_str()
        .map(|s| s.split(' ').map(|x| x.to_string()).collect())
        .unwrap_or_default();

    // Fetch profile info
    let profile_info = fetch_profile_info(&access_token).await;

    let token_account = data["account"].as_object().map(|acc| TokenAccountInfo {
        uuid: acc
            .get("uuid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        email_address: acc
            .get("email_address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        organization_uuid: data["organization"]["uuid"].as_str().map(|s| s.to_string()),
    });

    let tokens = OAuthStoredTokens {
        access_token,
        refresh_token: Some(new_refresh),
        expires_at: Some(expires_at),
        scopes: new_scopes,
        subscription_type: profile_info.subscription_type,
        rate_limit_tier: profile_info.rate_limit_tier,
    };

    Ok((tokens, profile_info.raw_profile, token_account))
}

// ── Post-login setup (installOAuthTokens) ────────────────────────────────────

/// Shared post-token-acquisition logic. Saves tokens, fetches profile/roles,
/// and sets up the local auth state.
///
/// Matches TS `installOAuthTokens()` in `src/cli/handlers/auth.ts`.
async fn install_oauth_tokens(
    tokens: &OAuthStoredTokens,
    profile: Option<&OAuthProfileResponse>,
    token_account: Option<&TokenAccountInfo>,
) -> Result<()> {
    // 1. Clear old state (matching TS performLogout with clearOnboarding=false)
    perform_logout().await;

    // 2. Store account info in global config
    if let Some(p) = profile {
        store_oauth_account_info(AccountInfo {
            account_uuid: p.account.uuid.clone(),
            email_address: p.account.email.clone(),
            organization_uuid: Some(p.organization.uuid.clone()),
            display_name: p.account.display_name.clone(),
            has_extra_usage_enabled: p.organization.has_extra_usage_enabled,
            billing_type: p.organization.billing_type.clone(),
            account_created_at: p.account.created_at.clone(),
            subscription_created_at: p.organization.subscription_created_at.clone(),
            ..Default::default()
        })?;
    } else if let Some(ta) = token_account {
        // Fallback to token exchange account data when profile endpoint fails
        store_oauth_account_info(AccountInfo {
            account_uuid: ta.uuid.clone(),
            email_address: ta.email_address.clone(),
            organization_uuid: ta.organization_uuid.clone(),
            ..Default::default()
        })?;
    }

    // 3. Save OAuth tokens to secure storage.
    //    Matches TS `saveOAuthTokensIfNeeded()` in `src/utils/auth.ts`:
    //    - Skip for non-Claude.ai auth (Console users don't persist OAuth tokens)
    //    - Skip for inference-only tokens (no refresh token / no expiry)
    if is_claude_ai_auth(&tokens.scopes)
        && tokens.refresh_token.is_some()
        && tokens.expires_at.is_some()
    {
        store_tokens(tokens).await?;
        // Match TS post-login trusted-device flow: clear any stale device token
        // from a previous account, then enroll this fresh login session.
        crate::bridge::trusted_device::clear_trusted_device_token().await;
        crate::bridge::trusted_device::enroll_trusted_device(&tokens.access_token).await;
    }

    // 4. Fetch and store user roles (non-critical, log errors)
    if let Err(e) = fetch_and_store_user_roles(&tokens.access_token).await {
        tracing::debug!("Failed to fetch user roles: {}", e);
    }

    // 5. For Claude.ai subscribers, fetch the first-token date (non-critical).
    //    Matches TS `fetchAndStoreClaudeCodeFirstTokenDate()` in
    //    `src/services/api/firstTokenDate.ts`.
    if is_claude_ai_auth(&tokens.scopes) {
        if let Err(e) = fetch_and_store_first_token_date(&tokens.access_token).await {
            tracing::debug!("Failed to fetch first token date: {}", e);
        }
    }

    // 6. For Console users (no user:inference scope), create an API key
    if !is_claude_ai_auth(&tokens.scopes) {
        tracing::info!("Console login detected — creating API key");
        match create_and_store_api_key(&tokens.access_token).await {
            Ok(Some(key)) => {
                store_managed_api_key(&key).await?;
            }
            Ok(None) => {
                anyhow::bail!(
                    "Unable to create API key. The server accepted the request but did not return a key."
                );
            }
            Err(e) => {
                tracing::warn!("Failed to create API key: {}", e);
            }
        }
    }

    Ok(())
}

/// Clear existing auth state before login.
///
/// Matches TS `performLogout({ clearOnboarding: false })`.
async fn perform_logout() {
    // Delete stored tokens (keychain + file)
    if let Err(e) = delete_tokens().await {
        tracing::debug!("Failed to delete tokens during logout: {}", e);
    }

    if let Err(e) = delete_managed_api_key().await {
        tracing::debug!("Failed to delete managed API key during logout: {}", e);
    }

    // Clear oauthAccount from global config (but preserve onboarding state)
    save_global_config(|mut config| {
        config.oauth_account = None;
        config
    })
    .ok();
}

/// Fetch and store the organization's first Claude Code token date.
///
/// Matches TS `fetchAndStoreClaudeCodeFirstTokenDate()` in
/// `src/services/api/firstTokenDate.ts`.
///
/// - Early-returns if the date is already cached in global config.
/// - Stores `null` (as `None`) if the API returns no date.
/// - Validates the date string before saving.
async fn fetch_and_store_first_token_date(access_token: &str) -> Result<()> {
    // Check if already cached
    let config = crate::config::global::load_global_config()?;
    if config.extra.contains_key("claudeCodeFirstTokenDate") {
        return Ok(());
    }

    let url = proxy_url("https://api.anthropic.com/api/organization/claude_code_first_token_date");
    let client = debug_http_client();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("anthropic-beta", OAUTH_BETA_HEADER)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("failed to fetch first token date")?;

    if !resp.status().is_success() {
        anyhow::bail!("first token date endpoint returned {}", resp.status());
    }

    let data: serde_json::Value = resp.json().await?;
    let first_token_date = data
        .get("first_token_date")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // Validate date string if not null (matching TS validation)
    if let Some(date_str) = first_token_date.as_str() {
        if chrono::DateTime::parse_from_rfc3339(date_str).is_err()
            && chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").is_err()
        {
            anyhow::bail!("invalid first_token_date from API: {}", date_str);
        }
    }

    save_global_config(|mut config| {
        config
            .extra
            .insert("claudeCodeFirstTokenDate".to_string(), first_token_date);
        config
    })?;

    Ok(())
}

/// Create an API key via the Console OAuth endpoint (for Console users).
///
/// Matches the TS `createAndStoreApiKey()` in `src/services/oauth/client.ts`.
async fn create_and_store_api_key(access_token: &str) -> Result<Option<String>> {
    let api_key_url = proxy_url(API_KEY_URL);
    let client = debug_http_client();
    let resp = client
        .post(&api_key_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .context("API key creation request failed")?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("API key creation failed: {}", text);
    }

    let data: serde_json::Value = resp.json().await?;
    Ok(data["raw_key"].as_str().map(|s| s.to_string()))
}

// ── Main login flow ──────────────────────────────────────────────────────────

/// Default login: Claude.ai flow.
pub async fn login() -> Result<()> {
    login_with_options(LoginOptions {
        use_claude_ai: true,
        ..Default::default()
    })
    .await
}

/// Full login flow with all options.
///
/// Matches TS `authLogin()` in `src/cli/handlers/auth.ts`.
pub async fn login_with_options(opts: LoginOptions) -> Result<()> {
    if opts.use_console && opts.use_claude_ai {
        anyhow::bail!("--console and --claudeai cannot be used together.");
    }

    // Resolve login_with_claude_ai: --console=false means Claude.ai, default is Claude.ai
    let login_with_claude_ai = !opts.use_console;

    // ── Fast path: env var refresh token (CI/CD) ──────────────────────────
    if let Ok(env_refresh_token) = std::env::var("CLAUDE_CODE_OAUTH_REFRESH_TOKEN") {
        if !env_refresh_token.is_empty() {
            return login_from_refresh_token(&env_refresh_token, login_with_claude_ai).await;
        }
    }

    // ── Browser OAuth flow ────────────────────────────────────────────────
    let login_method = if opts.sso { Some("sso") } else { None };

    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let state = generate_state();

    // Start local callback server on a random port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://localhost:{}/callback", port);

    // Build auth URLs — automatic (localhost redirect) and manual (copy-paste)
    let auto_url = build_auth_url(&AuthUrlOptions {
        code_challenge: &challenge,
        state: &state,
        port,
        is_manual: false,
        login_with_claude_ai,
        org_uuid: None,
        login_hint: opts.email.as_deref(),
        login_method,
    });

    let manual_url = build_auth_url(&AuthUrlOptions {
        code_challenge: &challenge,
        state: &state,
        port,
        is_manual: true,
        login_with_claude_ai,
        org_uuid: None,
        login_hint: opts.email.as_deref(),
        login_method,
    });

    println!("Opening browser to sign in...");
    println!("If the browser didn't open, visit: {}", manual_url);
    let _ = open::that(&auto_url);

    println!("If automatic login does not complete, paste the full callback URL or `code#state` and press Enter.");

    let (code, mut pending_stream) = await_authorization_code(&listener, &state).await?;

    // When manual input was used (no pending_stream), the redirect_uri must
    // match what was in the authorize URL — MANUAL_REDIRECT_URL, not localhost.
    // Matches TS `exchangeCodeForTokens(code, state, verifier, port, !isAutomaticFlow)`.
    let effective_redirect = if pending_stream.is_some() {
        &redirect_uri
    } else {
        MANUAL_REDIRECT_URL
    };

    // Exchange authorization code for tokens
    let (mut tokens, token_account) =
        exchange_code_for_tokens(&code, effective_redirect, &verifier, &state).await?;

    // Fetch profile info (subscription type, rate limit tier, etc.)
    let profile_info = fetch_profile_info(&tokens.access_token).await;
    tokens.subscription_type = profile_info.subscription_type;
    tokens.rate_limit_tier = profile_info.rate_limit_tier;

    // Determine success redirect based on scopes (matching TS handleSuccessRedirect)
    let success_url = if is_claude_ai_auth(&tokens.scopes) {
        CLAUDEAI_SUCCESS_URL
    } else {
        CONSOLE_SUCCESS_URL
    };
    if let Some(stream) = pending_stream.take() {
        let response = format!("HTTP/1.1 302 Found\r\nLocation: {}\r\n\r\n", success_url);
        write_all_to_stream(&stream, response.as_bytes()).await;
        drop(stream);
    }

    // Post-login setup: store account info, roles, API key
    install_oauth_tokens(
        &tokens,
        profile_info.raw_profile.as_ref(),
        token_account.as_ref(),
    )
    .await?;

    // Mark onboarding complete
    save_global_config(|mut config| {
        if config.has_completed_onboarding != Some(true) {
            config.has_completed_onboarding = Some(true);
        }
        config
    })
    .ok();

    println!("Login successful!");
    Ok(())
}

async fn await_authorization_code(
    listener: &tokio::net::TcpListener,
    expected_state: &str,
) -> Result<(String, Option<tokio::net::TcpStream>)> {
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut line = String::new();

    loop {
        line.clear();

        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let mut buf = vec![0u8; 4096];
                let n = loop {
                    stream.readable().await?;
                    match stream.try_read(&mut buf) {
                        Ok(n) => break n,
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(e) => anyhow::bail!("failed to read callback request: {}", e),
                    }
                };

                if n == 0 {
                    anyhow::bail!("empty callback request from browser");
                }

                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let request_line = request
                    .lines()
                    .next()
                    .context("empty HTTP request from callback")?;

                let (code, received_state) =
                    parse_callback_params(request_line).context("failed to parse OAuth callback")?;

                if received_state != expected_state {
                    let response =
                        b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\nInvalid state parameter";
                    write_all_to_stream(&stream, response).await;
                    anyhow::bail!("OAuth state mismatch: possible CSRF attack");
                }

                return Ok((code, Some(stream)));
            }
            read = stdin.read_line(&mut line) => {
                let read = read?;
                if read == 0 {
                    continue;
                }

                match parse_manual_callback_input(&line) {
                    Ok((code, received_state)) => {
                        if received_state != expected_state {
                            eprintln!("Invalid state in pasted callback. Try again.");
                            continue;
                        }
                        return Ok((code, None));
                    }
                    Err(err) => {
                        eprintln!("Manual auth input error: {}.", err);
                        continue;
                    }
                }
            }
        }
    }
}

/// Fast-path login from a refresh token env var (CI/CD).
///
/// Matches the TS env var check in `authLogin()` lines 140-186.
async fn login_from_refresh_token(refresh_token: &str, _login_with_claude_ai: bool) -> Result<()> {
    let env_scopes = std::env::var("CLAUDE_CODE_OAUTH_SCOPES")
        .map_err(|_| anyhow::anyhow!(
            "CLAUDE_CODE_OAUTH_SCOPES is required when using CLAUDE_CODE_OAUTH_REFRESH_TOKEN.\n\
             Set it to the space-separated scopes the refresh token was issued with\n\
             (e.g. \"user:inference\" or \"user:profile user:inference user:sessions:claude_code user:mcp_servers\")."
        ))?;

    let scopes: Vec<String> = env_scopes
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    let (tokens, profile, token_account) = refresh_for_login(refresh_token, &scopes).await?;

    install_oauth_tokens(&tokens, profile.as_ref(), token_account.as_ref()).await?;

    // Mark onboarding complete (env var path skips interactive onboarding)
    save_global_config(|mut config| {
        if config.has_completed_onboarding != Some(true) {
            config.has_completed_onboarding = Some(true);
        }
        config
    })
    .ok();

    println!("Login successful.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Auth URL tests ───────────────────────────────────────────────────

    #[test]
    fn test_build_auth_url_console_flow() {
        let url = build_auth_url(&AuthUrlOptions {
            code_challenge: "test-challenge",
            state: "test-state",
            port: 12345,
            is_manual: false,
            login_with_claude_ai: false,
            org_uuid: None,
            login_hint: None,
            login_method: None,
        });

        // Console flow uses platform.claude.com
        assert!(
            url.starts_with(CONSOLE_AUTHORIZE_URL),
            "URL should start with Console URL, got: {}",
            url
        );
        assert!(
            url.contains("code=true"),
            "URL must include code=true param"
        );
        assert!(
            url.contains(&format!("client_id={}", CLIENT_ID)),
            "URL must include client_id"
        );
        assert!(
            url.contains("response_type=code"),
            "URL must include response_type=code"
        );
        assert!(
            url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A12345%2Fcallback"),
            "URL must include localhost redirect_uri, got: {}",
            url
        );
        assert!(url.contains("code_challenge=test-challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=test-state"));
    }

    #[test]
    fn test_build_auth_url_claudeai_flow() {
        let url = build_auth_url(&AuthUrlOptions {
            code_challenge: "test-challenge",
            state: "test-state",
            port: 12345,
            is_manual: false,
            login_with_claude_ai: true,
            org_uuid: None,
            login_hint: None,
            login_method: None,
        });

        // Claude.ai flow uses claude.com/cai
        assert!(
            url.starts_with(CLAUDE_AI_AUTHORIZE_URL),
            "URL should start with Claude.ai URL, got: {}",
            url
        );
        assert!(url.contains("code=true"));
    }

    #[test]
    fn test_build_auth_url_manual_flow_uses_manual_redirect() {
        let url = build_auth_url(&AuthUrlOptions {
            code_challenge: "ch",
            state: "st",
            port: 9999,
            is_manual: true,
            login_with_claude_ai: false,
            org_uuid: None,
            login_hint: None,
            login_method: None,
        });

        // Manual flow redirect URI should be the platform callback URL, not localhost
        let encoded_manual = urlencoding::encode(MANUAL_REDIRECT_URL);
        assert!(
            url.contains(&format!("redirect_uri={}", encoded_manual)),
            "Manual flow must use MANUAL_REDIRECT_URL, got: {}",
            url
        );
    }

    #[test]
    fn test_build_auth_url_scopes_match_ts() {
        let url = build_auth_url(&AuthUrlOptions {
            code_challenge: "ch",
            state: "st",
            port: 8080,
            is_manual: false,
            login_with_claude_ai: false,
            org_uuid: None,
            login_hint: None,
            login_method: None,
        });

        let parsed = url::Url::parse(&url).unwrap();
        let scope_param = parsed
            .query_pairs()
            .find(|(k, _)| k == "scope")
            .map(|(_, v)| v.to_string())
            .unwrap();
        let url_scopes: Vec<&str> = scope_param.split(' ').collect();
        let expected_scopes = all_oauth_scopes();
        for scope in &expected_scopes {
            assert!(
                url_scopes.contains(scope),
                "URL must contain scope '{}', got scopes: {:?}",
                scope,
                url_scopes
            );
        }
        assert!(
            url_scopes.contains(&"org:create_api_key"),
            "Must contain org:create_api_key"
        );
        assert!(
            !url_scopes.contains(&"org:service_key_inference"),
            "Must NOT contain org:service_key_inference"
        );
    }

    #[test]
    fn test_build_auth_url_optional_params() {
        let url = build_auth_url(&AuthUrlOptions {
            code_challenge: "ch",
            state: "st",
            port: 8080,
            is_manual: false,
            login_with_claude_ai: false,
            org_uuid: Some("org-123"),
            login_hint: Some("user@example.com"),
            login_method: Some("sso"),
        });

        assert!(url.contains("orgUUID=org-123"), "URL must contain orgUUID");
        assert!(
            url.contains("login_hint=user%40example.com"),
            "URL must contain login_hint"
        );
        assert!(
            url.contains("login_method=sso"),
            "URL must contain login_method"
        );
    }

    // ── Scope helpers ────────────────────────────────────────────────────

    #[test]
    fn test_all_oauth_scopes_matches_ts() {
        let scopes = all_oauth_scopes();
        assert!(scopes.contains(&"org:create_api_key"));
        assert!(scopes.contains(&"user:profile"));
        assert!(scopes.contains(&"user:inference"));
        assert!(scopes.contains(&"user:sessions:claude_code"));
        assert!(scopes.contains(&"user:mcp_servers"));
        assert!(scopes.contains(&"user:file_upload"));
        assert_eq!(scopes.len(), 6, "Should have exactly 6 deduplicated scopes");
    }

    #[test]
    fn test_is_claude_ai_auth() {
        assert!(is_claude_ai_auth(&[
            "user:profile".to_string(),
            "user:inference".to_string(),
        ]));
        assert!(!is_claude_ai_auth(&[
            "org:create_api_key".to_string(),
            "user:profile".to_string(),
        ]));
        assert!(!is_claude_ai_auth(&[]));
    }

    // ── Token exchange body tests ────────────────────────────────────────

    #[test]
    fn test_build_token_exchange_body() {
        let body = build_token_exchange_body(
            "auth-code-123",
            "http://localhost:9999/callback",
            "verifier-xyz",
            "state-abc",
        );

        assert_eq!(body["grant_type"], "authorization_code");
        assert_eq!(body["code"], "auth-code-123");
        assert_eq!(body["redirect_uri"], "http://localhost:9999/callback");
        assert_eq!(body["client_id"], CLIENT_ID);
        assert_eq!(body["code_verifier"], "verifier-xyz");
        assert_eq!(body["state"], "state-abc");
    }

    #[test]
    fn test_build_token_refresh_body_default_scopes() {
        let body = build_token_refresh_body("rt_abc", None);

        assert_eq!(body["grant_type"], "refresh_token");
        assert_eq!(body["refresh_token"], "rt_abc");
        assert_eq!(body["client_id"], CLIENT_ID);
        let scope = body["scope"].as_str().unwrap();
        assert!(
            scope.contains("user:inference"),
            "Default refresh should use Claude AI scopes"
        );
        assert!(scope.contains("user:profile"));
    }

    #[test]
    fn test_build_token_refresh_body_custom_scopes() {
        let scopes = vec!["org:create_api_key".to_string(), "user:profile".to_string()];
        let body = build_token_refresh_body("rt_abc", Some(&scopes));

        let scope = body["scope"].as_str().unwrap();
        assert_eq!(scope, "org:create_api_key user:profile");
    }

    // ── Callback parsing tests ───────────────────────────────────────────

    #[test]
    fn test_parse_callback_params_valid() {
        let line = "GET /callback?code=abc123&state=xyz789 HTTP/1.1";
        let (code, state) = parse_callback_params(line).unwrap();
        assert_eq!(code, "abc123");
        assert_eq!(state, "xyz789");
    }

    #[test]
    fn test_parse_callback_params_url_encoded() {
        let line = "GET /callback?code=abc%20123&state=xyz%3D789 HTTP/1.1";
        let (code, state) = parse_callback_params(line).unwrap();
        assert_eq!(code, "abc 123");
        assert_eq!(state, "xyz=789");
    }

    #[test]
    fn test_parse_callback_params_missing_code() {
        let line = "GET /callback?state=xyz789 HTTP/1.1";
        assert!(parse_callback_params(line).is_err());
    }

    #[test]
    fn test_parse_callback_params_missing_query() {
        let line = "GET /callback HTTP/1.1";
        assert!(parse_callback_params(line).is_err());
    }

    #[test]
    fn test_parse_callback_params_invalid_request() {
        let line = "INVALID";
        assert!(parse_callback_params(line).is_err());
    }

    // ── Constants tests ──────────────────────────────────────────────────

    #[test]
    fn test_constants_match_ts() {
        assert_eq!(CLIENT_ID, "9d1c250a-e61b-44d9-88ed-5944d1962f5e");
        assert_eq!(
            CONSOLE_AUTHORIZE_URL,
            "https://platform.claude.com/oauth/authorize"
        );
        assert_eq!(
            CLAUDE_AI_AUTHORIZE_URL,
            "https://claude.com/cai/oauth/authorize"
        );
        assert_eq!(TOKEN_URL, "https://platform.claude.com/v1/oauth/token");
        assert_eq!(
            API_KEY_URL,
            "https://api.anthropic.com/api/oauth/claude_cli/create_api_key"
        );
        assert_eq!(
            MANUAL_REDIRECT_URL,
            "https://platform.claude.com/oauth/code/callback"
        );
        assert_eq!(OAUTH_BETA_HEADER, "oauth-2025-04-20");
    }

    // ── LoginOptions tests ───────────────────────────────────────────────

    #[test]
    fn test_login_options_default_is_claude_ai() {
        let opts = LoginOptions::default();
        assert!(!opts.use_console);
        assert!(!opts.use_claude_ai); // Default struct has false, login() sets it
        assert!(!opts.sso);
        assert!(opts.email.is_none());
    }
}
