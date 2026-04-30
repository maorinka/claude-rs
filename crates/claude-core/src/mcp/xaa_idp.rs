//! XAA IdP settings and cache helpers.
//!
//! This ports the general storage/keying behavior from TS
//! `services/mcp/xaaIdpLogin.ts`: a user-level `settings.xaaIdp` config plus
//! issuer-keyed secure-storage maps for cached id_tokens and client secrets.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

const ID_TOKEN_EXPIRY_BUFFER_S: u64 = 60;

pub use crate::config::settings::XaaIdpSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XaaSetupInput {
    pub issuer: String,
    pub client_id: String,
    pub callback_port: Option<u32>,
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
}
