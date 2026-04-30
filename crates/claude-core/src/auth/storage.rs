use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Credentials keychain service suffix, matching TS `CREDENTIALS_SERVICE_SUFFIX`.
/// Distinguishes the OAuth credentials entry from the legacy API key entry.
const CREDENTIALS_SERVICE_SUFFIX: &str = "-credentials";

/// Legacy API key storage uses the base service name without a suffix.
const LEGACY_API_KEY_SERVICE_SUFFIX: &str = "";

/// Compute the keychain service name, matching the TS
/// `getMacOsKeychainStorageServiceName` in macOsKeychainHelpers.ts.
///
/// Format: `Claude Code{oauthFileSuffix}{serviceSuffix}{dirHash}`
///
/// - `oauthFileSuffix`: empty for prod, `-staging-oauth` for staging, etc.
///   (from `getOauthConfig().OAUTH_FILE_SUFFIX`)
/// - `serviceSuffix`: `-credentials` for OAuth token storage
/// - `dirHash`: only appended when `CLAUDE_CONFIG_DIR` is set (non-default dir)
fn keychain_service_name_with_suffix(service_suffix: &str) -> String {
    let dir_hash = match std::env::var("CLAUDE_CONFIG_DIR") {
        Ok(dir) if !dir.is_empty() => {
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(dir.as_bytes());
            format!("-{}", &hex::encode(hash)[..8])
        }
        _ => String::new(),
    };
    // Production: "Claude Code-credentials" (+ dirHash if custom config dir)
    format!("Claude Code{}{}", service_suffix, dir_hash)
}

pub fn keychain_service_name() -> String {
    keychain_service_name_with_suffix(CREDENTIALS_SERVICE_SUFFIX)
}

pub fn legacy_api_key_service_name() -> String {
    keychain_service_name_with_suffix(LEGACY_API_KEY_SERVICE_SUFFIX)
}

/// Get the username for keychain operations, matching TS `getUsername()`.
fn get_username() -> String {
    std::env::var("USER").unwrap_or_else(|_| "claude-code-user".into())
}

/// The token structure stored in secure storage.
///
/// Matches the TS `claudeAiOauth` object in `SecureStorageData`:
/// ```json
/// {
///   "claudeAiOauth": {
///     "accessToken": "...",
///     "refreshToken": "...",
///     "expiresAt": 1234567890000,
///     "scopes": ["user:profile", "user:inference", ...],
///     "subscriptionType": "max" | "pro" | null,
///     "rateLimitTier": "tier1" | null
///   }
/// }
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthStoredTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub subscription_type: Option<String>,
    #[serde(default)]
    pub rate_limit_tier: Option<String>,
}

/// Top-level secure storage structure, matching the TS `SecureStorageData`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecureStorageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    claude_ai_oauth: Option<OAuthStoredTokens>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trusted_device_token: Option<String>,
}

/// Load OAuth tokens from the macOS Keychain (same location as real Claude Code).
/// Falls back to ~/.claude/.credentials.json on non-macOS.
pub async fn load_tokens() -> Result<Option<OAuthStoredTokens>> {
    Ok(load_secure_storage_data()
        .await?
        .and_then(|data| data.claude_ai_oauth))
}

async fn load_secure_storage_data() -> Result<Option<SecureStorageData>> {
    if cfg!(target_os = "macos") {
        if let Some(data) = load_data_from_keychain().await? {
            return Ok(Some(data));
        }
    }
    load_data_from_file().await
}

async fn load_data_from_keychain() -> Result<Option<SecureStorageData>> {
    let username = get_username();
    let service = keychain_service_name();

    let output = tokio::process::Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            &username,
            "-w",
            "-s",
            &service,
        ])
        .output()
        .await
        .context("Failed to run security command")?;

    if !output.status.success() {
        // No entry found -- not an error, just means not logged in
        return Ok(None);
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Ok(None);
    }

    // The real Claude Code stores data as hex-encoded JSON via `security -i`
    // with the `-X` flag (hexadecimal). Try hex decode first, then raw JSON.
    let json_str = if raw.starts_with('{') {
        raw
    } else {
        // Hex-encoded JSON (the normal case for real Claude Code)
        let bytes = hex::decode(&raw).context("Keychain value is neither JSON nor valid hex")?;
        String::from_utf8(bytes).context("Hex-decoded keychain value is not valid UTF-8")?
    };

    let data: SecureStorageData =
        serde_json::from_str(&json_str).context("Failed to parse keychain JSON")?;
    Ok(Some(data))
}

/// Fallback: read from ~/.claude/.credentials.json
async fn load_data_from_file() -> Result<Option<SecureStorageData>> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = tokio::fs::read_to_string(&path).await?;
    let data: SecureStorageData = serde_json::from_str(&content)?;
    Ok(Some(data))
}

/// Store tokens.
///
/// On macOS, writes to the Keychain using `security -i` with hex encoding
/// (matching the TS `macOsKeychainStorage.update()`). Also writes to the
/// credentials file as a fallback / for cross-process cache invalidation.
///
/// On non-macOS, writes to ~/.claude/.credentials.json only.
pub async fn store_tokens(tokens: &OAuthStoredTokens) -> Result<()> {
    let mut data = load_secure_storage_data().await?.unwrap_or_default();
    data.claude_ai_oauth = Some(tokens.clone());

    // Always write to the credentials file (used for cross-process staleness
    // detection via mtime, matching TS `invalidateOAuthCacheIfDiskChanged`)
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string(&data)?;
    tokio::fs::write(&path, &json).await?;

    // On macOS, also store to the Keychain
    if cfg!(target_os = "macos") {
        if let Err(e) = store_to_keychain(&json).await {
            tracing::warn!("Failed to store tokens in keychain: {}", e);
        }
    }

    Ok(())
}

pub async fn load_trusted_device_token() -> Result<Option<String>> {
    Ok(load_secure_storage_data()
        .await?
        .and_then(|data| data.trusted_device_token)
        .filter(|token| !token.trim().is_empty()))
}

pub async fn store_trusted_device_token(token: &str) -> Result<()> {
    let mut data = load_secure_storage_data().await?.unwrap_or_default();
    data.trusted_device_token = Some(token.to_string());
    let json = serde_json::to_string(&data)?;
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, &json).await?;
    if cfg!(target_os = "macos") {
        if let Err(e) = store_to_keychain(&json).await {
            tracing::warn!("Failed to store trusted device token in keychain: {}", e);
        }
    }
    Ok(())
}

pub async fn clear_trusted_device_token() -> Result<()> {
    let Some(mut data) = load_secure_storage_data().await? else {
        return Ok(());
    };
    data.trusted_device_token = None;
    let json = serde_json::to_string(&data)?;
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, &json).await?;
    if cfg!(target_os = "macos") {
        if let Err(e) = store_to_keychain(&json).await {
            tracing::warn!("Failed to clear trusted device token in keychain: {}", e);
        }
    }
    Ok(())
}

/// Load the `/login`-managed API key.
///
/// Matches the TS `getApiKeyFromConfigOrMacOSKeychain()` behavior:
/// - macOS keychain first
/// - then `primaryApiKey` in global config
pub async fn load_managed_api_key() -> Result<Option<String>> {
    if cfg!(target_os = "macos") {
        if let Some(key) = load_api_key_from_keychain().await? {
            return Ok(Some(key));
        }
    }

    let config = crate::config::global::load_global_config()?;
    Ok(config.primary_api_key)
}

/// Store the `/login`-managed API key.
///
/// On macOS, prefer keychain storage and keep the raw key out of config when
/// that succeeds, matching the TS client.
pub async fn store_managed_api_key(api_key: &str) -> Result<()> {
    let mut saved_to_keychain = false;

    if cfg!(target_os = "macos") {
        match store_api_key_to_keychain(api_key).await {
            Ok(()) => saved_to_keychain = true,
            Err(e) => tracing::warn!("Failed to store managed API key in keychain: {}", e),
        }
    }

    crate::config::global::save_global_config(|mut config| {
        if !saved_to_keychain {
            config.primary_api_key = Some(api_key.to_string());
        }
        config
    })?;

    Ok(())
}

/// Delete the `/login`-managed API key from keychain and config.
pub async fn delete_managed_api_key() -> Result<()> {
    if cfg!(target_os = "macos") {
        let username = get_username();
        let service = legacy_api_key_service_name();
        let _ = tokio::process::Command::new("security")
            .args(["delete-generic-password", "-a", &username, "-s", &service])
            .output()
            .await;
    }

    crate::config::global::save_global_config(|mut config| {
        config.primary_api_key = None;
        config
    })?;

    Ok(())
}

/// Write to the macOS Keychain using `security -i` with hex-encoded value.
///
/// Matches the TS `macOsKeychainStorage.update()` which pipes
/// `add-generic-password -U -a USER -s SERVICE -X HEX` to `security -i`.
async fn store_to_keychain(json: &str) -> Result<()> {
    let username = get_username();
    let service = keychain_service_name();
    let hex_value = hex::encode(json.as_bytes());

    store_hex_value_in_keychain(&service, &username, &hex_value).await
}

async fn store_api_key_to_keychain(api_key: &str) -> Result<()> {
    let username = get_username();
    let service = legacy_api_key_service_name();
    let hex_value = hex::encode(api_key.as_bytes());

    store_hex_value_in_keychain(&service, &username, &hex_value).await
}

async fn store_hex_value_in_keychain(service: &str, username: &str, hex_value: &str) -> Result<()> {
    let command = format!(
        "add-generic-password -U -a \"{}\" -s \"{}\" -X \"{}\"\n",
        username, service, hex_value
    );

    let mut child = tokio::process::Command::new("security")
        .arg("-i")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn security command")?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(command.as_bytes()).await?;
        drop(stdin); // Close stdin to signal EOF
    }

    let output = child.wait_with_output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("security add-generic-password failed: {}", stderr);
    }

    Ok(())
}

async fn load_api_key_from_keychain() -> Result<Option<String>> {
    let username = get_username();
    let service = legacy_api_key_service_name();

    let output = tokio::process::Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            &username,
            "-w",
            "-s",
            &service,
        ])
        .output()
        .await
        .context("Failed to run security command for API key")?;

    if !output.status.success() {
        return Ok(None);
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Ok(None);
    }

    if raw.starts_with("sk-") {
        return Ok(Some(raw));
    }

    let bytes = hex::decode(&raw)
        .context("Managed API key keychain value is neither raw text nor valid hex")?;
    let decoded =
        String::from_utf8(bytes).context("Managed API key keychain value is not valid UTF-8")?;

    Ok(Some(decoded))
}

/// Delete stored OAuth tokens (both keychain and file).
///
/// Used during logout and before re-login to clear old state.
/// Matches TS `performLogout()` → `secureStorage.delete()`.
pub async fn delete_tokens() -> Result<()> {
    // Delete the credentials file
    let path = credentials_path()?;
    if path.exists() {
        tokio::fs::remove_file(&path).await.ok();
    }

    // On macOS, also delete from keychain
    if cfg!(target_os = "macos") {
        let username = get_username();
        let service = keychain_service_name();
        let _ = tokio::process::Command::new("security")
            .args(["delete-generic-password", "-a", &username, "-s", &service])
            .output()
            .await;
    }

    Ok(())
}

fn credentials_path() -> Result<PathBuf> {
    let dir = crate::config::paths::claude_dir()?;
    Ok(dir.join(".credentials.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keychain_service_name_default() {
        // When CLAUDE_CONFIG_DIR is not set, should be "Claude Code-credentials"
        // (This test may vary depending on env, but the format should be correct)
        let name = keychain_service_name();
        assert!(name.starts_with("Claude Code-credentials"), "Got: {}", name);
    }

    #[test]
    fn test_oauth_tokens_serialization() {
        let tokens = OAuthStoredTokens {
            access_token: "at_test".to_string(),
            refresh_token: Some("rt_test".to_string()),
            expires_at: Some(1234567890000),
            scopes: vec!["user:profile".to_string(), "user:inference".to_string()],
            subscription_type: Some("max".to_string()),
            rate_limit_tier: None,
        };

        let data = SecureStorageData {
            claude_ai_oauth: Some(tokens),
            trusted_device_token: None,
        };

        let json = serde_json::to_string(&data).unwrap();
        // Must use camelCase (matching TS)
        assert!(json.contains("claudeAiOauth"), "Must use camelCase key");
        assert!(
            json.contains("accessToken"),
            "Must use camelCase for fields"
        );
        assert!(json.contains("refreshToken"));
        assert!(json.contains("expiresAt"));

        // Verify round-trip
        let parsed: SecureStorageData = serde_json::from_str(&json).unwrap();
        let oauth = parsed.claude_ai_oauth.unwrap();
        assert_eq!(oauth.access_token, "at_test");
        assert_eq!(oauth.refresh_token.as_deref(), Some("rt_test"));
        assert_eq!(oauth.expires_at, Some(1234567890000));
        assert_eq!(oauth.scopes, vec!["user:profile", "user:inference"]);
        assert_eq!(oauth.subscription_type.as_deref(), Some("max"));
        assert_eq!(oauth.rate_limit_tier, None);
    }

    #[test]
    fn test_deserialize_from_ts_format() {
        // Simulate what the real Claude Code stores
        let ts_json = r#"{
            "claudeAiOauth": {
                "accessToken": "ey_test_token",
                "refreshToken": "rt_refresh",
                "expiresAt": 1700000000000,
                "scopes": ["user:profile", "user:inference", "user:sessions:claude_code"],
                "subscriptionType": "pro",
                "rateLimitTier": "tier1"
            }
        }"#;

        let data: SecureStorageData = serde_json::from_str(ts_json).unwrap();
        let oauth = data.claude_ai_oauth.unwrap();
        assert_eq!(oauth.access_token, "ey_test_token");
        assert_eq!(oauth.scopes.len(), 3);
        assert_eq!(oauth.subscription_type.as_deref(), Some("pro"));
        assert_eq!(oauth.rate_limit_tier.as_deref(), Some("tier1"));
    }
}
