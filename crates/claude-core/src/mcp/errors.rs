//! Typed errors for MCP client operations.
//!
//! Ports the three error classes from `src/services/mcp/client.ts:152-186`:
//!
//! - `McpAuthError` — a tool call failed because the server rejected
//!   authentication (e.g. an expired OAuth token returning 401). The
//!   tool-execution layer catches this to flip the client status to
//!   `needs-auth` and surface a re-auth prompt.
//! - `McpSessionExpiredError` — the SDK reported HTTP 404 + JSON-RPC
//!   code `-32001`, meaning the server dropped the session. Callers
//!   should clear their client cache and retry with a fresh client.
//! - `McpToolCallError` — the server returned `isError: true` on a
//!   tool result. Carries the result's `_meta` field so SDK consumers
//!   can still surface it (per the MCP spec, `_meta` is on the base
//!   Result type and is valid on error results). The TS name has a
//!   `_I_VERIFIED_THIS_IS_NOT_CODE_OR_FILEPATHS` suffix to dodge a
//!   naive regex scanner; the Rust side doesn't share that constraint
//!   so the type is named cleanly.
//!
//! The predicate `is_mcp_session_expired_error` that TS exports
//! alongside these types lives in `mcp::helpers` (ported separately
//! under gap G1).

use std::collections::BTreeMap;

/// A tool call failed because the MCP server rejected authentication
/// (typically HTTP 401 against a remote OAuth-gated server). The
/// server name is preserved so the tool layer can flip the right
/// connection's status to `needs-auth`.
///
/// TS parity: `McpAuthError` at `client.ts:152-159`.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct McpAuthError {
    pub server_name: String,
    pub message: String,
}

impl McpAuthError {
    pub fn new(server_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            server_name: server_name.into(),
            message: message.into(),
        }
    }
}

/// The SDK reported HTTP 404 + JSON-RPC `-32001` — the server
/// dropped our session. The caller should clear its connection
/// cache and retry via `ensure_connected_client`. The message is
/// built server-side so downstream formatters can use it
/// verbatim.
///
/// TS parity: `McpSessionExpiredError` at `client.ts:165-170` with
/// its `MCP server "${serverName}" session expired` template.
#[derive(Debug, Clone, thiserror::Error)]
#[error("MCP server \"{server_name}\" session expired")]
pub struct McpSessionExpiredError {
    pub server_name: String,
}

impl McpSessionExpiredError {
    pub fn new(server_name: impl Into<String>) -> Self {
        Self {
            server_name: server_name.into(),
        }
    }
}

/// The server returned a tool result with `isError: true`. Carries:
///
/// - `message`: the human-facing error text the tool surfaced.
/// - `telemetry_message`: a scrubbed variant for analytics. TS
///   distinguishes these via a `TelemetrySafeError` base class; Rust
///   stores both fields directly on this concrete type.
/// - `mcp_meta`: the server's `_meta` on the result. Per the MCP
///   spec, `_meta` is on the base `Result` type and is valid on
///   error results; SDK consumers can still read it.
///
/// TS parity: the class at `client.ts:177-186`. The `_meta` shape
/// on the wire is an arbitrary JSON object (keys under the caller's
/// control); we store a `BTreeMap<String, serde_json::Value>` for
/// predictable ordering when the map appears in logs.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct McpToolCallError {
    pub message: String,
    pub telemetry_message: String,
    pub mcp_meta: Option<BTreeMap<String, serde_json::Value>>,
}

impl McpToolCallError {
    pub fn new(message: impl Into<String>, telemetry_message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            telemetry_message: telemetry_message.into(),
            mcp_meta: None,
        }
    }

    pub fn with_meta(
        message: impl Into<String>,
        telemetry_message: impl Into<String>,
        meta: BTreeMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            message: message.into(),
            telemetry_message: telemetry_message.into(),
            mcp_meta: Some(meta),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auth_error_display_is_the_message() {
        let e = McpAuthError::new("jira", "token rejected");
        assert_eq!(e.to_string(), "token rejected");
        assert_eq!(e.server_name, "jira");
    }

    #[test]
    fn session_expired_display_matches_ts_template() {
        // TS: `MCP server "${serverName}" session expired`
        let e = McpSessionExpiredError::new("github");
        assert_eq!(e.to_string(), "MCP server \"github\" session expired");
    }

    #[test]
    fn tool_call_error_display_uses_human_message() {
        let e = McpToolCallError::new("rate limited", "rate_limited_telemetry");
        assert_eq!(e.to_string(), "rate limited");
        // The telemetry message is a distinct, accessible field.
        assert_eq!(e.telemetry_message, "rate_limited_telemetry");
        assert!(e.mcp_meta.is_none());
    }

    #[test]
    fn tool_call_error_carries_mcp_meta_when_present() {
        let mut meta = BTreeMap::new();
        meta.insert("cause".to_string(), json!("upstream_timeout"));
        meta.insert("retry_after_s".to_string(), json!(30));
        let e = McpToolCallError::with_meta("upstream timeout", "u_tmo", meta);
        let m = e.mcp_meta.as_ref().expect("meta should be populated");
        assert_eq!(m["cause"], json!("upstream_timeout"));
        assert_eq!(m["retry_after_s"], json!(30));
    }

    #[test]
    fn errors_are_send_sync_clone() {
        // Sanity: these types need to survive crossing task boundaries
        // and being stored in analytics pipelines.
        fn assert_bounds<T: Send + Sync + Clone + std::error::Error + 'static>() {}
        assert_bounds::<McpAuthError>();
        assert_bounds::<McpSessionExpiredError>();
        assert_bounds::<McpToolCallError>();
    }
}
