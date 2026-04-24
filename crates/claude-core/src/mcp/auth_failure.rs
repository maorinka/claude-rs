//! Shared auth-failure handling for remote MCP transports.
//!
//! Gap-fill ticket **G16** (scoped to the sync handler). Ports
//! `handleRemoteAuthFailure` from
//! `src/services/mcp/client.ts:340-361`:
//!
//! 1. Emit telemetry (`tengu_mcp_server_needs_auth`).
//! 2. Write the needs-auth entry to the on-disk cache so
//!    subsequent reconnect attempts within the 15-minute TTL
//!    short-circuit (G2's `auth_cache`).
//! 3. Return a `needs-auth` `McpServerConnection`.
//!
//! The OAuth `createClaudeAiProxyFetch` wrapper from the same TS
//! region isn't ported yet — it depends on the keychain and
//! OAuth-refresh subsystems that haven't been ported to Rust.
//! Landed as a follow-up once those are wired.
//!
//! `tengu_mcp_server_needs_auth` telemetry is surfaced via
//! `tracing::info!` with structured fields. When the analytics
//! subsystem lands on the Rust side, upgrade the `info!` to a
//! proper event emission.

use tracing::{debug, info};

use crate::mcp::auth_cache::set_mcp_auth_cache_entry;
use crate::mcp::types::{McpConnectionStatus, McpServerConnection, ScopedMcpServerConfig};

/// Transport flavours that can encounter a remote-auth failure.
/// Matches the TS `'sse' | 'http' | 'claudeai-proxy'` union at
/// `client.ts:343`, plus the `Ws` variant added with G18b for
/// WebSocket transports (no direct TS correspondence — TS
/// handles `ws` / `ws-ide` auth failures inline rather than
/// through `handleRemoteAuthFailure`, but the telemetry surface
/// needs distinct classification on the Rust side so operators
/// can tell `ws` failures apart from `sse` ones).
///
/// `ClaudeAiProxy` isn't yet a first-class Rust `McpServerConfig`
/// variant; it's included here so downstream auth code that
/// drives the claude.ai proxy path can log the same label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteTransportKind {
    Sse,
    Http,
    ClaudeAiProxy,
    /// G18b: WebSocket auth failures during the handshake.
    Ws,
}

impl RemoteTransportKind {
    /// The human label used in debug logs — matches TS
    /// `client.ts:350-354` for sse/http/claudeai-proxy; Rust
    /// adds `"WebSocket"` for the WS case.
    pub fn label(self) -> &'static str {
        match self {
            RemoteTransportKind::Sse => "SSE",
            RemoteTransportKind::Http => "HTTP",
            RemoteTransportKind::ClaudeAiProxy => "claude.ai proxy",
            RemoteTransportKind::Ws => "WebSocket",
        }
    }

    /// The analytics tag used in `tengu_mcp_server_needs_auth`.
    /// Matches TS `transportType` string in the event payload
    /// for the three TS-present kinds; WS uses `"ws"` so the
    /// tag stays single-token like the others.
    pub fn analytics_tag(self) -> &'static str {
        match self {
            RemoteTransportKind::Sse => "sse",
            RemoteTransportKind::Http => "http",
            RemoteTransportKind::ClaudeAiProxy => "claudeai-proxy",
            RemoteTransportKind::Ws => "ws",
        }
    }
}

/// Shared handler for SSE / HTTP / claude.ai-proxy auth failures
/// during connect:
///
/// 1. Emit `tengu_mcp_server_needs_auth` telemetry (currently a
///    structured `tracing::info!` pending the analytics
///    subsystem port).
/// 2. Write the needs-auth entry to the disk cache so subsequent
///    reconnects within the 15-minute TTL short-circuit.
/// 3. Return an `McpServerConnection` with
///    `McpConnectionStatus::NeedsAuth` so the UI shows the
///    re-auth prompt.
///
/// Byte-for-byte port of TS `handleRemoteAuthFailure` at
/// `client.ts:340-361`.
pub fn handle_remote_auth_failure(
    name: &str,
    server_ref: &ScopedMcpServerConfig,
    transport: RemoteTransportKind,
) -> McpServerConnection {
    info!(
        event = "tengu_mcp_server_needs_auth",
        server = name,
        transport_type = transport.analytics_tag(),
        "MCP remote server needs re-authorization"
    );
    debug!(
        server = name,
        "Authentication required for {} server",
        transport.label()
    );
    set_mcp_auth_cache_entry(name);
    McpServerConnection {
        name: name.to_string(),
        status: McpConnectionStatus::NeedsAuth,
        config: server_ref.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::auth_cache::{clear_mcp_auth_cache, is_mcp_auth_cached, shared_test_lock};
    use crate::mcp::types::{
        ConfigScope, McpHttpServerConfig, McpServerConfig, McpSseServerConfig,
        ScopedMcpServerConfig,
    };

    // Share the lock with `auth_cache::tests` — both modules touch
    // `CLAUDE_CONFIG_DIR` and the on-disk auth cache file, so
    // running their tests concurrently from separate module locks
    // would race and flake.
    fn t_lock() -> &'static std::sync::Mutex<()> {
        shared_test_lock()
    }

    struct ConfigHomeGuard {
        prev: Option<std::ffi::OsString>,
    }

    impl ConfigHomeGuard {
        fn set(path: &std::path::Path) -> Self {
            let prev = std::env::var_os("CLAUDE_CONFIG_DIR");
            std::env::set_var("CLAUDE_CONFIG_DIR", path);
            Self { prev }
        }
    }

    impl Drop for ConfigHomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }
    }

    fn sse_config() -> ScopedMcpServerConfig {
        ScopedMcpServerConfig {
            config: McpServerConfig::Sse(McpSseServerConfig {
                url: "https://example.invalid".into(),
                headers: None,
            }),
            scope: ConfigScope::Project,
        }
    }

    fn http_config() -> ScopedMcpServerConfig {
        ScopedMcpServerConfig {
            config: McpServerConfig::Http(McpHttpServerConfig {
                url: "https://example.invalid".into(),
                headers: None,
            }),
            scope: ConfigScope::Project,
        }
    }

    #[test]
    fn transport_labels_match_ts() {
        // TS `client.ts:350-354` label map.
        assert_eq!(RemoteTransportKind::Sse.label(), "SSE");
        assert_eq!(RemoteTransportKind::Http.label(), "HTTP");
        assert_eq!(
            RemoteTransportKind::ClaudeAiProxy.label(),
            "claude.ai proxy"
        );
    }

    #[test]
    fn analytics_tags_are_wire_format() {
        // The event payload field `transportType` must match TS
        // `client.ts:346-347` — lowercase single-token names.
        assert_eq!(RemoteTransportKind::Sse.analytics_tag(), "sse");
        assert_eq!(RemoteTransportKind::Http.analytics_tag(), "http");
        assert_eq!(
            RemoteTransportKind::ClaudeAiProxy.analytics_tag(),
            "claudeai-proxy"
        );
        // G18b: WS classification, Rust-side addition.
        assert_eq!(RemoteTransportKind::Ws.analytics_tag(), "ws");
    }

    #[test]
    fn ws_label_is_websocket() {
        // G18b addition: the `Ws` variant should log as
        // "WebSocket" not "HTTP" — ensures operators see the
        // correct transport flavour in debug logs.
        assert_eq!(RemoteTransportKind::Ws.label(), "WebSocket");
    }

    #[test]
    fn handle_failure_writes_auth_cache_and_returns_needs_auth() {
        let _g = t_lock().lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let _guard = ConfigHomeGuard::set(tmp.path());
        clear_mcp_auth_cache();

        let cfg = sse_config();
        let conn = handle_remote_auth_failure("jira", &cfg, RemoteTransportKind::Sse);

        // Returned status is NeedsAuth.
        assert_eq!(conn.name, "jira");
        assert!(matches!(conn.status, McpConnectionStatus::NeedsAuth));
        // Config round-tripped into the connection.
        match &conn.config.config {
            McpServerConfig::Sse(sse) => assert_eq!(sse.url, "https://example.invalid"),
            other => panic!("expected SSE config, got {:?}", other),
        }

        // Side effect: cache entry now present.
        assert!(
            is_mcp_auth_cached("jira"),
            "handle_remote_auth_failure must write a cache entry"
        );
    }

    #[test]
    fn handle_failure_marks_http_transport_too() {
        let _g = t_lock().lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let _guard = ConfigHomeGuard::set(tmp.path());
        clear_mcp_auth_cache();

        let cfg = http_config();
        let conn = handle_remote_auth_failure("vault", &cfg, RemoteTransportKind::Http);
        assert!(matches!(conn.status, McpConnectionStatus::NeedsAuth));
        assert!(is_mcp_auth_cached("vault"));
    }

    #[test]
    fn needs_auth_status_serialises_with_wire_tag() {
        // Guards the `#[serde(rename = "needs-auth")]` tag so
        // status JSON on the wire matches TS's string literal.
        let conn = McpServerConnection {
            name: "x".into(),
            status: McpConnectionStatus::NeedsAuth,
            config: sse_config(),
        };
        // We only serialize the status portion since the full
        // struct doesn't derive Serialize (no accidental persist).
        let json = serde_json::to_string(&conn.status).expect("serialize");
        assert!(
            json.contains("\"type\":\"needs-auth\""),
            "status must serialize with 'needs-auth' tag; got {}",
            json
        );
    }
}
