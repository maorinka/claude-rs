use anyhow::{Context, Result};
use super::pkce::*;
use super::storage::{OAuthStoredTokens, store_tokens};

const AUTHORIZE_URL: &str = "https://platform.claude.com/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const SCOPES: &str = "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

/// Build the OAuth authorization URL with PKCE parameters.
pub fn build_auth_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &str,
    code_challenge: &str,
    state: &str,
) -> String {
    format!(
        "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        AUTHORIZE_URL,
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(scopes),
        code_challenge,
        state,
    )
}

/// Build the JSON request body for the token exchange POST.
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

/// Exchange an authorization code for OAuth tokens via POST to the token endpoint.
async fn exchange_code_for_tokens(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    state: &str,
) -> Result<OAuthStoredTokens> {
    let body = build_token_exchange_body(code, redirect_uri, code_verifier, state);

    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
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

    let data: serde_json::Value = resp.json().await.context("failed to parse token response")?;

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
    let scopes = data["scope"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    Ok(OAuthStoredTokens {
        access_token,
        refresh_token,
        expires_at: Some(expires_at),
        scopes,
        subscription_type: None,
        rate_limit_tier: None,
    })
}

/// Run the full OAuth login flow:
/// 1. Start an HTTP server on a random localhost port
/// 2. Build the auth URL with PKCE params
/// 3. Open the browser
/// 4. Wait for the callback and extract `code` + `state`
/// 5. Exchange code for tokens
/// 6. Store tokens
pub async fn login() -> Result<()> {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let state = generate_state();

    // Start local callback server on a random port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://localhost:{}/callback", port);

    // Build auth URL
    let auth_url = build_auth_url(CLIENT_ID, &redirect_uri, SCOPES, &challenge, &state);

    println!("Opening browser for authentication...");
    println!("If the browser doesn't open, visit: {}", auth_url);
    let _ = open::that(&auth_url);

    // Wait for the callback request
    let (stream, _) = listener.accept().await?;
    stream.readable().await?;

    // Read the HTTP request from the callback
    let mut buf = vec![0u8; 4096];
    let n = stream.try_read(&mut buf).unwrap_or(0);
    let request = String::from_utf8_lossy(&buf[..n]).to_string();

    // Extract the first line (request line)
    let request_line = request
        .lines()
        .next()
        .context("empty HTTP request from callback")?;

    // Parse code and state from the callback URL
    let (code, received_state) =
        parse_callback_params(request_line).context("failed to parse OAuth callback")?;

    // Validate state parameter (CSRF protection)
    if received_state != state {
        // Send error response to browser
        let response = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\nInvalid state parameter";
        let _ = stream.try_write(response.as_bytes());
        anyhow::bail!("OAuth state mismatch: possible CSRF attack");
    }

    // Send success response to browser before exchanging tokens
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h2>Authentication successful!</h2><p>You can close this tab and return to the terminal.</p></body></html>";
    let _ = stream.try_write(response.as_bytes());
    drop(stream);

    // Exchange authorization code for tokens
    println!("Exchanging authorization code for tokens...");
    let tokens = exchange_code_for_tokens(&code, &redirect_uri, &verifier, &state).await?;

    // Store tokens
    store_tokens(&tokens).await?;

    println!("Login successful!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_auth_url_contains_required_params() {
        let url = build_auth_url(
            "test-client-id",
            "http://localhost:12345/callback",
            "scope1 scope2",
            "test-challenge",
            "test-state",
        );

        assert!(url.starts_with(AUTHORIZE_URL));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("localhost%3A12345"));
        assert!(url.contains("scope=scope1%20scope2"));
        assert!(url.contains("code_challenge=test-challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=test-state"));
    }

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
}
