use anyhow::{Context, Result};
use serde::Deserialize;

use super::login::{proxy_url, OAUTH_BETA_HEADER};
use crate::config::global::{save_global_config, AccountInfo};

// ── Profile endpoint URLs ───────────────────────────────────────────────────

const BASE_API_URL: &str = "https://api.anthropic.com";

/// OAuth profile endpoint (Bearer token).
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";

/// User roles endpoint.
const ROLES_URL: &str = "https://api.anthropic.com/api/oauth/claude_cli/roles";

/// CLI profile endpoint (API key).
fn cli_profile_url() -> String {
    proxy_url(&format!("{}/api/claude_cli_profile", BASE_API_URL))
}

// ── Profile response types (matching TS OAuthProfileResponse) ───────────────

#[derive(Clone, Debug, Deserialize)]
pub struct OAuthProfileResponse {
    pub account: ProfileAccount,
    pub organization: ProfileOrganization,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProfileAccount {
    pub uuid: String,
    pub email: String,
    pub display_name: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProfileOrganization {
    pub uuid: String,
    pub organization_type: Option<String>,
    pub billing_type: Option<String>,
    pub has_extra_usage_enabled: Option<bool>,
    pub rate_limit_tier: Option<String>,
    pub subscription_created_at: Option<String>,
}

/// User roles response from `/api/oauth/claude_cli/roles`.
#[derive(Clone, Debug, Deserialize)]
pub struct UserRolesResponse {
    pub organization_role: Option<String>,
    pub workspace_role: Option<String>,
    pub organization_name: Option<String>,
}

// ── Subscription type mapping (matching TS fetchProfileInfo in client.ts) ───

/// Map organization_type to subscription type.
/// Matches TS `fetchProfileInfo()` switch in `src/services/oauth/client.ts`.
pub fn map_subscription_type(org_type: Option<&str>) -> Option<String> {
    match org_type {
        Some("claude_max") => Some("max".to_string()),
        Some("claude_pro") => Some("pro".to_string()),
        Some("claude_enterprise") => Some("enterprise".to_string()),
        Some("claude_team") => Some("team".to_string()),
        _ => None,
    }
}

// ── Profile info result ─────────────────────────────────────────────────────

/// Aggregated profile info, matching the return of TS `fetchProfileInfo()`.
pub struct ProfileInfo {
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub display_name: Option<String>,
    pub has_extra_usage_enabled: Option<bool>,
    pub billing_type: Option<String>,
    pub account_created_at: Option<String>,
    pub subscription_created_at: Option<String>,
    pub raw_profile: Option<OAuthProfileResponse>,
}

// ── Fetch functions ─────────────────────────────────────────────────────────

/// Fetch the OAuth profile using a Bearer access token.
///
/// Matches TS `getOauthProfileFromOauthToken()` in `getOauthProfile.ts`.
pub async fn fetch_profile_from_oauth_token(
    access_token: &str,
) -> Result<Option<OAuthProfileResponse>> {
    let profile_url = proxy_url(PROFILE_URL);
    let client = super::login::debug_http_client();
    let resp = client
        .get(&profile_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let profile: OAuthProfileResponse =
                r.json().await.context("failed to parse profile response")?;
            Ok(Some(profile))
        }
        Ok(r) => {
            tracing::warn!("Profile fetch returned {}", r.status());
            Ok(None)
        }
        Err(e) => {
            tracing::warn!("Profile fetch failed: {}", e);
            Ok(None)
        }
    }
}

/// Fetch the CLI profile using an API key.
///
/// Matches TS `getOauthProfileFromApiKey()` in `getOauthProfile.ts`.
pub async fn fetch_profile_from_api_key(
    api_key: &str,
    account_uuid: &str,
) -> Result<Option<OAuthProfileResponse>> {
    let client = super::login::debug_http_client();
    let resp = client
        .get(cli_profile_url())
        .header("x-api-key", api_key)
        .header("anthropic-beta", OAUTH_BETA_HEADER)
        .query(&[("account_uuid", account_uuid)])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let profile: OAuthProfileResponse = r
                .json()
                .await
                .context("failed to parse CLI profile response")?;
            Ok(Some(profile))
        }
        Ok(_) | Err(_) => Ok(None),
    }
}

/// Fetch profile info and map to subscription/billing types.
///
/// Matches TS `fetchProfileInfo()` in `src/services/oauth/client.ts`.
pub async fn fetch_profile_info(access_token: &str) -> ProfileInfo {
    let profile = fetch_profile_from_oauth_token(access_token)
        .await
        .unwrap_or(None);

    let org_type = profile
        .as_ref()
        .and_then(|p| p.organization.organization_type.as_deref());

    ProfileInfo {
        subscription_type: map_subscription_type(org_type),
        rate_limit_tier: profile
            .as_ref()
            .and_then(|p| p.organization.rate_limit_tier.clone()),
        display_name: profile
            .as_ref()
            .and_then(|p| p.account.display_name.clone()),
        has_extra_usage_enabled: profile
            .as_ref()
            .and_then(|p| p.organization.has_extra_usage_enabled),
        billing_type: profile
            .as_ref()
            .and_then(|p| p.organization.billing_type.clone()),
        account_created_at: profile.as_ref().and_then(|p| p.account.created_at.clone()),
        subscription_created_at: profile
            .as_ref()
            .and_then(|p| p.organization.subscription_created_at.clone()),
        raw_profile: profile,
    }
}

/// Fetch user roles and store them in global config.
///
/// Matches TS `fetchAndStoreUserRoles()` in `src/services/oauth/client.ts`.
pub async fn fetch_and_store_user_roles(access_token: &str) -> Result<()> {
    let roles_url = proxy_url(ROLES_URL);
    let client = super::login::debug_http_client();
    let resp = client
        .get(&roles_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("failed to fetch user roles")?;

    if !resp.status().is_success() {
        anyhow::bail!("Failed to fetch user roles: {}", resp.status());
    }

    let roles: UserRolesResponse = resp
        .json()
        .await
        .context("failed to parse roles response")?;

    save_global_config(|mut config| {
        if let Some(ref mut account) = config.oauth_account {
            account.organization_role = roles.organization_role;
            account.workspace_role = roles.workspace_role;
            account.organization_name = roles.organization_name;
        }
        config
    })?;

    Ok(())
}

/// Store OAuth account info in the global config.
///
/// Matches TS `storeOAuthAccountInfo()` in `src/services/oauth/client.ts`.
pub fn store_oauth_account_info(info: AccountInfo) -> Result<()> {
    save_global_config(|mut config| {
        config.oauth_account = Some(info);
        config
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_subscription_type() {
        assert_eq!(
            map_subscription_type(Some("claude_pro")),
            Some("pro".to_string())
        );
        assert_eq!(
            map_subscription_type(Some("claude_max")),
            Some("max".to_string())
        );
        assert_eq!(
            map_subscription_type(Some("claude_team")),
            Some("team".to_string())
        );
        assert_eq!(
            map_subscription_type(Some("claude_enterprise")),
            Some("enterprise".to_string())
        );
        assert_eq!(map_subscription_type(Some("unknown")), None);
        assert_eq!(map_subscription_type(None), None);
    }

    #[test]
    fn test_profile_response_deserialize() {
        let json = r#"{
            "account": {
                "uuid": "acc-123",
                "email": "user@example.com",
                "display_name": "Test User",
                "created_at": "2024-01-01T00:00:00Z"
            },
            "organization": {
                "uuid": "org-456",
                "organization_type": "claude_pro",
                "billing_type": "stripe",
                "has_extra_usage_enabled": true,
                "rate_limit_tier": "tier1",
                "subscription_created_at": "2024-01-15T00:00:00Z"
            }
        }"#;

        let profile: OAuthProfileResponse = serde_json::from_str(json).unwrap();
        assert_eq!(profile.account.uuid, "acc-123");
        assert_eq!(profile.account.email, "user@example.com");
        assert_eq!(profile.account.display_name.as_deref(), Some("Test User"));
        assert_eq!(profile.organization.uuid, "org-456");
        assert_eq!(
            profile.organization.organization_type.as_deref(),
            Some("claude_pro")
        );
        assert_eq!(
            profile.organization.rate_limit_tier.as_deref(),
            Some("tier1")
        );
    }

    #[test]
    fn test_user_roles_deserialize() {
        let json = r#"{
            "organization_role": "admin",
            "workspace_role": "developer",
            "organization_name": "Acme Corp"
        }"#;

        let roles: UserRolesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(roles.organization_role.as_deref(), Some("admin"));
        assert_eq!(roles.workspace_role.as_deref(), Some("developer"));
        assert_eq!(roles.organization_name.as_deref(), Some("Acme Corp"));
    }

    #[test]
    fn test_profile_response_minimal() {
        // Profile with only required fields
        let json = r#"{
            "account": {
                "uuid": "a",
                "email": "b"
            },
            "organization": {
                "uuid": "c"
            }
        }"#;

        let profile: OAuthProfileResponse = serde_json::from_str(json).unwrap();
        assert_eq!(profile.account.uuid, "a");
        assert_eq!(profile.account.display_name, None);
        assert_eq!(profile.organization.organization_type, None);
        assert_eq!(profile.organization.rate_limit_tier, None);
    }
}
