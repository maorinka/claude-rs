use std::path::PathBuf;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// Keychain service name used by the real Claude Code (production).
// Format: "Claude Code{OAUTH_FILE_SUFFIX}{serviceSuffix}{dirHash}"
// Production: OAUTH_FILE_SUFFIX="" , serviceSuffix="-credentials", dirHash="" (default dir)
const KEYCHAIN_SERVICE_NAME: &str = "Claude Code-credentials";

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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecureStorageData {
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
async fn load_from_keychain() -> Result<Option<OAuthStoredTokens>> {
    let username = std::env::var("USER").unwrap_or_else(|_| "claude-code-user".into());

    let output = tokio::process::Command::new("security")
        .args([
            "find-generic-password",
            "-a", &username,
            "-w",
            "-s", KEYCHAIN_SERVICE_NAME,
        ])
        .output()
        .await
        .context("Failed to run security command")?;

    if !output.status.success() {
        // No entry found — not an error, just means not logged in
        return Ok(None);
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Ok(None);
    }

    // The real Claude Code stores data as raw JSON (or sometimes hex-encoded).
    // Try JSON first, then hex decode.
    let json_str = if raw.starts_with('{') {
        raw
    } else {
        // Hex-encoded JSON (used when data contains special chars)
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

/// Store tokens to ~/.claude/.credentials.json (and optionally keychain)
pub async fn store_tokens(tokens: &OAuthStoredTokens) -> Result<()> {
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let data = SecureStorageData { claude_ai_oauth: Some(tokens.clone()) };
    let json = serde_json::to_string_pretty(&data)?;
    tokio::fs::write(&path, json).await?;
    Ok(())
}

fn credentials_path() -> Result<PathBuf> {
    let dir = crate::config::paths::claude_dir()?;
    Ok(dir.join(".credentials.json"))
}
