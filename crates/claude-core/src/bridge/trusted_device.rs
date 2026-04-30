//! Trusted-device token source for elevated bridge sessions.
//!
//! Mirrors TS `src/bridge/trustedDevice.ts`: the CLI sends
//! `X-Trusted-Device-Token` only when the rollout gate is enabled, an env var
//! takes precedence over secure storage, and the cached storage read can be
//! cleared after login/logout mutations.

use std::sync::{OnceLock, RwLock};

const TRUSTED_DEVICE_GATE: &str = "tengu_sessions_elevated_auth_enforcement";

fn cached_token() -> &'static RwLock<Option<Option<String>>> {
    static CACHE: OnceLock<RwLock<Option<Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(None))
}

pub async fn get_trusted_device_token() -> Option<String> {
    if !crate::growthbook::get_feature_value_cached_may_be_stale_bool(TRUSTED_DEVICE_GATE, false) {
        return None;
    }
    if let Ok(token) = std::env::var("CLAUDE_TRUSTED_DEVICE_TOKEN") {
        if !token.trim().is_empty() {
            return Some(token);
        }
    }
    if let Ok(guard) = cached_token().read() {
        if let Some(value) = guard.clone() {
            return value;
        }
    }
    let token = crate::auth::storage::load_trusted_device_token()
        .await
        .ok()
        .flatten();
    if let Ok(mut guard) = cached_token().write() {
        *guard = Some(token.clone());
    }
    token
}

pub fn clear_trusted_device_token_cache() {
    if let Ok(mut guard) = cached_token().write() {
        *guard = None;
    }
}

pub async fn clear_trusted_device_token() {
    if !crate::growthbook::get_feature_value_cached_may_be_stale_bool(TRUSTED_DEVICE_GATE, false) {
        return;
    }
    let _ = crate::auth::storage::clear_trusted_device_token().await;
    clear_trusted_device_token_cache();
}

#[derive(Debug, serde::Deserialize)]
struct TrustedDeviceEnrollResponse {
    device_token: Option<String>,
    device_id: Option<String>,
}

pub async fn enroll_trusted_device(access_token: &str) {
    if !crate::growthbook::get_feature_value_cached_may_be_stale_bool(TRUSTED_DEVICE_GATE, false) {
        tracing::debug!(
            gate = TRUSTED_DEVICE_GATE,
            "trusted-device enrollment skipped because gate is off"
        );
        return;
    }
    if std::env::var("CLAUDE_TRUSTED_DEVICE_TOKEN")
        .ok()
        .is_some_and(|token| !token.trim().is_empty())
    {
        tracing::debug!(
            "trusted-device enrollment skipped because CLAUDE_TRUSTED_DEVICE_TOKEN is set"
        );
        return;
    }
    if access_token.trim().is_empty() {
        tracing::debug!("trusted-device enrollment skipped because OAuth token is missing");
        return;
    }
    if crate::privacy_level::is_essential_traffic_only() {
        tracing::debug!("trusted-device enrollment skipped in essential-traffic-only mode");
        return;
    }

    let Ok(oauth) = crate::constants::oauth::get_oauth_config() else {
        tracing::debug!("trusted-device enrollment skipped because OAuth config is unavailable");
        return;
    };
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-host".to_string());
    let display_name = format!("Claude Code on {hostname} · {}", std::env::consts::OS);
    let client = match crate::proxy::build_proxy_client_from_env() {
        Ok(client) => client,
        Err(err) => {
            tracing::debug!(error = %err, "trusted-device enrollment skipped: client build failed");
            return;
        }
    };
    let url = format!(
        "{}/api/auth/trusted_devices",
        oauth.base_api_url.trim_end_matches('/')
    );
    let response = match client
        .post(url)
        .bearer_auth(access_token)
        .header("content-type", "application/json")
        .json(&serde_json::json!({ "display_name": display_name }))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            tracing::debug!(error = %err, "trusted-device enrollment request failed");
            return;
        }
    };
    let status = response.status();
    if status.as_u16() != 200 && status.as_u16() != 201 {
        let body = response.text().await.unwrap_or_default();
        tracing::debug!(
            status = status.as_u16(),
            body = %body.chars().take(200).collect::<String>(),
            "trusted-device enrollment failed"
        );
        return;
    }
    let payload = match response.json::<TrustedDeviceEnrollResponse>().await {
        Ok(payload) => payload,
        Err(err) => {
            tracing::debug!(error = %err, "trusted-device enrollment response parse failed");
            return;
        }
    };
    let Some(token) = payload
        .device_token
        .filter(|token| !token.trim().is_empty())
    else {
        tracing::debug!("trusted-device enrollment response missing device_token");
        return;
    };
    match crate::auth::storage::store_trusted_device_token(&token).await {
        Ok(()) => {
            clear_trusted_device_token_cache();
            tracing::debug!(
                device_id = payload.device_id.as_deref().unwrap_or("unknown"),
                "trusted-device enrolled"
            );
        }
        Err(err) => {
            tracing::debug!(error = %err, "trusted-device token persist failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::ENV_LOCK;

    #[tokio::test]
    async fn gate_disabled_hides_env_token() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_FEATURES");
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_TENGU_SESSIONS_ELEVATED_AUTH_ENFORCEMENT");
        std::env::set_var("CLAUDE_TRUSTED_DEVICE_TOKEN", "td_env");
        clear_trusted_device_token_cache();
        assert_eq!(get_trusted_device_token().await, None);
        std::env::remove_var("CLAUDE_TRUSTED_DEVICE_TOKEN");
    }

    #[tokio::test]
    async fn env_token_wins_when_gate_enabled() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(
            "CLAUDE_CODE_GROWTHBOOK_FEATURES",
            r#"{"tengu_sessions_elevated_auth_enforcement":true}"#,
        );
        std::env::set_var("CLAUDE_TRUSTED_DEVICE_TOKEN", "td_env");
        clear_trusted_device_token_cache();
        assert_eq!(get_trusted_device_token().await.as_deref(), Some("td_env"));
        std::env::remove_var("CLAUDE_TRUSTED_DEVICE_TOKEN");
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_FEATURES");
    }

    #[tokio::test]
    async fn enrollment_skips_when_env_token_present() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var(
            "CLAUDE_CODE_GROWTHBOOK_FEATURES",
            r#"{"tengu_sessions_elevated_auth_enforcement":true}"#,
        );
        std::env::set_var("CLAUDE_TRUSTED_DEVICE_TOKEN", "td_env");
        enroll_trusted_device("oauth-token").await;
        std::env::remove_var("CLAUDE_TRUSTED_DEVICE_TOKEN");
        std::env::remove_var("CLAUDE_CODE_GROWTHBOOK_FEATURES");
    }
}
