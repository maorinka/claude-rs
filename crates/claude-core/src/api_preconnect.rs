//! Fire-and-forget API preconnect to overlap TCP + TLS handshake
//! with startup work.
//!
//! Port of TS `utils/apiPreconnect.ts:1-72`.
//!
//! The TCP + TLS handshake to `api.anthropic.com` costs ~100-200ms
//! and normally blocks inside the first real API call. Doing a HEAD
//! request during init lets the handshake happen in parallel with
//! action-handler work — ~100ms in print mode, unbounded "user is
//! typing" window in interactive mode.
//!
//! `reqwest`'s default client reuses the connection pool per-client,
//! so the real API request will land on the warmed connection as
//! long as the same client instance is reused.
//!
//! Skip rules match TS `apiPreconnect.ts:35-54`:
//! - Cloud providers (Bedrock / Vertex / Foundry) have different
//!   endpoints + auth.
//! - Proxy / mTLS / unix socket configurations use custom
//!   dispatchers that wouldn't share the warmed pool.
//!
//! Idempotent: the first call fires, subsequent calls no-op.

use crate::constants::oauth::get_oauth_config;
use crate::errors_util::is_env_truthy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Cloud-provider env vars that invalidate the preconnect — the real
/// API request will go to a completely different host with different
/// auth, so warming the anthropic.com pool would be wasted work.
/// TS `apiPreconnect.ts:37-41`.
const CLOUD_PROVIDER_FLAGS: &[&str] = &[
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

/// Proxy / mTLS / socket env vars that make the warm pool useless
/// because the SDK plugs in a custom transport/dispatcher that
/// doesn't share the default pool. TS `apiPreconnect.ts:45-53`.
const CUSTOM_TRANSPORT_ENVS: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ANTHROPIC_UNIX_SOCKET",
    "CLAUDE_CODE_CLIENT_CERT",
    "CLAUDE_CODE_CLIENT_KEY",
];

static FIRED: AtomicBool = AtomicBool::new(false);

/// Decide whether a preconnect should actually fire, exposing the
/// decision for testing + so callers can log "skipped because …"
/// without a round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreconnectDecision {
    /// Will fire to this URL.
    Fire { base_url: String },
    /// Already fired once in this process.
    AlreadyFired,
    /// Cloud-provider env set (Bedrock / Vertex / Foundry).
    SkippedCloudProvider,
    /// Proxy / mTLS / socket env set.
    SkippedCustomTransport,
    /// OAuth config resolution failed — can't resolve a URL to fire.
    BaseUrlUnavailable,
}

/// Build the decision without side effects. Exposed so callers can
/// unit-test the env-gate logic + log the reason for skipping.
pub fn decide_preconnect() -> PreconnectDecision {
    if FIRED.load(Ordering::Relaxed) {
        return PreconnectDecision::AlreadyFired;
    }
    if CLOUD_PROVIDER_FLAGS.iter().any(|k| is_env_truthy(k)) {
        return PreconnectDecision::SkippedCloudProvider;
    }
    if CUSTOM_TRANSPORT_ENVS
        .iter()
        .any(|k| std::env::var(k).ok().filter(|v| !v.is_empty()).is_some())
    {
        return PreconnectDecision::SkippedCustomTransport;
    }

    // TS: `ANTHROPIC_BASE_URL || getOauthConfig().BASE_API_URL`.
    let base_url = match std::env::var("ANTHROPIC_BASE_URL") {
        Ok(v) if !v.is_empty() => v,
        _ => match get_oauth_config() {
            Ok(cfg) => cfg.base_api_url,
            Err(_) => return PreconnectDecision::BaseUrlUnavailable,
        },
    };

    PreconnectDecision::Fire { base_url }
}

/// Fire the preconnect if the gate approves. Returns the decision
/// so the caller can log / record it. The HEAD request itself is
/// spawned onto the caller's runtime and detached — errors are
/// swallowed because the real request handles its own handshake on
/// failure.
///
/// Idempotent: the `FIRED` latch is set on the first Fire-producing
/// call, so repeated invocations become `AlreadyFired`.
pub fn preconnect_anthropic_api(client: reqwest::Client) -> PreconnectDecision {
    let decision = decide_preconnect();
    if let PreconnectDecision::Fire { ref base_url } = decision {
        // Set the latch BEFORE spawning so concurrent calls between
        // the read and the swap can't both dispatch.
        if FIRED.swap(true, Ordering::AcqRel) {
            return PreconnectDecision::AlreadyFired;
        }
        let url = base_url.clone();
        tokio::spawn(async move {
            // TS uses HEAD with AbortSignal.timeout(10_000). reqwest
            // takes the timeout per-request. Response body discarded.
            let _ = client
                .head(&url)
                .timeout(Duration::from_secs(10))
                .send()
                .await;
        });
    }
    decision
}

/// Test-only hook to reset the latch. TS has no equivalent — there's
/// no test harness in the TS source — but Rust unit tests need this
/// for the idempotency coverage.
#[cfg(test)]
fn reset_fired_latch() {
    FIRED.store(false, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn clear_env() {
        for k in CLOUD_PROVIDER_FLAGS
            .iter()
            .chain(CUSTOM_TRANSPORT_ENVS.iter())
            .chain(&["ANTHROPIC_BASE_URL"])
        {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn bedrock_env_skips() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("CLAUDE_CODE_USE_BEDROCK", "1");
        assert_eq!(decide_preconnect(), PreconnectDecision::SkippedCloudProvider);
        std::env::remove_var("CLAUDE_CODE_USE_BEDROCK");
    }

    #[test]
    fn vertex_env_skips() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("CLAUDE_CODE_USE_VERTEX", "1");
        assert_eq!(decide_preconnect(), PreconnectDecision::SkippedCloudProvider);
        std::env::remove_var("CLAUDE_CODE_USE_VERTEX");
    }

    #[test]
    fn foundry_env_skips() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("CLAUDE_CODE_USE_FOUNDRY", "true");
        assert_eq!(decide_preconnect(), PreconnectDecision::SkippedCloudProvider);
        std::env::remove_var("CLAUDE_CODE_USE_FOUNDRY");
    }

    #[test]
    fn https_proxy_skips() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("HTTPS_PROXY", "http://proxy.example:8080");
        assert_eq!(
            decide_preconnect(),
            PreconnectDecision::SkippedCustomTransport
        );
        std::env::remove_var("HTTPS_PROXY");
    }

    #[test]
    fn http_proxy_lowercase_skips() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("http_proxy", "http://proxy.example:8080");
        assert_eq!(
            decide_preconnect(),
            PreconnectDecision::SkippedCustomTransport
        );
        std::env::remove_var("http_proxy");
    }

    #[test]
    fn anthropic_unix_socket_skips() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("ANTHROPIC_UNIX_SOCKET", "/var/run/claude.sock");
        assert_eq!(
            decide_preconnect(),
            PreconnectDecision::SkippedCustomTransport
        );
        std::env::remove_var("ANTHROPIC_UNIX_SOCKET");
    }

    #[test]
    fn mtls_cert_skips() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("CLAUDE_CODE_CLIENT_CERT", "/etc/claude/client.crt");
        assert_eq!(
            decide_preconnect(),
            PreconnectDecision::SkippedCustomTransport
        );
        std::env::remove_var("CLAUDE_CODE_CLIENT_CERT");
    }

    #[test]
    fn empty_proxy_var_does_not_skip() {
        // TS: `process.env.HTTPS_PROXY` is truthy only for non-empty
        // strings (empty string is falsy in JS). Rust mirrors the
        // same "empty string = absent" semantic.
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("HTTPS_PROXY", "");
        let d = decide_preconnect();
        assert!(
            matches!(d, PreconnectDecision::Fire { .. }),
            "empty proxy should not trigger skip, got {d:?}"
        );
        std::env::remove_var("HTTPS_PROXY");
    }

    #[test]
    fn clean_env_fires_with_oauth_base_url() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        let d = decide_preconnect();
        // Ambient OAuth-config resolution: on a vanilla CI host this
        // should produce the prod URL; but any legit base_url means
        // the gate approved.
        match d {
            PreconnectDecision::Fire { base_url } => {
                assert!(base_url.starts_with("http"));
            }
            other => panic!("expected Fire, got {other:?}"),
        }
    }

    #[test]
    fn anthropic_base_url_override_wins() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("ANTHROPIC_BASE_URL", "https://gateway.example.com");
        let d = decide_preconnect();
        assert_eq!(
            d,
            PreconnectDecision::Fire {
                base_url: "https://gateway.example.com".into()
            }
        );
        std::env::remove_var("ANTHROPIC_BASE_URL");
    }

    #[tokio::test]
    async fn preconnect_sets_fired_latch_and_subsequent_is_already_fired() {
        let _g = lock_env();
        clear_env();
        reset_fired_latch();
        std::env::set_var("ANTHROPIC_BASE_URL", "http://127.0.0.1:1");
        let client = reqwest::Client::new();
        let first = preconnect_anthropic_api(client.clone());
        assert!(matches!(first, PreconnectDecision::Fire { .. }));
        let second = preconnect_anthropic_api(client);
        assert_eq!(second, PreconnectDecision::AlreadyFired);
        std::env::remove_var("ANTHROPIC_BASE_URL");
    }
}
