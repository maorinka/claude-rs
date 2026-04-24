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
    McpServerConfig, McpToolDefinition, ScopedMcpServerConfig, MAX_MCP_DESCRIPTION_LENGTH,
    MCP_CONNECTION_TIMEOUT_MS, MCP_TOOL_TIMEOUT_MS,
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
pub const ALLOWED_IDE_TOOLS: &[&str] = &["mcp__ide__executeCode", "mcp__ide__getDiagnostics"];

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
/// variant). Remote transports (SSE, Streamable HTTP, WS,
/// sse-ide, ws-ide) return `false`. Mirrors TS
/// `isLocalMcpServer` at `client.ts:563-565`.
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

/// `true` when the server is an IDE-scoped transport. Matches
/// both `sse-ide` (G9) and `ws-ide` (G18) variants. IDE servers
/// have their tool list filtered through the `mcp__ide__*`
/// allow-list (`is_included_mcp_tool`) and follow a different
/// connection ordering in the CLI's
/// `connect_all_respecting_auth_cache`. Matches TS
/// `client.ts:678-783` which treats `sse-ide` / `ws-ide` as a
/// distinct connection branch.
pub fn is_ide_mcp_server(config: &ScopedMcpServerConfig) -> bool {
    matches!(
        config.config,
        McpServerConfig::SseIde(_) | McpServerConfig::WsIde(_)
    )
}

// ─── HTTP request defaults ───────────────────────────────────────────

/// Attach the MCP Streamable-HTTP defaults to a `reqwest` POST:
/// per-request 60-second timeout and the required
/// `Accept: application/json, text/event-stream` header.
///
/// Mirrors TS `wrapFetchWithTimeout` at `client.ts:492-550`:
///
/// - **Timeout** is per-request rather than per-client so long-lived
///   SSE `GET` streams don't inherit the 60s cap. The equivalent JS
///   pattern creates a fresh `AbortController` for every call to
///   avoid a single stale `AbortSignal.timeout()` poisoning later
///   requests.
/// - **Accept header** is only applied when the user's configured
///   headers don't already carry one. Case-insensitive match —
///   servers that enforce RFC 7230 treat `accept` and `Accept`
///   identically, and users in the wild do both. Mirrors TS
///   `if (!headers.has('accept'))` at `client.ts:508-510`.
///
/// The caller applies `content-type: application/json` and body
/// separately — this helper only handles the two concerns TS
/// centralises in its fetch wrapper.
pub fn mcp_streamable_http_post(
    http: &reqwest::Client,
    url: &str,
    user_headers: Option<&std::collections::HashMap<String, String>>,
) -> reqwest::RequestBuilder {
    let mut req = http
        .post(url)
        .timeout(std::time::Duration::from_millis(MCP_REQUEST_TIMEOUT_MS));
    let user_set_accept = user_headers
        .map(|h| h.keys().any(|k| k.eq_ignore_ascii_case("accept")))
        .unwrap_or(false);
    if !user_set_accept {
        req = req.header("accept", MCP_STREAMABLE_HTTP_ACCEPT);
    }
    req
}

/// Deterministic cache key for a server connection: `"${name}-${json(serverRef)}"`.
/// Mirrors TS `getServerCacheKey` at `client.ts:581-586`. The JSON
/// serialization matches TS's `JSON.stringify(ScopedMcpServerConfig)`
/// output because both sides use flattened config + `type`-tagged
/// variants. Falls back to the debug form if serialization somehow
/// fails — a cache-key collision is preferable to a panic here.
pub fn get_server_cache_key(name: &str, server_ref: &ScopedMcpServerConfig) -> String {
    let body = serde_json::to_string(server_ref).unwrap_or_else(|_| format!("{:?}", server_ref));
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

// ─── Tool enrichment (G7) ────────────────────────────────────────────

/// The suffix TS appends when `tool.description` exceeds
/// `MAX_MCP_DESCRIPTION_LENGTH`. Literal from `client.ts:1792`.
pub const TRUNCATION_SUFFIX: &str = "… [truncated]";

/// Produce the description string the model sees for this tool.
/// If `description` is longer than `MAX_MCP_DESCRIPTION_LENGTH`
/// UTF-16 code units, truncate at the cap and append
/// `"… [truncated]"`. Matches TS `client.ts:1789-1794` which
/// uses JS `String.slice(0, 2048)` — that slices by UTF-16 code
/// units, not bytes.
///
/// For ASCII the three counting conventions (bytes, chars,
/// UTF-16 units) are equivalent. They diverge on BMP non-ASCII:
/// 2048 Cyrillic chars are 4096 UTF-8 bytes but 2048 UTF-16
/// units, so a byte-based cap would keep half as many chars as
/// TS. Counting UTF-16 units preserves visible-length parity.
///
/// Astral-plane characters (len_utf16 == 2) can't be split in
/// Rust (our `String` is valid UTF-8 and can't represent a lone
/// surrogate). If the cap lands mid-pair, we stop before the
/// char so the output stays valid UTF-8; TS can produce a lone
/// surrogate, but the rendered result is indistinguishable in
/// practice.
pub fn tool_description_for_model(tool: &McpToolDefinition) -> String {
    let desc = tool.description.as_deref().unwrap_or("");
    let mut out = String::new();
    let mut units: u32 = 0;
    let cap = MAX_MCP_DESCRIPTION_LENGTH as u32;
    let mut truncated = false;
    for c in desc.chars() {
        let take = c.len_utf16() as u32;
        if units + take > cap {
            truncated = true;
            break;
        }
        out.push(c);
        units += take;
    }
    // Didn't exhaust the input AND didn't consume the whole
    // thing → real truncation; otherwise the buffered string is
    // already the complete original.
    if truncated {
        out.push_str(TRUNCATION_SUFFIX);
    }
    out
}

/// Extract the optional search hint from `tool._meta.anthropic/searchHint`,
/// collapsing any whitespace run to a single space. TS
/// `client.ts:1779-1784` — the whitespace normalisation prevents
/// newlines in a server-controlled field from injecting orphan
/// lines into the formatted tool list.
///
/// Returns `None` when the meta field is absent, non-string, or
/// trims to the empty string.
pub fn extract_search_hint(tool: &McpToolDefinition) -> Option<String> {
    let raw = tool.meta.as_ref()?.get("anthropic/searchHint")?.as_str()?;
    // Whitespace run → single space; trim end; drop if now empty.
    let collapsed: String = {
        let mut out = String::with_capacity(raw.len());
        let mut prev_was_ws = false;
        for ch in raw.chars() {
            if ch.is_whitespace() {
                if !prev_was_ws {
                    out.push(' ');
                    prev_was_ws = true;
                }
            } else {
                out.push(ch);
                prev_was_ws = false;
            }
        }
        out.trim().to_string()
    };
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

/// Is `tool._meta.anthropic/alwaysLoad === true`? Used by the
/// deferred-tool-loader to decide whether to skip the lazy-load
/// gate for this tool. TS `client.ts:1785`.
pub fn extract_always_load(tool: &McpToolDefinition) -> bool {
    tool.meta
        .as_ref()
        .and_then(|m| m.get("anthropic/alwaysLoad"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

// ─── Manager-level pure helpers (G6) ─────────────────────────────────

/// Are two MCP server configs equivalent for connection-reuse
/// purposes? Compares the connection-relevant fields; the
/// `scope` field is explicitly ignored because it's metadata
/// (project/user/global), not something that changes what socket
/// the transport opens. Matches TS `areMcpConfigsEqual` at
/// `client.ts:1710-1722`.
///
/// Uses `PartialEq` directly instead of serialize-and-compare:
/// the config structs contain `HashMap<String,String>` for `env`
/// and `headers`, and `HashMap`'s iteration order is randomised
/// — serializing two equivalent configs could produce different
/// JSON strings and spuriously compare unequal. `HashMap`'s
/// `PartialEq` impl is order-insensitive.
pub fn are_mcp_configs_equal(a: &ScopedMcpServerConfig, b: &ScopedMcpServerConfig) -> bool {
    a.config == b.config
}

/// Encode an MCP tool's input arguments for the auto-mode
/// security classifier. Matches TS
/// `mcpToolInputToAutoClassifierInput` at `client.ts:1733-1740`:
///
/// - Empty input → the tool name alone.
/// - Non-empty → `k1=v1 k2=v2 …` joined with a single space.
///
/// Used by the Rust auto-classifier stubs so their input shape
/// matches what production models see.
///
/// # Divergences from TS
///
/// * **Key order**: TS iterates `Object.keys(input)` (insertion
///   order). Rust's default `serde_json::Map` is backed by
///   `BTreeMap` (alphabetical). Keys come out sorted. This is
///   stable and deterministic but not TS-identical; classifier
///   outputs are order-invariant in practice.
/// * **String values** (primitive): unquoted, matches TS.
/// * **Numbers / booleans / null**: match TS (`1`, `true`,
///   `null`).
/// * **Nested objects**: TS `String({a:1})` → `"[object Object]"`.
///   Rust emits the compact JSON `{"a":1}`. Divergent but more
///   useful for the classifier.
/// * **Arrays**: TS `String([1,2,3])` → `"1,2,3"`. Rust emits
///   `[1,2,3]` (JSON form). Divergent but more useful.
pub fn mcp_tool_input_to_auto_classifier_input(
    input: &serde_json::Map<String, serde_json::Value>,
    tool_name: &str,
) -> String {
    if input.is_empty() {
        return tool_name.to_string();
    }
    input
        .iter()
        .map(|(k, v)| {
            let vs = match v {
                serde_json::Value::String(s) => s.clone(),
                _ => v.to_string(),
            };
            format!("{}={}", k, vs)
        })
        .collect::<Vec<_>>()
        .join(" ")
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
        McpStdioServerConfig, McpWsServerConfig, ScopedMcpServerConfig,
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
        // G9: sse-ide is a remote transport too — it wires the
        // same SSE protocol at a locally-reachable IDE endpoint,
        // but `is_local_mcp_server` is about in-process, not
        // about network locality.
        let sse_ide = scoped(McpServerConfig::SseIde(McpSseServerConfig {
            url: "http://127.0.0.1:3000".into(),
            headers: None,
        }));
        assert!(!is_local_mcp_server(&sse_ide));
    }

    #[test]
    fn ide_server_predicate_matches_sse_ide_only() {
        let stdio = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "echo".into(),
            args: vec![],
            env: None,
        }));
        let sse = scoped(McpServerConfig::Sse(McpSseServerConfig {
            url: "https://remote".into(),
            headers: None,
        }));
        let http = scoped(McpServerConfig::Http(McpHttpServerConfig {
            url: "https://remote".into(),
            headers: None,
        }));
        let sse_ide = scoped(McpServerConfig::SseIde(McpSseServerConfig {
            url: "http://127.0.0.1:3000".into(),
            headers: None,
        }));
        assert!(!is_ide_mcp_server(&stdio));
        assert!(!is_ide_mcp_server(&sse));
        assert!(!is_ide_mcp_server(&http));
        assert!(is_ide_mcp_server(&sse_ide));
    }

    #[test]
    fn ide_server_predicate_matches_ws_ide() {
        // G18 scaffolding: ws-ide must also classify as IDE so
        // the tool allow-list + connection ordering apply.
        let ws_ide = scoped(McpServerConfig::WsIde(McpWsServerConfig {
            url: "ws://127.0.0.1:9000".into(),
            headers: None,
            auth_token: Some("tok".into()),
        }));
        assert!(is_ide_mcp_server(&ws_ide));
        // Plain ws is NOT IDE — distinct variant.
        let ws = scoped(McpServerConfig::Ws(McpWsServerConfig {
            url: "wss://example.invalid".into(),
            headers: None,
            auth_token: None,
        }));
        assert!(!is_ide_mcp_server(&ws));
    }

    #[test]
    fn local_server_ws_variants_are_not_local() {
        let ws = scoped(McpServerConfig::Ws(McpWsServerConfig {
            url: "ws://example.invalid".into(),
            headers: None,
            auth_token: None,
        }));
        assert!(!is_local_mcp_server(&ws));
        let ws_ide = scoped(McpServerConfig::WsIde(McpWsServerConfig {
            url: "ws://127.0.0.1".into(),
            headers: None,
            auth_token: Some("t".into()),
        }));
        assert!(!is_local_mcp_server(&ws_ide));
    }

    #[test]
    fn ws_config_round_trips_with_distinct_type_tag() {
        let cfg = McpServerConfig::Ws(McpWsServerConfig {
            url: "wss://example.invalid".into(),
            headers: None,
            auth_token: None,
        });
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(
            json.contains("\"type\":\"ws\""),
            "expected 'ws' tag in {}",
            json
        );
        let back: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, McpServerConfig::Ws(_)));
    }

    #[test]
    fn ws_ide_config_round_trips_with_auth_token_camel_case() {
        // authToken must serialise as camelCase on the wire
        // (matches TS's `serverRef.authToken` field name).
        let cfg = McpServerConfig::WsIde(McpWsServerConfig {
            url: "ws://127.0.0.1:9000".into(),
            headers: None,
            auth_token: Some("secret-token".into()),
        });
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"type\":\"ws-ide\""));
        assert!(
            json.contains("\"authToken\":\"secret-token\""),
            "expected camelCase 'authToken' field in {}",
            json
        );
        let back: McpServerConfig = serde_json::from_str(&json).unwrap();
        match back {
            McpServerConfig::WsIde(ws) => {
                assert_eq!(ws.auth_token.as_deref(), Some("secret-token"));
            }
            other => panic!("expected WsIde, got {:?}", other),
        }
    }

    #[test]
    fn sse_ide_config_round_trips_with_distinct_type_tag() {
        // `sse-ide` must serialize with its own `"type"` tag so
        // the distinction survives reads from disk.
        let cfg = McpServerConfig::SseIde(McpSseServerConfig {
            url: "http://127.0.0.1:9000".into(),
            headers: None,
        });
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(
            json.contains("\"type\":\"sse-ide\""),
            "expected 'sse-ide' tag in {}",
            json
        );
        let back: McpServerConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, McpServerConfig::SseIde(_)));
    }

    // ─── tool predicate ──────────────────────────────────────────

    fn tool_named(name: &str) -> McpToolDefinition {
        McpToolDefinition {
            name: name.to_string(),
            description: None,
            input_schema: Some(serde_json::json!({})),
            annotations: None,
            meta: None,
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
        assert!(is_included_mcp_tool(&tool_named(
            "mcp__ide__getDiagnostics"
        )));
    }

    #[test]
    fn included_tool_ide_other_tools_are_dropped() {
        assert!(!is_included_mcp_tool(&tool_named(
            "mcp__ide__listOpenFiles"
        )));
        assert!(!is_included_mcp_tool(&tool_named("mcp__ide__closeTab")));
        assert!(!is_included_mcp_tool(&tool_named("mcp__ide__")));
    }

    // ─── tool enrichment (G7) ────────────────────────────────────

    #[test]
    fn description_passthrough_when_under_cap() {
        let mut t = tool_named("x");
        t.description = Some("short description".to_string());
        assert_eq!(tool_description_for_model(&t), "short description");
    }

    #[test]
    fn description_truncates_with_marker_when_over_cap() {
        let mut t = tool_named("x");
        // cap = 2048 (MAX_MCP_DESCRIPTION_LENGTH). Build a 3000-byte
        // ASCII description so truncation clearly fires.
        t.description = Some("a".repeat(3000));
        let s = tool_description_for_model(&t);
        assert!(
            s.ends_with(TRUNCATION_SUFFIX),
            "expected truncation suffix, got tail: {:?}",
            &s[s.len().saturating_sub(30)..]
        );
        // Body length = cap; total = cap + suffix bytes.
        assert!(s.len() > MAX_MCP_DESCRIPTION_LENGTH);
    }

    #[test]
    fn description_empty_when_missing() {
        let t = tool_named("x"); // no description
        assert_eq!(tool_description_for_model(&t), "");
    }

    #[test]
    fn description_truncation_counts_utf16_code_units() {
        // Codex CR parity gap: TS JS `.slice(0, 2048)` counts
        // UTF-16 code units. Cyrillic 'я' is 1 code unit (BMP).
        // 2049 'я' chars → truncate to the first 2048 + suffix,
        // NOT to byte-floor(2048/2)=1024 (the byte-cap bug).
        let mut t = tool_named("x");
        t.description = Some("я".repeat(2049));
        let s = tool_description_for_model(&t);
        assert!(s.ends_with(TRUNCATION_SUFFIX));
        // Strip suffix; remaining chars must be exactly 2048.
        let body = s.strip_suffix(TRUNCATION_SUFFIX).expect("suffix present");
        let chars = body.chars().count();
        assert_eq!(
            chars, MAX_MCP_DESCRIPTION_LENGTH,
            "BMP chars should fill exactly 2048 units, got {} chars",
            chars
        );
    }

    #[test]
    fn description_truncation_astral_char_does_not_split() {
        // Non-BMP char ('🎉' U+1F389) is 2 UTF-16 code units.
        // With 1024 emoji = 2048 units exactly, we keep them
        // all. 1025 = 2050 units → stops before the 1025th to
        // avoid a cap-straddling surrogate pair.
        let mut t = tool_named("x");
        t.description = Some("🎉".repeat(1025));
        let s = tool_description_for_model(&t);
        assert!(s.ends_with(TRUNCATION_SUFFIX));
        let body = s.strip_suffix(TRUNCATION_SUFFIX).unwrap();
        let chars = body.chars().count();
        assert_eq!(
            chars, 1024,
            "astral pairs must not be split, got {} chars",
            chars
        );
    }

    #[test]
    fn description_exactly_at_cap_is_not_truncated() {
        // Boundary parity: exactly 2048 UTF-16 units → no suffix.
        let mut t = tool_named("x");
        t.description = Some("a".repeat(MAX_MCP_DESCRIPTION_LENGTH));
        let s = tool_description_for_model(&t);
        assert!(!s.ends_with(TRUNCATION_SUFFIX));
        assert_eq!(s.len(), MAX_MCP_DESCRIPTION_LENGTH);
    }

    #[test]
    fn search_hint_extracts_and_collapses_whitespace() {
        let mut t = tool_named("x");
        t.meta = Some(serde_json::json!({
            "anthropic/searchHint": "one\t\ttwo\n\nthree  four",
        }));
        assert_eq!(
            extract_search_hint(&t).as_deref(),
            Some("one two three four")
        );
    }

    #[test]
    fn search_hint_none_when_missing_or_wrong_shape() {
        let t_no_meta = tool_named("x");
        assert!(extract_search_hint(&t_no_meta).is_none());

        let mut t_wrong_key = tool_named("x");
        t_wrong_key.meta = Some(serde_json::json!({ "other": "v" }));
        assert!(extract_search_hint(&t_wrong_key).is_none());

        let mut t_non_string = tool_named("x");
        t_non_string.meta = Some(serde_json::json!({
            "anthropic/searchHint": 42,
        }));
        assert!(extract_search_hint(&t_non_string).is_none());
    }

    #[test]
    fn search_hint_whitespace_only_returns_none() {
        let mut t = tool_named("x");
        t.meta = Some(serde_json::json!({
            "anthropic/searchHint": "   \t\n  ",
        }));
        assert!(extract_search_hint(&t).is_none());
    }

    #[test]
    fn always_load_strict_true_only() {
        let mut t = tool_named("x");
        assert!(!extract_always_load(&t)); // no meta
        t.meta = Some(serde_json::json!({ "anthropic/alwaysLoad": false }));
        assert!(!extract_always_load(&t));
        t.meta = Some(serde_json::json!({ "anthropic/alwaysLoad": "true" }));
        assert!(!extract_always_load(&t)); // strict: string-true doesn't qualify
        t.meta = Some(serde_json::json!({ "anthropic/alwaysLoad": 1 }));
        assert!(!extract_always_load(&t)); // strict: truthy doesn't qualify
        t.meta = Some(serde_json::json!({ "anthropic/alwaysLoad": true }));
        assert!(extract_always_load(&t));
    }

    #[test]
    fn annotations_round_trip_from_server_payload() {
        // Verify the new McpToolDefinition fields deserialize from
        // a representative wire payload. Guards against accidental
        // rename breakage on `annotations` / `_meta`.
        let wire = serde_json::json!({
            "name": "search",
            "description": "Search",
            "inputSchema": { "type": "object" },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "openWorldHint": true,
                "title": "Search"
            },
            "_meta": { "anthropic/alwaysLoad": true }
        });
        let parsed: McpToolDefinition = serde_json::from_value(wire).expect("deserialize");
        let a = parsed.annotations.as_ref().expect("annotations present");
        assert_eq!(a.read_only_hint, Some(true));
        assert_eq!(a.destructive_hint, Some(false));
        assert_eq!(a.open_world_hint, Some(true));
        assert_eq!(a.title.as_deref(), Some("Search"));
        assert!(extract_always_load(&parsed));
    }

    // ─── manager-level pure helpers (G6) ─────────────────────────

    #[test]
    fn configs_equal_same_stdio() {
        let a = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "echo".into(),
            args: vec!["hi".into()],
            env: None,
        }));
        let b = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "echo".into(),
            args: vec!["hi".into()],
            env: None,
        }));
        assert!(are_mcp_configs_equal(&a, &b));
    }

    #[test]
    fn configs_equal_different_types_rejected_fast() {
        let stdio = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "echo".into(),
            args: vec![],
            env: None,
        }));
        let sse = scoped(McpServerConfig::Sse(McpSseServerConfig {
            url: "https://x".into(),
            headers: None,
        }));
        // Discriminant check rejects before the serialize path.
        assert!(!are_mcp_configs_equal(&stdio, &sse));
    }

    #[test]
    fn configs_equal_ignores_scope() {
        // TS excludes `scope` — projects pinned at different
        // scopes but with identical connection config should be
        // considered equivalent.
        let cfg = McpServerConfig::Stdio(McpStdioServerConfig {
            command: "echo".into(),
            args: vec![],
            env: None,
        });
        let a = ScopedMcpServerConfig {
            config: cfg.clone(),
            scope: ConfigScope::Project,
        };
        let b = ScopedMcpServerConfig {
            config: cfg,
            scope: ConfigScope::User,
        };
        assert!(
            are_mcp_configs_equal(&a, &b),
            "scope must not affect equivalence"
        );
    }

    #[test]
    fn configs_equal_env_order_invariant() {
        // HashMap iteration order is randomised. Two configs with
        // the SAME env vars but inserted in different orders must
        // still compare equal. Guards against the
        // serialize-and-compare regression codex flagged.
        use std::collections::HashMap;
        let mut env_a = HashMap::new();
        env_a.insert("FOO".to_string(), "1".to_string());
        env_a.insert("BAR".to_string(), "2".to_string());
        let mut env_b = HashMap::new();
        env_b.insert("BAR".to_string(), "2".to_string());
        env_b.insert("FOO".to_string(), "1".to_string());
        let a = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "x".into(),
            args: vec![],
            env: Some(env_a),
        }));
        let b = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "x".into(),
            args: vec![],
            env: Some(env_b),
        }));
        assert!(
            are_mcp_configs_equal(&a, &b),
            "env order must not affect equivalence"
        );
    }

    #[test]
    fn configs_equal_headers_order_invariant() {
        // Same guard for SSE/HTTP headers.
        use std::collections::HashMap;
        let mut h_a = HashMap::new();
        h_a.insert("X-A".to_string(), "1".to_string());
        h_a.insert("X-B".to_string(), "2".to_string());
        let mut h_b = HashMap::new();
        h_b.insert("X-B".to_string(), "2".to_string());
        h_b.insert("X-A".to_string(), "1".to_string());
        let a = scoped(McpServerConfig::Sse(McpSseServerConfig {
            url: "https://x".into(),
            headers: Some(h_a),
        }));
        let b = scoped(McpServerConfig::Sse(McpSseServerConfig {
            url: "https://x".into(),
            headers: Some(h_b),
        }));
        assert!(are_mcp_configs_equal(&a, &b));
    }

    #[test]
    fn configs_equal_different_env_values_rejected() {
        use std::collections::HashMap;
        let mut env_a = HashMap::new();
        env_a.insert("FOO".to_string(), "1".to_string());
        let mut env_b = HashMap::new();
        env_b.insert("FOO".to_string(), "2".to_string());
        let a = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "x".into(),
            args: vec![],
            env: Some(env_a),
        }));
        let b = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "x".into(),
            args: vec![],
            env: Some(env_b),
        }));
        assert!(!are_mcp_configs_equal(&a, &b));
    }

    #[test]
    fn configs_equal_different_commands_rejected() {
        let a = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "echo".into(),
            args: vec![],
            env: None,
        }));
        let b = scoped(McpServerConfig::Stdio(McpStdioServerConfig {
            command: "cat".into(),
            args: vec![],
            env: None,
        }));
        assert!(!are_mcp_configs_equal(&a, &b));
    }

    #[test]
    fn auto_classifier_empty_input_returns_tool_name() {
        let input = serde_json::Map::new();
        assert_eq!(
            mcp_tool_input_to_auto_classifier_input(&input, "mcp__svc__do_thing"),
            "mcp__svc__do_thing"
        );
    }

    #[test]
    fn auto_classifier_single_string_arg() {
        let mut input = serde_json::Map::new();
        input.insert("query".into(), serde_json::json!("rust borrow checker"));
        assert_eq!(
            mcp_tool_input_to_auto_classifier_input(&input, "search"),
            "query=rust borrow checker"
        );
    }

    #[test]
    fn auto_classifier_multiple_args_joined_with_space() {
        let mut input = serde_json::Map::new();
        // Inserted alphabetically so the assertion holds whether
        // the backing map is a BTreeMap (default) or IndexMap
        // (feature `preserve_order`). Order-specific behaviour
        // is pinned separately by
        // `auto_classifier_keys_emerge_alphabetically`.
        input.insert("a".into(), serde_json::json!(1));
        input.insert("b".into(), serde_json::json!(true));
        input.insert("c".into(), serde_json::json!("hi"));
        let out = mcp_tool_input_to_auto_classifier_input(&input, "tool");
        assert_eq!(out, "a=1 b=true c=hi");
    }

    #[test]
    fn auto_classifier_keys_emerge_alphabetically() {
        // Codex CR: serde_json::Map is backed by BTreeMap without
        // the `preserve_order` feature, so iteration is
        // alphabetical — NOT insertion-order (which is what the
        // original commit claimed). The output is still stable
        // and deterministic, just sorted rather than as-inserted.
        let mut input = serde_json::Map::new();
        input.insert("zebra".into(), serde_json::json!(1));
        input.insert("apple".into(), serde_json::json!(2));
        input.insert("mango".into(), serde_json::json!(3));
        let out = mcp_tool_input_to_auto_classifier_input(&input, "tool");
        assert_eq!(out, "apple=2 mango=3 zebra=1");
    }

    #[test]
    fn auto_classifier_array_value_differs_from_ts() {
        // TS `String([1,2,3])` emits "1,2,3"; Rust emits the JSON
        // form "[1,2,3]". Divergent but more informative for the
        // classifier prompt. Pin the Rust shape so it doesn't
        // drift silently.
        let mut input = serde_json::Map::new();
        input.insert("items".into(), serde_json::json!([1, 2, 3]));
        let out = mcp_tool_input_to_auto_classifier_input(&input, "tool");
        assert_eq!(out, "items=[1,2,3]");
    }

    #[test]
    fn auto_classifier_nested_object_value_differs_from_ts() {
        // TS `String({a:1})` emits "[object Object]"; Rust emits
        // the JSON form. Divergent but informative.
        let mut input = serde_json::Map::new();
        input.insert("opts".into(), serde_json::json!({ "a": 1 }));
        let out = mcp_tool_input_to_auto_classifier_input(&input, "tool");
        assert_eq!(out, r#"opts={"a":1}"#);
    }

    #[test]
    fn auto_classifier_strings_are_unquoted() {
        // TS `String(x)` on a string drops the quotes; Rust's
        // serde_json Display would add them. Our helper special-
        // cases strings to match TS.
        let mut input = serde_json::Map::new();
        input.insert("msg".into(), serde_json::json!("hello world"));
        let out = mcp_tool_input_to_auto_classifier_input(&input, "tool");
        assert_eq!(out, "msg=hello world");
        assert!(!out.contains('"'), "strings must be unquoted: {}", out);
    }

    // ─── streamable-http post helper ─────────────────────────────

    #[tokio::test]
    async fn streamable_post_adds_accept_when_user_has_none() {
        let http = reqwest::Client::new();
        // Build the request — `try_clone` to harvest an inspectable
        // `Request` without actually sending.
        let rb = mcp_streamable_http_post(&http, "http://example.invalid/x", None);
        let req = rb.build().expect("build should succeed");
        let accept = req
            .headers()
            .get("accept")
            .expect("accept header must be present");
        assert_eq!(accept, MCP_STREAMABLE_HTTP_ACCEPT);
        // Per-request timeout must be exactly MCP_REQUEST_TIMEOUT_MS.
        assert_eq!(
            req.timeout(),
            Some(&std::time::Duration::from_millis(MCP_REQUEST_TIMEOUT_MS))
        );
    }

    #[tokio::test]
    async fn streamable_post_respects_user_accept_header_any_case() {
        let http = reqwest::Client::new();
        let mut user = std::collections::HashMap::new();
        user.insert("Accept".to_string(), "application/json".to_string());
        let rb = mcp_streamable_http_post(&http, "http://example.invalid/x", Some(&user));
        let req = rb.build().expect("build should succeed");
        // The helper must NOT have set a default; the user's header
        // is supplied by the caller (who iterates the user map after
        // the helper call), so the built request here has no accept.
        // The *contract* is: helper doesn't stomp user-set accept.
        assert!(
            req.headers().get("accept").is_none(),
            "helper must not force a default when user set accept (case-insensitive)"
        );
    }

    #[tokio::test]
    async fn streamable_post_user_header_mixed_case_detected() {
        // "aCcEpT" must also suppress the default — TS's Headers
        // lookup is case-insensitive.
        let http = reqwest::Client::new();
        let mut user = std::collections::HashMap::new();
        user.insert("aCcEpT".to_string(), "text/plain".to_string());
        let rb = mcp_streamable_http_post(&http, "http://example.invalid/x", Some(&user));
        let req = rb.build().expect("build should succeed");
        assert!(req.headers().get("accept").is_none());
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
