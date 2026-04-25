use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Account info stored in global config after OAuth login.
///
/// Matches the TS `AccountInfo` type in `src/utils/config.ts`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub account_uuid: String,
    pub email_address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_extra_usage_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_created_at: Option<String>,
}

/// Global config stored at `~/.claude.json`.
///
/// Only includes fields relevant to auth/login. Unknown fields are preserved
/// via `flatten` so we don't clobber data written by the real Claude Code.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_completed_onboarding: Option<bool>,

    /// Stable anonymous user/device id.
    ///
    /// TS uses the exact key `userID` rather than camelCase `userId`.
    #[serde(rename = "userID", skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_account: Option<AccountInfo>,

    /// API key stored via Console OAuth flow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_api_key: Option<String>,

    /// Preserve all other fields we don't know about.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Get the path to the global config file.
///
/// Matches TS `getGlobalClaudeFile()` in `src/utils/env.ts`:
/// - Legacy: `~/.claude/.config.json`
/// - Current: `~/.claude.json`
pub fn global_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;

    // Check legacy path first (matching TS behavior)
    let legacy = home.join(".claude").join(".config.json");
    if legacy.exists() {
        return Ok(legacy);
    }

    Ok(home.join(".claude.json"))
}

/// Load the global config from disk, returning defaults if the file doesn't exist.
pub fn load_global_config() -> Result<GlobalConfig> {
    let path = global_config_path()?;
    if !path.exists() {
        return Ok(GlobalConfig::default());
    }
    let content = std::fs::read_to_string(&path).context("failed to read global config")?;
    let config: GlobalConfig = serde_json::from_str(&content).unwrap_or_default();
    Ok(config)
}

/// Save the global config to disk. Reads the existing config first and applies
/// the updater function, preserving unknown fields.
pub fn save_global_config<F>(updater: F) -> Result<()>
where
    F: FnOnce(GlobalConfig) -> GlobalConfig,
{
    let path = global_config_path()?;

    // Read existing config (or defaults)
    let current = if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        GlobalConfig::default()
    };

    let updated = updater(current);
    let json =
        serde_json::to_string_pretty(&updated).context("failed to serialize global config")?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    std::fs::write(&path, json).context("failed to write global config")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_info_serialization_camel_case() {
        let info = AccountInfo {
            account_uuid: "uuid-123".to_string(),
            email_address: "test@example.com".to_string(),
            organization_uuid: Some("org-456".to_string()),
            organization_name: None,
            organization_role: Some("admin".to_string()),
            workspace_role: None,
            display_name: Some("Test User".to_string()),
            has_extra_usage_enabled: Some(true),
            billing_type: Some("stripe".to_string()),
            account_created_at: Some("2024-01-01T00:00:00Z".to_string()),
            subscription_created_at: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        // Must use camelCase (matching TS)
        assert!(json.contains("accountUuid"));
        assert!(json.contains("emailAddress"));
        assert!(json.contains("organizationUuid"));
        assert!(json.contains("organizationRole"));
        assert!(json.contains("displayName"));
        assert!(json.contains("hasExtraUsageEnabled"));
        assert!(json.contains("billingType"));
        assert!(json.contains("accountCreatedAt"));
        // None fields should be skipped
        assert!(!json.contains("organizationName"));
        assert!(!json.contains("workspaceRole"));
        assert!(!json.contains("subscriptionCreatedAt"));
    }

    #[test]
    fn test_global_config_preserves_unknown_fields() {
        let json = r#"{
            "hasCompletedOnboarding": true,
            "oauthAccount": {
                "accountUuid": "a",
                "emailAddress": "b"
            },
            "numStartups": 42,
            "theme": "dark",
            "userID": "uid-xyz"
        }"#;

        let config: GlobalConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.has_completed_onboarding, Some(true));
        assert_eq!(config.user_id.as_deref(), Some("uid-xyz"));
        assert!(config.oauth_account.is_some());
        // Unknown fields preserved in `extra`
        assert_eq!(config.extra["numStartups"], 42);
        assert_eq!(config.extra["theme"], "dark");

        // Round-trip: unknown fields must survive serialization
        let reserialized = serde_json::to_string(&config).unwrap();
        assert!(reserialized.contains("numStartups"));
        assert!(
            reserialized.contains("\"theme\":\"dark\"")
                || reserialized.contains("\"theme\": \"dark\"")
        );
        assert!(reserialized.contains("userID"));
    }

    #[test]
    fn test_global_config_deserialize_from_ts_format() {
        let ts_json = r#"{
            "hasCompletedOnboarding": true,
            "oauthAccount": {
                "accountUuid": "acc-123",
                "emailAddress": "user@example.com",
                "organizationUuid": "org-456",
                "displayName": "Jane Doe",
                "hasExtraUsageEnabled": false,
                "billingType": "stripe",
                "accountCreatedAt": "2024-06-01T00:00:00Z",
                "subscriptionCreatedAt": "2024-06-15T00:00:00Z",
                "organizationRole": "member",
                "workspaceRole": "developer"
            },
            "primaryApiKey": "sk-ant-test",
            "numStartups": 10,
            "verbose": false
        }"#;

        let config: GlobalConfig = serde_json::from_str(ts_json).unwrap();
        assert_eq!(config.has_completed_onboarding, Some(true));
        assert_eq!(config.user_id, None);
        let account = config.oauth_account.unwrap();
        assert_eq!(account.account_uuid, "acc-123");
        assert_eq!(account.email_address, "user@example.com");
        assert_eq!(account.organization_uuid.as_deref(), Some("org-456"));
        assert_eq!(account.display_name.as_deref(), Some("Jane Doe"));
        assert_eq!(account.has_extra_usage_enabled, Some(false));
        assert_eq!(account.billing_type.as_deref(), Some("stripe"));
        assert_eq!(account.organization_role.as_deref(), Some("member"));
        assert_eq!(account.workspace_role.as_deref(), Some("developer"));
        assert_eq!(config.primary_api_key.as_deref(), Some("sk-ant-test"));
    }
}
