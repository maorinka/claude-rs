use std::path::PathBuf;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Credentials keychain service suffix, matching TS `CREDENTIALS_SERVICE_SUFFIX`.
/// Distinguishes the OAuth credentials entry from the legacy API key entry.
const CREDENTIALS_SERVICE_SUFFIX: &str = "-credentials";

/// Compute the keychain service name, matching the TS
/// `getMacOsKeychainStorageServiceName` in macOsKeychainHelpers.ts.
///
/// Format: `Claude Code{oauthFileSuffix}{serviceSuffix}{dirHash}`
///
/// - `oauthFileSuffix`: empty for prod, `-staging-oauth` for staging, etc.
///   (from `getOauthConfig().OAUTH_FILE_SUFFIX`)
/// - `serviceSuffix`: `-credentials` for OAuth token storage
/// - `dirHash`: only appended when `CLAUDE_CONFIG_DIR` is set (non-default dir)
pub fn keychain_service_name() -> String {
    let dir_hash = match std::env::var("CLAUDE_CONFIG_DIR") {
        Ok(dir) if !dir.is_empty() => {
            use sha2::{Sha256, Digest};
            let hash = Sha256::digest(dir.as_bytes());
            format!("-{}", &hex::encode(hash)[..8])
        }
        _ => String::new(),
    };
    // Production: "Claude Code-credentials" (+ dirHash if custom config dir)
    format!("Claude Code{}{}", CREDENTIALS_SERVICE_SUFFIX, dir_hash)
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecureStorageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    claude_ai_oauth: Option<OAuthStoredTokens>,
}

/// Load OAuth tokens from the macOS Keychain (same location as real Claude Code).
/// Falls back to ~/.claude/.credentials.json on non-macOS.
pub async fn load_tokens() -> Result<Option<OAuthStoredTokens>> {
    // Try macOS Keychain first
    if cfg!(target_os = "macos") {
        if let Some(tokens) = load_from_keychain().await? {
            return Ok(Some(tokens));
        }
    }

    // Fallback to file
    load_from_file().await
}

/// Read from macOS Keychain using `security find-generic-password`.
/// This reads the exact same entry that the real Claude Code writes.
///
/// Matches the TS `macOsKeychainStorage.read()` and `doReadAsync()` in
/// `macOsKeychainStorage.ts`.
async fn load_from_keychain() -> Result<Option<OAuthStoredTokens>> {
    let username = get_username();
    let service = keychain_service_name();

    let output = tokio::process::Command::new("security")
        .args([
            "find-generic-password",
            "-a", &username,
            "-w",
            "-s", &service,
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
        let bytes = hex::decode(&raw)
            .context("Keychain value is neither JSON nor valid hex")?;
        String::from_utf8(bytes)
            .context("Hex-decoded keychain value is not valid UTF-8")?
    };

    let data: SecureStorageData = serde_json::from_str(&json_str)
        .context("Failed to parse keychain JSON")?;

    Ok(data.claude_ai_oauth)
}

/// Fallback: read from ~/.claude/.credentials.json
async fn load_from_file() -> Result<Option<OAuthStoredTokens>> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let content = tokio::fs::read_to_string(&path).await?;
    let data: SecureStorageData = serde_json::from_str(&content)?;
    Ok(data.claude_ai_oauth)
}

/// Store tokens.
///
/// On macOS, writes to the Keychain using `security -i` with hex encoding
/// (matching the TS `macOsKeychainStorage.update()`). Also writes to the
/// credentials file as a fallback / for cross-process cache invalidation.
///
/// On non-macOS, writes to ~/.claude/.credentials.json only.
pub async fn store_tokens(tokens: &OAuthStoredTokens) -> Result<()> {
    let data = SecureStorageData {
        claude_ai_oauth: Some(tokens.clone()),
    };

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

/// Write to the macOS Keychain using `security -i` with hex-encoded value.
///
/// Matches the TS `macOsKeychainStorage.update()` which pipes
/// `add-generic-password -U -a USER -s SERVICE -X HEX` to `security -i`.
async fn store_to_keychain(json: &str) -> Result<()> {
    let username = get_username();
    let service = keychain_service_name();
    let hex_value = hex::encode(json.as_bytes());

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
        };

        let json = serde_json::to_string(&data).unwrap();
        // Must use camelCase (matching TS)
        assert!(json.contains("claudeAiOauth"), "Must use camelCase key");
        assert!(json.contains("accessToken"), "Must use camelCase for fields");
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
