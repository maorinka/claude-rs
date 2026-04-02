use super::login::{build_token_refresh_body, is_claude_ai_auth, proxy_url, TOKEN_URL};
use crate::api::client::AuthMethod;
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::Mutex;

const CCR_OAUTH_TOKEN_PATH: &str = "/home/claude/.claude/remote/.oauth_token";
const CCR_API_KEY_PATH: &str = "/home/claude/.claude/remote/.api_key";
const REFRESH_LOCK_RETRIES: usize = 5;
const DEFAULT_API_KEY_HELPER_TTL_MS: u64 = 5 * 60 * 1000;

static API_KEY_HELPER_CACHE: Lazy<Mutex<Option<ApiKeyHelperCacheEntry>>> =
    Lazy::new(|| Mutex::new(None));

#[derive(Clone)]
struct ApiKeyHelperCacheEntry {
    value: String,
    cached_at_ms: u64,
}

/// Check if an OAuth token has expired (with a 5 minute buffer, matching TS).
fn is_token_expired(expires_at: Option<u64>) -> bool {
    match expires_at {
        None => false,
        Some(exp) => {
            let buffer_ms = 5 * 60 * 1000;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            now_ms + buffer_ms >= exp
        }
    }
}

/// Resolve authentication, matching the leaked client's runtime precedence.
///
/// Priority:
/// 1. `CLAUDE_CODE_OAUTH_TOKEN`
/// 2. `CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR` (or CCR disk fallback)
/// 3. Stored Claude.ai OAuth tokens
/// 4. `CLAUDE_CODE_OAUTH_REFRESH_TOKEN` (+ `CLAUDE_CODE_OAUTH_SCOPES`)
/// 5. `ANTHROPIC_AUTH_TOKEN`
/// 6. `ANTHROPIC_API_KEY`
/// 7. `apiKeyHelper`
/// 8. `/login`-managed API key (keychain / config)
/// 9. `~/.claude/settings.json` `api_key`
pub async fn resolve_auth() -> Result<AuthMethod> {
    if is_anthropic_unix_socket_proxy_mode() {
        if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
            if !token.is_empty() {
                return Ok(AuthMethod::OAuthToken(token));
            }
        }
        anyhow::bail!("No authentication found for proxied OAuth session");
    }

    if is_third_party_auth_mode() {
        anyhow::bail!("Anthropic direct authentication is disabled in third-party provider mode");
    }

    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        if !token.is_empty() {
            return Ok(AuthMethod::OAuthToken(token));
        }
    }

    if let Some(token) = read_credential_from_fd_or_file(
        "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
        CCR_OAUTH_TOKEN_PATH,
    )? {
        return Ok(AuthMethod::OAuthToken(token));
    }

    if let Some(tokens) = resolve_stored_oauth_token(false).await? {
        return Ok(AuthMethod::OAuthToken(tokens));
    }

    if let Ok(refresh_token) = std::env::var("CLAUDE_CODE_OAUTH_REFRESH_TOKEN") {
        if !refresh_token.is_empty() {
            let scopes = std::env::var("CLAUDE_CODE_OAUTH_SCOPES")
                .unwrap_or_default()
                .split_whitespace()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            if scopes.is_empty() {
                anyhow::bail!(
                    "CLAUDE_CODE_OAUTH_SCOPES is required when using CLAUDE_CODE_OAUTH_REFRESH_TOKEN"
                );
            }

            return Ok(AuthMethod::OAuthToken(
                refresh_oauth_token(&refresh_token, &scopes).await?,
            ));
        }
    }

    if !is_managed_oauth_context() {
        if let Ok(token) = std::env::var("ANTHROPIC_AUTH_TOKEN") {
            if !token.is_empty() {
                return Ok(AuthMethod::OAuthToken(token));
            }
        }
    }

    if !is_managed_oauth_context() {
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.is_empty() {
                return Ok(AuthMethod::ApiKey(key));
            }
        }
    }

    if let Some(key) =
        read_credential_from_fd_or_file("CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR", CCR_API_KEY_PATH)?
    {
        return Ok(AuthMethod::ApiKey(key));
    }

    if !is_managed_oauth_context() {
        if let Some(key) = get_api_key_from_helper().await? {
            if !key.is_empty() {
                return Ok(AuthMethod::ApiKey(key));
            }
        }
    }

    if let Some(key) = super::storage::load_managed_api_key().await? {
        if !key.is_empty() {
            return Ok(AuthMethod::ApiKey(key));
        }
    }

    if !is_managed_oauth_context() {
        let settings_path = crate::config::paths::user_settings_path()?;
        let settings = crate::config::settings::Settings::load_from_file(&settings_path);
        if let Some(key) = settings.api_key {
            if !key.is_empty() {
                return Ok(AuthMethod::ApiKey(key));
            }
        }
    }

    anyhow::bail!("No authentication found. Run `claude-rs login` or set ANTHROPIC_API_KEY")
}

fn is_managed_oauth_context() -> bool {
    env_truthy("CLAUDE_CODE_REMOTE")
        || std::env::var("CLAUDE_CODE_ENTRYPOINT")
            .map(|v| v == "claude-desktop")
            .unwrap_or(false)
}

fn is_anthropic_unix_socket_proxy_mode() -> bool {
    std::env::var_os("ANTHROPIC_UNIX_SOCKET").is_some()
}

fn is_third_party_auth_mode() -> bool {
    env_truthy("CLAUDE_CODE_USE_BEDROCK")
        || env_truthy("CLAUDE_CODE_USE_VERTEX")
        || env_truthy("CLAUDE_CODE_USE_FOUNDRY")
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            matches!(
                v.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(false)
}

async fn refresh_oauth_token(refresh_token: &str, stored_scopes: &[String]) -> Result<String> {
    let scopes = if is_claude_ai_auth(stored_scopes) {
        None
    } else {
        Some(stored_scopes)
    };

    let body = build_token_refresh_body(refresh_token, scopes);

    let token_url = proxy_url(TOKEN_URL);
    let client = super::login::debug_http_client();
    let resp = client
        .post(&token_url)
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

    let existing = super::storage::load_tokens().await?;
    let new_tokens = super::storage::OAuthStoredTokens {
        access_token: access_token.clone(),
        refresh_token: data["refresh_token"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| Some(refresh_token.to_string())),
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
            .unwrap_or_else(|| stored_scopes.to_vec()),
        subscription_type: existing
            .as_ref()
            .and_then(|tokens| tokens.subscription_type.clone()),
        rate_limit_tier: existing
            .as_ref()
            .and_then(|tokens| tokens.rate_limit_tier.clone()),
    };
    let _ = super::storage::store_tokens(&new_tokens).await;

    Ok(access_token)
}

pub async fn handle_oauth_401_error(failed_access_token: &str) -> Result<bool> {
    let current_tokens = super::storage::load_tokens().await?;
    let Some(current_tokens) = current_tokens else {
        return Ok(false);
    };

    if !is_claude_ai_auth(&current_tokens.scopes) || current_tokens.refresh_token.is_none() {
        return Ok(false);
    }

    if current_tokens.access_token != failed_access_token {
        return Ok(true);
    }

    let refreshed = resolve_stored_oauth_token(true).await?;
    Ok(refreshed
        .as_deref()
        .map(|token| token != failed_access_token)
        .unwrap_or(false))
}

pub async fn resolve_stored_oauth_token(force_refresh: bool) -> Result<Option<String>> {
    let Some(tokens) = super::storage::load_tokens().await? else {
        return Ok(None);
    };

    if !is_claude_ai_auth(&tokens.scopes) {
        return Ok(None);
    }

    let needs_refresh = force_refresh || is_token_expired(tokens.expires_at);
    if !needs_refresh {
        return Ok(Some(tokens.access_token));
    }

    let Some(refresh_token) = tokens.refresh_token.clone() else {
        return Ok(Some(tokens.access_token));
    };

    match refresh_oauth_token_with_lock(&refresh_token, &tokens.scopes, force_refresh).await {
        Ok(token) => Ok(Some(token)),
        Err(e) => {
            tracing::warn!("Token refresh failed ({}), using stored token", e);
            Ok(Some(tokens.access_token))
        }
    }
}

async fn refresh_oauth_token_with_lock(
    refresh_token: &str,
    stored_scopes: &[String],
    force_refresh: bool,
) -> Result<String> {
    let lock_path = refresh_lock_path()?;
    if let Some(parent) = lock_path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let _lock = acquire_refresh_lock(&lock_path).await?;

    if let Some(latest) = super::storage::load_tokens().await? {
        if is_claude_ai_auth(&latest.scopes) {
            let still_expired = force_refresh || is_token_expired(latest.expires_at);
            if !still_expired {
                return Ok(latest.access_token);
            }
        }
    }

    refresh_oauth_token(refresh_token, stored_scopes).await
}

fn refresh_lock_path() -> Result<PathBuf> {
    Ok(crate::config::paths::claude_dir()?.join(".lock"))
}

struct RefreshLock {
    path: PathBuf,
}

impl Drop for RefreshLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn acquire_refresh_lock(path: &Path) -> Result<RefreshLock> {
    for attempt in 0..=REFRESH_LOCK_RETRIES {
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await
        {
            Ok(_) => {
                return Ok(RefreshLock {
                    path: path.to_path_buf(),
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if attempt == REFRESH_LOCK_RETRIES {
                    anyhow::bail!("timed out waiting for OAuth refresh lock");
                }
                tokio::time::sleep(Duration::from_millis(1000 + (attempt as u64 * 250))).await;
            }
            Err(e) => {
                return Err(e).with_context(|| format!("failed to create lock {}", path.display()))
            }
        }
    }

    anyhow::bail!("timed out waiting for OAuth refresh lock")
}

fn read_credential_from_fd_or_file(env_var: &str, fallback_path: &str) -> Result<Option<String>> {
    if let Ok(fd_env) = std::env::var(env_var) {
        if !fd_env.is_empty() {
            if let Ok(fd) = fd_env.parse::<i32>() {
                let fd_path = if cfg!(any(target_os = "macos", target_os = "freebsd")) {
                    format!("/dev/fd/{}", fd)
                } else {
                    format!("/proc/self/fd/{}", fd)
                };

                match std::fs::read_to_string(&fd_path) {
                    Ok(value) => {
                        let trimmed = value.trim().to_string();
                        if !trimmed.is_empty() {
                            return Ok(Some(trimmed));
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Failed to read {} via {}: {}", env_var, fd_path, e);
                    }
                }
            } else {
                tracing::warn!("{} is not a valid file descriptor number", env_var);
            }
        }
    }

    read_token_file(fallback_path)
}

fn read_token_file(path: &str) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(value) => {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("failed to read credential file {}", path)),
    }
}

async fn get_api_key_from_helper() -> Result<Option<String>> {
    let settings_path = crate::config::paths::user_settings_path()?;
    let settings = crate::config::settings::Settings::load_from_file(&settings_path);
    let Some(command) = settings.api_key_helper else {
        return Ok(None);
    };

    let ttl_ms = std::env::var("CLAUDE_CODE_API_KEY_HELPER_TTL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_API_KEY_HELPER_TTL_MS);
    let now_ms = current_time_ms();

    {
        let cache = API_KEY_HELPER_CACHE.lock().await;
        if let Some(entry) = cache.as_ref() {
            if now_ms.saturating_sub(entry.cached_at_ms) < ttl_ms {
                return Ok(Some(entry.value.clone()));
            }
        }
    }

    let output = tokio::process::Command::new("/bin/sh")
        .arg("-lc")
        .arg(&command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .context("failed to execute apiKeyHelper")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "apiKeyHelper failed: {}",
            if stderr.is_empty() {
                format!("exited with {}", output.status)
            } else {
                stderr
            }
        );
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        anyhow::bail!("apiKeyHelper did not return a value");
    }

    let mut cache = API_KEY_HELPER_CACHE.lock().await;
    *cache = Some(ApiKeyHelperCacheEntry {
        value: value.clone(),
        cached_at_ms: now_ms,
    });

    Ok(Some(value))
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
        let far_future = 4_102_444_800_000u64;
        assert!(!is_token_expired(Some(far_future)));
    }

    #[test]
    fn test_is_token_expired_past() {
        assert!(is_token_expired(Some(0)));
        assert!(is_token_expired(Some(1000)));
    }

    #[test]
    fn test_is_token_expired_within_buffer() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let three_min = 3 * 60 * 1000;
        assert!(is_token_expired(Some(now_ms + three_min)));
    }

    #[test]
    fn test_is_token_expired_outside_buffer() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let ten_min = 10 * 60 * 1000;
        assert!(!is_token_expired(Some(now_ms + ten_min)));
    }

    #[test]
    fn test_read_token_file_missing_returns_none() {
        let missing = format!("/tmp/claude-rs-missing-{}", std::process::id());
        assert!(read_token_file(&missing).unwrap().is_none());
    }

    #[test]
    fn test_read_token_file_trims_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "  abc123 \n").unwrap();

        let token = read_token_file(path.to_str().unwrap()).unwrap();
        assert_eq!(token.as_deref(), Some("abc123"));
    }
}
