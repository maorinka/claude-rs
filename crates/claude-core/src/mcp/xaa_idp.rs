//! XAA IdP settings and cache helpers.
//!
//! This ports the general storage/keying behavior from TS
//! `services/mcp/xaaIdpLogin.ts`: a user-level `settings.xaaIdp` config plus
//! issuer-keyed secure-storage maps for cached id_tokens and client secrets.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use rand::Rng;
use serde::Deserialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use url::Url;

const ID_TOKEN_EXPIRY_BUFFER_S: u64 = 60;
const IDP_LOGIN_TIMEOUT_MS: u64 = 5 * 60 * 1000;
const IDP_REQUEST_TIMEOUT_MS: u64 = 30_000;
const REDIRECT_PORT_FALLBACK: u16 = 3118;

pub use crate::config::settings::XaaIdpSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XaaSetupInput {
    pub issuer: String,
    pub client_id: String,
    pub callback_port: Option<u32>,
}

pub struct IdpLoginOptions<'a> {
    pub idp_issuer: &'a str,
    pub idp_client_id: &'a str,
    pub idp_client_secret: Option<&'a str>,
    pub callback_port: Option<u32>,
    pub on_authorization_url: Option<&'a dyn Fn(&str)>,
    pub skip_browser_open: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenIdProviderDiscoveryMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct IdpTokenResponse {
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

pub fn is_xaa_enabled() -> bool {
    crate::errors_util::is_env_truthy("CLAUDE_CODE_ENABLE_XAA")
}

/// Normalize an IdP issuer URL for use as a cache key.
///
/// Matches TS `issuerKey`: strip trailing path slashes, lowercase host, and
/// fall back to slash stripping for non-URL input.
pub fn issuer_key(issuer: &str) -> String {
    match Url::parse(issuer) {
        Ok(mut url) => {
            let trimmed_path = url.path().trim_end_matches('/').to_string();
            url.set_path(&trimmed_path);
            if let Some(host) = url.host_str() {
                let _ = url.set_host(Some(&host.to_lowercase()));
            }
            url.to_string()
        }
        Err(_) => issuer.trim_end_matches('/').to_string(),
    }
}

pub fn validate_setup_input(
    issuer: &str,
    client_id: &str,
    callback_port: Option<u32>,
) -> Result<XaaSetupInput> {
    let issuer_url = Url::parse(issuer)
        .map_err(|_| anyhow!("--issuer must be a valid URL (got \"{}\")", issuer))?;
    if issuer_url.scheme() != "https" && !is_loopback_http_url(&issuer_url) {
        let host = issuer_url.host_str().unwrap_or_default();
        let port = issuer_url
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        return Err(anyhow!(
            "--issuer must use https:// (got \"{}://{}{}\")",
            issuer_url.scheme(),
            host,
            port
        ));
    }
    if client_id.is_empty() {
        return Err(anyhow!("--client-id is required"));
    }
    Ok(XaaSetupInput {
        issuer: issuer.to_string(),
        client_id: client_id.to_string(),
        callback_port,
    })
}

fn is_loopback_http_url(url: &Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }
    matches!(
        url.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("[::1]") | Some("::1")
    )
}

pub fn jwt_exp(jwt: &str) -> Option<u64> {
    let mut parts = jwt.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let _signature = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.get("exp")?.as_u64()
}

pub async fn get_cached_idp_id_token(idp_issuer: &str) -> Result<Option<String>> {
    get_cached_idp_id_token_at(idp_issuer, now_ms()).await
}

async fn get_cached_idp_id_token_at(idp_issuer: &str, now_ms: u64) -> Result<Option<String>> {
    let key = issuer_key(idp_issuer);
    let Some(entry) = crate::auth::storage::load_xaa_idp_token_entry(&key).await? else {
        return Ok(None);
    };
    if entry.expires_at <= now_ms + ID_TOKEN_EXPIRY_BUFFER_S * 1000 {
        return Ok(None);
    }
    Ok(Some(entry.id_token))
}

pub async fn save_idp_id_token_from_jwt(idp_issuer: &str, id_token: &str) -> Result<u64> {
    let expires_at = jwt_exp(id_token)
        .map(|exp| exp * 1000)
        .unwrap_or_else(|| now_ms() + 3600 * 1000);
    save_idp_id_token(idp_issuer, id_token, expires_at).await?;
    Ok(expires_at)
}

pub async fn save_idp_id_token(idp_issuer: &str, id_token: &str, expires_at: u64) -> Result<()> {
    let key = issuer_key(idp_issuer);
    crate::auth::storage::store_xaa_idp_token_entry(
        &key,
        crate::auth::storage::XaaIdpTokenEntry {
            id_token: id_token.to_string(),
            expires_at,
        },
    )
    .await
}

pub async fn clear_idp_id_token(idp_issuer: &str) -> Result<()> {
    crate::auth::storage::clear_xaa_idp_token_entry(&issuer_key(idp_issuer)).await
}

pub async fn get_idp_client_secret(idp_issuer: &str) -> Result<Option<String>> {
    crate::auth::storage::load_xaa_idp_client_secret(&issuer_key(idp_issuer)).await
}

pub async fn save_idp_client_secret(idp_issuer: &str, client_secret: &str) -> Result<()> {
    crate::auth::storage::store_xaa_idp_client_secret(&issuer_key(idp_issuer), client_secret).await
}

pub async fn clear_idp_client_secret(idp_issuer: &str) -> Result<()> {
    crate::auth::storage::clear_xaa_idp_client_secret(&issuer_key(idp_issuer)).await
}

/// OIDC Discovery §4.1 appends `.well-known/openid-configuration` to the
/// issuer path. This intentionally does not replace the path, matching TS and
/// preserving Azure AD/Okta/Keycloak realm issuers.
pub async fn discover_oidc(idp_issuer: &str) -> Result<OpenIdProviderDiscoveryMetadata> {
    let base = if idp_issuer.ends_with('/') {
        idp_issuer.to_string()
    } else {
        format!("{idp_issuer}/")
    };
    let url = Url::parse(&base)
        .and_then(|base| base.join(".well-known/openid-configuration"))
        .map_err(|err| anyhow!("XAA IdP: invalid issuer URL: {err}"))?;

    let client = reqwest::Client::new();
    let res = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(std::time::Duration::from_millis(IDP_REQUEST_TIMEOUT_MS))
        .send()
        .await
        .map_err(|err| anyhow!("XAA IdP: OIDC discovery failed: {err}"))?;
    let status = res.status();
    if !status.is_success() {
        return Err(anyhow!(
            "XAA IdP: OIDC discovery failed: HTTP {} at {}",
            status.as_u16(),
            url
        ));
    }
    let body = res.json::<Value>().await.map_err(|_| {
        anyhow!("XAA IdP: OIDC discovery returned non-JSON at {url} (captive portal or proxy?)")
    })?;
    let metadata: OpenIdProviderDiscoveryMetadata = serde_json::from_value(body)
        .map_err(|err| anyhow!("XAA IdP: invalid OIDC metadata: {err}"))?;
    if Url::parse(&metadata.token_endpoint)
        .map(|url| url.scheme() != "https")
        .unwrap_or(true)
    {
        return Err(anyhow!(
            "XAA IdP: refusing non-HTTPS token endpoint: {}",
            metadata.token_endpoint
        ));
    }
    Ok(metadata)
}

pub async fn acquire_idp_id_token(opts: IdpLoginOptions<'_>) -> Result<String> {
    if let Some(cached) = get_cached_idp_id_token(opts.idp_issuer).await? {
        tracing::debug!("[xaa] Using cached id_token for {}", opts.idp_issuer);
        return Ok(cached);
    }

    tracing::debug!(
        "[xaa] No cached id_token for {}; starting OIDC login",
        opts.idp_issuer
    );
    let metadata = discover_oidc(opts.idp_issuer).await?;
    let port = match opts.callback_port {
        Some(port) => u16::try_from(port).map_err(|_| {
            anyhow!("XAA IdP: callback port {port} is outside the valid TCP port range")
        })?,
        None => find_available_port().await?,
    };
    let redirect_uri = build_redirect_uri(port);
    let state = crate::auth::pkce::generate_state();
    let code_verifier = crate::auth::pkce::generate_code_verifier();
    let code_challenge = crate::auth::pkce::generate_code_challenge(&code_verifier);
    let authorization_url = build_authorization_url(
        &metadata,
        opts.idp_client_id,
        &redirect_uri,
        &state,
        &code_challenge,
    )?;

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::AddrInUse {
                anyhow!(
                    "XAA IdP: callback port {} is already in use. Run `lsof -ti:{} -sTCP:LISTEN` to find the holder.",
                    port,
                    port
                )
            } else {
                anyhow!("XAA IdP: callback server failed: {err}")
            }
        })?;

    if let Some(callback) = opts.on_authorization_url {
        callback(authorization_url.as_str());
    }
    if !opts.skip_browser_open {
        tracing::debug!("[xaa] Opening browser to IdP authorization endpoint");
        let _ = crate::browser::open_browser(authorization_url.as_str());
    }

    let code = tokio::time::timeout(
        std::time::Duration::from_millis(IDP_LOGIN_TIMEOUT_MS),
        wait_for_callback(&listener, &state),
    )
    .await
    .map_err(|_| anyhow!("XAA IdP: login timed out"))??;

    let tokens = exchange_authorization_code(
        &metadata,
        opts.idp_client_id,
        opts.idp_client_secret,
        &code,
        &code_verifier,
        &redirect_uri,
    )
    .await?;
    let id_token = tokens
        .id_token
        .ok_or_else(|| anyhow!("XAA IdP: token response missing id_token (check scope=openid)"))?;
    let expires_at = jwt_exp(&id_token)
        .map(|exp| exp * 1000)
        .unwrap_or_else(|| now_ms() + tokens.expires_in.unwrap_or(3600) * 1000);
    save_idp_id_token(opts.idp_issuer, &id_token, expires_at).await?;
    tracing::debug!(
        "[xaa] Cached id_token for {} (expires {})",
        opts.idp_issuer,
        expires_at
    );
    Ok(id_token)
}

pub fn build_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/callback")
}

pub async fn find_available_port() -> Result<u16> {
    if let Ok(raw) = std::env::var("MCP_OAUTH_CALLBACK_PORT") {
        if let Ok(port) = raw.trim().parse::<u16>() {
            if port > 0 {
                return Ok(port);
            }
        }
    }

    let (min, max) = if cfg!(windows) {
        (39152_u16, 49151_u16)
    } else {
        (49152_u16, 65535_u16)
    };
    let range = u32::from(max) - u32::from(min) + 1;
    let max_attempts = range.min(100);
    for _ in 0..max_attempts {
        let port = rand::thread_rng().gen_range(min..=max);
        if port_is_available(port).await {
            return Ok(port);
        }
    }
    if port_is_available(REDIRECT_PORT_FALLBACK).await {
        return Ok(REDIRECT_PORT_FALLBACK);
    }
    Err(anyhow!("No available ports for OAuth redirect"))
}

async fn port_is_available(port: u16) -> bool {
    match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => {
            drop(listener);
            true
        }
        Err(_) => false,
    }
}

fn build_authorization_url(
    metadata: &OpenIdProviderDiscoveryMetadata,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> Result<Url> {
    let mut url = Url::parse(&metadata.authorization_endpoint)
        .map_err(|err| anyhow!("XAA IdP: invalid authorization endpoint: {err}"))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", "openid")
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url)
}

async fn wait_for_callback(
    listener: &tokio::net::TcpListener,
    expected_state: &str,
) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = vec![0_u8; 4096];
        stream.readable().await?;
        let n = stream.try_read(&mut buf)?;
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let request_line = request
            .lines()
            .next()
            .ok_or_else(|| anyhow!("XAA IdP: callback missing request line"))?;
        let url = request_line
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| anyhow!("XAA IdP: callback missing path"))?;
        let parsed = Url::parse(&format!("http://localhost{url}"))
            .map_err(|err| anyhow!("XAA IdP: invalid callback URL: {err}"))?;
        if parsed.path() != "/callback" {
            write_http_response(&mut stream, 404, "").await?;
            continue;
        }
        let mut code = None;
        let mut state = None;
        let mut error = None;
        let mut error_description = None;
        for (key, value) in parsed.query_pairs() {
            match key.as_ref() {
                "code" => code = Some(value.into_owned()),
                "state" => state = Some(value.into_owned()),
                "error" => error = Some(value.into_owned()),
                "error_description" => error_description = Some(value.into_owned()),
                _ => {}
            }
        }
        if let Some(error) = error {
            write_http_response(
                &mut stream,
                400,
                "<html><body><h3>IdP login failed</h3></body></html>",
            )
            .await?;
            return Err(anyhow!(
                "XAA IdP: {}{}",
                error,
                error_description
                    .map(|desc| format!(" — {desc}"))
                    .unwrap_or_default()
            ));
        }
        if state.as_deref() != Some(expected_state) {
            write_http_response(
                &mut stream,
                400,
                "<html><body><h3>State mismatch</h3></body></html>",
            )
            .await?;
            return Err(anyhow!("XAA IdP: state mismatch (possible CSRF)"));
        }
        let Some(code) = code else {
            write_http_response(
                &mut stream,
                400,
                "<html><body><h3>Missing code</h3></body></html>",
            )
            .await?;
            return Err(anyhow!("XAA IdP: callback missing code"));
        };
        write_http_response(
            &mut stream,
            200,
            "<html><body><h3>IdP login complete — you can close this window.</h3></body></html>",
        )
        .await?;
        return Ok(code);
    }
}

async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body: &str,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

async fn exchange_authorization_code(
    metadata: &OpenIdProviderDiscoveryMetadata,
    client_id: &str,
    client_secret: Option<&str>,
    authorization_code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<IdpTokenResponse> {
    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", authorization_code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("client_id", client_id.to_string()),
        ("code_verifier", code_verifier.to_string()),
    ];
    let mut request = reqwest::Client::new()
        .post(&metadata.token_endpoint)
        .timeout(std::time::Duration::from_millis(IDP_REQUEST_TIMEOUT_MS));

    if let Some(secret) = client_secret {
        if should_use_client_secret_post(&metadata.token_endpoint_auth_methods_supported) {
            params.push(("client_secret", secret.to_string()));
        } else {
            let credentials = STANDARD.encode(format!("{client_id}:{secret}"));
            request = request.header(
                reqwest::header::AUTHORIZATION,
                format!("Basic {credentials}"),
            );
        }
    }

    let response = request
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .form(&params)
        .send()
        .await
        .map_err(|err| anyhow!("XAA IdP: token exchange failed: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "XAA IdP: token exchange failed: HTTP {}: {}",
            status.as_u16(),
            body
        ));
    }
    response
        .json::<IdpTokenResponse>()
        .await
        .map_err(|err| anyhow!("XAA IdP: invalid token response: {err}"))
}

fn should_use_client_secret_post(methods: &[String]) -> bool {
    !methods.iter().any(|method| method == "client_secret_basic")
        && methods.iter().any(|method| method == "client_secret_post")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issuer_key_normalizes_host_and_trailing_path_slashes() {
        assert_eq!(
            issuer_key("https://LOGIN.Example.COM/tenant///"),
            "https://login.example.com/tenant"
        );
        assert_eq!(issuer_key("not a url///"), "not a url");
    }

    #[test]
    fn jwt_exp_reads_base64url_payload() {
        let payload =
            URL_SAFE_NO_PAD.encode(serde_json::json!({ "exp": 1_800_000_000_u64 }).to_string());
        let jwt = format!("header.{payload}.sig");
        assert_eq!(jwt_exp(&jwt), Some(1_800_000_000));
        assert_eq!(jwt_exp("bad"), None);
    }

    #[test]
    fn setup_validation_allows_https_and_loopback_http_only() {
        assert!(validate_setup_input("https://idp.example.com", "client", None).is_ok());
        assert!(validate_setup_input("http://localhost:3000", "client", Some(8080)).is_ok());
        assert!(validate_setup_input("http://127.0.0.1:3000", "client", None).is_ok());
        assert!(validate_setup_input("http://idp.example.com", "client", None).is_err());
        assert!(validate_setup_input("not-url", "client", None).is_err());
    }

    #[test]
    fn authorization_url_uses_pkce_openid_and_loopback_redirect() {
        let metadata = OpenIdProviderDiscoveryMetadata {
            issuer: "https://idp.example.com".into(),
            authorization_endpoint: "https://idp.example.com/authorize".into(),
            token_endpoint: "https://idp.example.com/token".into(),
            token_endpoint_auth_methods_supported: vec![],
        };
        let url = build_authorization_url(
            &metadata,
            "client",
            "http://localhost:49152/callback",
            "state",
            "challenge",
        )
        .unwrap();
        let params = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            params.get("response_type").map(String::as_str),
            Some("code")
        );
        assert_eq!(params.get("client_id").map(String::as_str), Some("client"));
        assert_eq!(params.get("scope").map(String::as_str), Some("openid"));
        assert_eq!(params.get("state").map(String::as_str), Some("state"));
        assert_eq!(
            params.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some("http://localhost:49152/callback")
        );
    }

    #[test]
    fn client_secret_auth_method_selection_matches_ts() {
        assert!(!should_use_client_secret_post(&[]));
        assert!(!should_use_client_secret_post(&[
            "client_secret_basic".into()
        ]));
        assert!(should_use_client_secret_post(
            &["client_secret_post".into()]
        ));
        assert!(!should_use_client_secret_post(&[
            "client_secret_basic".into(),
            "client_secret_post".into()
        ]));
    }
}
