//! Pure, stateless helpers extracted from `src/services/mcp/client.ts`.
//!
//! Gap-fill ticket **G1** in the MCP client plan. These are the
//! small leaf utilities the rest of the MCP subsystem leans on:
//! timeout env-var readers, batch-size env-var readers, the
//! local-vs-remote server predicate, the IDE tool allow-list filter,
//! and the session-expiry error classifier.
//!
//! Keeping these co-located in `mcp::helpers` (rather than inlined)
//! mirrors the TS structure and gives the rest of the MCP gap-fill
//! tickets a single import surface.

use crate::mcp::types::{
    McpServerConfig, McpToolDefinition, ScopedMcpServerConfig, MCP_CONNECTION_TIMEOUT_MS,
    MCP_TOOL_TIMEOUT_MS,
};

// ─── Constants ───────────────────────────────────────────────────────
//
// `MCP_TOOL_TIMEOUT_MS`, `MAX_MCP_DESCRIPTION_LENGTH`, and
// `MCP_CONNECTION_TIMEOUT_MS` live in `mcp::types` — re-exporting
// here would hide the single-source-of-truth by letting callers drift
// between `helpers::X` and `types::X`. Import where needed.

/// Per-request timeout for MCP HTTP/SSE transports (ms). Applied by
/// `wrap_fetch_with_timeout` (gap G3). Mirrors TS
/// `MCP_REQUEST_TIMEOUT_MS` at `client.ts:463`.
pub const MCP_REQUEST_TIMEOUT_MS: u64 = 60_000;

/// Required `Accept` header value for MCP Streamable HTTP POSTs.
/// Servers enforcing the spec strictly reject requests missing this
/// with HTTP 406. Mirrors TS `MCP_STREAMABLE_HTTP_ACCEPT` at
/// `client.ts:471`.
/// See: <https://modelcontextprotocol.io/specification/2025-03-26/basic/transports#sending-messages-to-the-server>
pub const MCP_STREAMABLE_HTTP_ACCEPT: &str = "application/json, text/event-stream";

/// IDE tools that pass the `isIncludedMcpTool` filter. Every other
/// `mcp__ide__*` tool is dropped before the tool list reaches the
/// model. Mirrors TS `ALLOWED_IDE_TOOLS` at `client.ts:568`.
pub const ALLOWED_IDE_TOOLS: &[&str] =
    &["mcp__ide__executeCode", "mcp__ide__getDiagnostics"];

// ─── Env-var readers ─────────────────────────────────────────────────

/// Parse a u64 env-var; return `default` on empty / missing / parse
/// error. Mirrors TS's `parseInt(process.env.X || '', 10) || default`
/// pattern.
fn env_u64_or(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0) // TS `|| default` treats 0 as falsy too
        .unwrap_or(default)
}

fn env_usize_or(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

/// Read `MCP_TOOL_TIMEOUT` (ms); fall back to
/// `MCP_TOOL_TIMEOUT_MS` (defined in `mcp::types`). Mirrors TS
/// `getMcpToolTimeoutMs` at `client.ts:224-229`.
pub fn get_mcp_tool_timeout_ms() -> u64 {
    env_u64_or("MCP_TOOL_TIMEOUT", MCP_TOOL_TIMEOUT_MS)
}

/// Read `MCP_TIMEOUT` (ms, connection timeout); fall back to
/// `MCP_CONNECTION_TIMEOUT_MS` (30_000, defined in `mcp::types`).
/// Mirrors TS `getConnectionTimeoutMs` at `client.ts:456-458`.
pub fn get_connection_timeout_ms() -> u64 {
    env_u64_or("MCP_TIMEOUT", MCP_CONNECTION_TIMEOUT_MS)
}

/// Read `MCP_SERVER_CONNECTION_BATCH_SIZE`; fall back to 3. Mirrors
/// TS `getMcpServerConnectionBatchSize` at `client.ts:552-554`.
pub fn get_mcp_server_connection_batch_size() -> usize {
    env_usize_or("MCP_SERVER_CONNECTION_BATCH_SIZE", 3)
}

/// Read `MCP_REMOTE_SERVER_CONNECTION_BATCH_SIZE`; fall back to 20.
/// Mirrors TS `getRemoteMcpServerConnectionBatchSize` at
/// `client.ts:556-561`.
pub fn get_remote_mcp_server_connection_batch_size() -> usize {
    env_usize_or("MCP_REMOTE_SERVER_CONNECTION_BATCH_SIZE", 20)
}

// ─── Server / tool predicates ────────────────────────────────────────

/// `true` when the server is in-process (stdio or future `sdk`
/// variant). Remote transports (SSE, Streamable HTTP, WS) return
/// `false`. Mirrors TS `isLocalMcpServer` at `client.ts:563-565`.
///
/// TS also accepts `type === undefined` as local (stdio is the
/// default). The Rust port encodes transport as a required enum
/// discriminator, so "no type" is impossible at the type level.
///
/// NOTE: TS also returns `true` for `config.type === 'sdk'`. The
/// Rust `McpServerConfig` enum does not yet have an `Sdk` variant
/// (pending gap G17). When that lands, extend this match.
pub fn is_local_mcp_server(config: &ScopedMcpServerConfig) -> bool {
    matches!(config.config, McpServerConfig::Stdio(_))
}

/// Deterministic cache key for a server connection: `"${name}-${json(serverRef)}"`.
/// Mirrors TS `getServerCacheKey` at `client.ts:581-586`. The JSON
/// serialization matches TS's `JSON.stringify(ScopedMcpServerConfig)`
/// output because both sides use flattened config + `type`-tagged
/// variants. Falls back to the debug form if serialization somehow
/// fails — a cache-key collision is preferable to a panic here.
pub fn get_server_cache_key(name: &str, server_ref: &ScopedMcpServerConfig) -> String {
    let body = serde_json::to_string(server_ref)
        .unwrap_or_else(|_| format!("{:?}", server_ref));
    format!("{}-{}", name, body)
}

/// Filter for the tool list sent to the model. IDE MCP servers
/// expose many tools internally; only a short allow-list reaches
/// the model. Non-IDE tools pass through unconditionally. Mirrors
/// TS `isIncludedMcpTool` at `client.ts:569-573`.
pub fn is_included_mcp_tool(tool: &McpToolDefinition) -> bool {
    if !tool.name.starts_with("mcp__ide__") {
        return true;
    }
    ALLOWED_IDE_TOOLS.iter().any(|&t| t == tool.name)
}

// ─── Error classifiers ───────────────────────────────────────────────

/// Detect whether an error message carries the "session expired"
/// signature: HTTP 404 paired with JSON-RPC `-32001`. Both must
/// match to avoid false positives from generic 404s (wrong URL,
/// server gone). Mirrors TS `isMcpSessionExpiredError` at
/// `client.ts:193-206`.
///
/// TS reads `error.code` (attached by the MCP SDK's HTTP transport)
/// *and* the error message body. The Rust side doesn't yet have a
/// structured transport-error type, so both inputs are passed in:
/// - `http_status`: the HTTP status the SDK attached (`Some(404)`
///   when known).
/// - `message`: the full error `Display` string; must contain the
///   serialized `-32001` JSON-RPC body to confirm.
///
/// The JSON-RPC code substring is matched in both dense and
/// space-separated forms (`"code":-32001` and `"code": -32001`) to
/// survive any SDK pretty-printing.
pub fn is_mcp_session_expired_error(http_status: Option<u16>, message: &str) -> bool {
    if http_status != Some(404) {
        return false;
    }
    message.contains("\"code\":-32001") || message.contains("\"code\": -32001")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::types::{
        ConfigScope, McpHttpServerConfig, McpServerConfig, McpSseServerConfig,
        McpStdioServerConfig, ScopedMcpServerConfig,
    };
    use std::sync::Mutex;

    // Env mutation is global; serialise.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn scoped(cfg: McpServerConfig) -> ScopedMcpServerConfig {
        ScopedMcpServerConfig {
            config: cfg,
            scope: ConfigScope::Project,
        }
    }

    // ─── env helpers ────────────────────────────────────────────

    #[test]
    fn timeout_ms_falls_back_to_default() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("MCP_TOOL_TIMEOUT");
        assert_eq!(get_mcp_tool_timeout_ms(), MCP_TOOL_TIMEOUT_MS);
    }

    #[test]
    fn timeout_ms_reads_env_when_set() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("MCP_TOOL_TIMEOUT", "5000");
        assert_eq!(get_mcp_tool_timeout_ms(), 5000);
        std::env::remove_var("MCP_TOOL_TIMEOUT");
    }

    #[test]
    fn timeout_ms_rejects_zero_and_garbage() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // TS `parseInt('0',10) || DEFAULT` returns DEFAULT (0 is falsy).
        std::env::set_var("MCP_TOOL_TIMEOUT", "0");
        assert_eq!(get_mcp_tool_timeout_ms(), MCP_TOOL_TIMEOUT_MS);
        std::env::set_var("MCP_TOOL_TIMEOUT", "not-a-number");
        assert_eq!(get_mcp_tool_timeout_ms(), MCP_TOOL_TIMEOUT_MS);
        std::env::set_var("MCP_TOOL_TIMEOUT", "");
        assert_eq!(get_mcp_tool_timeout_ms(), MCP_TOOL_TIMEOUT_MS);
        std::env::remove_var("MCP_TOOL_TIMEOUT");
    }

    #[test]
    fn connection_timeout_defaults_to_30s() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("MCP_TIMEOUT");
        assert_eq!(get_connection_timeout_ms(), 30_000);
    }

    #[test]
    fn batch_sizes_defaults() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("MCP_SERVER_CONNECTION_BATCH_SIZE");
        std::env::remove_var("MCP_REMOTE_SERVER_CONNECTION_BATCH_SIZE");
        assert_eq!(get_mcp_server_connection_batch_size(), 3);
        assert_eq!(get_remote_mcp_server_connection_batch_size(), 20);
    }

    #[test]
    fn batch_sizes_reject_zero_and_garbage() {
        // Codex CR gap: the `usize` readers also need zero-is-falsy
        // coverage so JS `|| DEFAULT` parity doesn't regress silently.
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("MCP_SERVER_CONNECTION_BATCH_SIZE", "0");
        assert_eq!(get_mcp_server_connection_batch_size(), 3);
        std::env::set_var("MCP_SERVER_CONNECTION_BATCH_SIZE", "nope");
        assert_eq!(get_mcp_server_connection_batch_size(), 3);
        std::env::set_var("MCP_REMOTE_SERVER_CONNECTION_BATCH_SIZE", "0");
        assert_eq!(get_remote_mcp_server_connection_batch_size(), 20);
        std::env::remove_var("MCP_SERVER_CONNECTION_BATCH_SIZE");
        std::env::remove_var("MCP_REMOTE_SERVER_CONNECTION_BATCH_SIZE");
    }

    // ─── server predicate ────────────────────────────────────────

    #[test]
    fn local_server_stdio_is_local() {
        let cfg = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "echo".into(),
            args: vec![],
            env: None,
        }));
        assert!(is_local_mcp_server(&cfg));
    }

    #[test]
    fn local_server_remote_types_are_not_local() {
        let sse = scoped(McpServerConfig::Sse(McpSseServerConfig {
            url: "https://x".into(),
            headers: None,
        }));
        assert!(!is_local_mcp_server(&sse));
        let http = scoped(McpServerConfig::Http(McpHttpServerConfig {
            url: "https://x".into(),
            headers: None,
        }));
        assert!(!is_local_mcp_server(&http));
    }

    // ─── tool predicate ──────────────────────────────────────────

    fn tool_named(name: &str) -> McpToolDefinition {
        McpToolDefinition {
            name: name.to_string(),
            description: None,
            input_schema: Some(serde_json::json!({})),
        }
    }

    #[test]
    fn server_cache_key_is_name_dash_json() {
        let cfg = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "echo".into(),
            args: vec![],
            env: None,
        }));
        let k = get_server_cache_key("foo", &cfg);
        // Format: "<name>-<serde_json>". Exact JSON body depends on
        // the derived Serialize but is deterministic for the same
        // input — verify prefix + embedded type tag.
        assert!(k.starts_with("foo-{"), "expected 'foo-{{...}}', got {}", k);
        assert!(k.contains("\"type\":\"stdio\""));
        assert!(k.contains("\"command\":\"echo\""));
        // Same config → same key (caches must hit).
        let cfg2 = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "echo".into(),
            args: vec![],
            env: None,
        }));
        assert_eq!(k, get_server_cache_key("foo", &cfg2));
        // Different name → different key.
        assert_ne!(k, get_server_cache_key("bar", &cfg));
    }

    #[test]
    fn included_tool_non_ide_passes() {
        assert!(is_included_mcp_tool(&tool_named("mcp__jira__search")));
        assert!(is_included_mcp_tool(&tool_named("plain_name")));
    }

    #[test]
    fn included_tool_ide_allowlist() {
        assert!(is_included_mcp_tool(&tool_named("mcp__ide__executeCode")));
        assert!(is_included_mcp_tool(&tool_named("mcp__ide__getDiagnostics")));
    }

    #[test]
    fn included_tool_ide_other_tools_are_dropped() {
        assert!(!is_included_mcp_tool(&tool_named("mcp__ide__listOpenFiles")));
        assert!(!is_included_mcp_tool(&tool_named("mcp__ide__closeTab")));
        assert!(!is_included_mcp_tool(&tool_named("mcp__ide__")));
    }

    // ─── session-expired classifier ──────────────────────────────

    #[test]
    fn session_expired_requires_404_plus_jsonrpc_code() {
        let body = r#"{"error":{"code":-32001,"message":"Session not found"}}"#;
        assert!(is_mcp_session_expired_error(Some(404), body));
        // Space-separated code survives SDK pretty-printing.
        assert!(is_mcp_session_expired_error(
            Some(404),
            r#"{"error":{"code": -32001,"message":"Session not found"}}"#
        ));
    }

    #[test]
    fn session_expired_rejects_404_without_jsonrpc_code() {
        // Generic 404 — wrong URL, server deployed away.
        assert!(!is_mcp_session_expired_error(
            Some(404),
            "HTTP 404 Not Found"
        ));
    }

    #[test]
    fn session_expired_rejects_non_404_even_with_code() {
        // Some other HTTP code that happens to carry -32001 in body.
        assert!(!is_mcp_session_expired_error(
            Some(500),
            r#"{"code":-32001}"#
        ));
        assert!(!is_mcp_session_expired_error(None, r#"{"code":-32001}"#));
    }
}
