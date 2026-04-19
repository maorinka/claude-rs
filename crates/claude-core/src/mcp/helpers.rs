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

// ─── Tool enrichment (G7) ────────────────────────────────────────────

/// The suffix TS appends when `tool.description` exceeds
/// `MAX_MCP_DESCRIPTION_LENGTH`. Literal from `client.ts:1792`.
pub const TRUNCATION_SUFFIX: &str = "… [truncated]";

/// Produce the description string the model sees for this tool.
/// If `description` is longer than `MAX_MCP_DESCRIPTION_LENGTH`,
/// truncate at the cap and append `"… [truncated]"`. Matches TS
/// `client.ts:1789-1794`.
///
/// Byte-level slicing would be unsafe on UTF-8 (could cut a
/// multi-byte char); we slice on the nearest char boundary at or
/// below the cap.
pub fn tool_description_for_model(tool: &McpToolDefinition) -> String {
    let desc = tool.description.as_deref().unwrap_or("");
    if desc.len() <= MAX_MCP_DESCRIPTION_LENGTH {
        return desc.to_string();
    }
    // Walk char boundaries backwards from the cap until we land on
    // one. `is_char_boundary(0)` and `is_char_boundary(len)` are
    // always true, so this terminates.
    let mut end = MAX_MCP_DESCRIPTION_LENGTH;
    while end > 0 && !desc.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &desc[..end], TRUNCATION_SUFFIX)
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
    let raw = tool
        .meta
        .as_ref()?
        .get("anthropic/searchHint")?
        .as_str()?;
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
        assert!(is_included_mcp_tool(&tool_named("mcp__ide__getDiagnostics")));
    }

    #[test]
    fn included_tool_ide_other_tools_are_dropped() {
        assert!(!is_included_mcp_tool(&tool_named("mcp__ide__listOpenFiles")));
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
    fn description_truncation_respects_utf8_boundaries() {
        // The cap may land mid-multibyte-char; the helper must
        // back up to the nearest char boundary so the returned
        // `String` is still valid UTF-8.
        let mut t = tool_named("x");
        // 2-byte-per-char Cyrillic → ~1500 chars fills ~3000
        // bytes, well past the cap.
        t.description = Some("я".repeat(1500));
        let s = tool_description_for_model(&t);
        // Must not panic and must contain the marker.
        assert!(s.ends_with(TRUNCATION_SUFFIX));
        // Body must be valid UTF-8 (trivially true for &str /
        // String; the assertion is "no panic on non-boundary
        // slice").
        assert!(!s.is_empty());
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
        let parsed: McpToolDefinition =
            serde_json::from_value(wire).expect("deserialize");
        let a = parsed.annotations.as_ref().expect("annotations present");
        assert_eq!(a.read_only_hint, Some(true));
        assert_eq!(a.destructive_hint, Some(false));
        assert_eq!(a.open_world_hint, Some(true));
        assert_eq!(a.title.as_deref(), Some("Search"));
        assert!(extract_always_load(&parsed));
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
