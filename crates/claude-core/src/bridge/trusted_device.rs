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
}
