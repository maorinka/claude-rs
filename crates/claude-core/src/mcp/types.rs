use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Transport type for connecting to an MCP server.
/// Matches the TS `Transport` enum: 'stdio' | 'sse' | 'sse-ide' | 'http' | 'ws' | 'sdk'
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum TransportType {
    #[default]
    Stdio,
    Sse,
    SseIde,
    Http,
    Ws,
    Sdk,
}

/// Configuration scope for where the MCP server config came from.
/// Matches TS `ConfigScope`: 'local' | 'user' | 'project' | 'dynamic' | 'enterprise' | 'claudeai' | 'managed'
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigScope {
    Local,
    User,
    Project,
    Dynamic,
    Enterprise,
    #[serde(rename = "claudeai")]
    ClaudeAi,
    Managed,
}

/// Stdio server configuration.
/// Matches TS `McpStdioServerConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpStdioServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
}

/// SSE server configuration.
/// Matches TS `McpSSEServerConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSseServerConfig {
    pub url: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub oauth: Option<McpOAuthConfig>,
}

/// HTTP server configuration.
/// Matches TS `McpHTTPServerConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpHttpServerConfig {
    pub url: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub oauth: Option<McpOAuthConfig>,
}

/// Remote MCP OAuth settings. Matches TS `McpOAuthConfigSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthConfig {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub callback_port: Option<u32>,
    #[serde(default)]
    pub auth_server_metadata_url: Option<String>,
    #[serde(default)]
    pub xaa: Option<bool>,
}

/// WebSocket server configuration, shared shape for `ws` and
/// `ws-ide`. Matches TS's ws/ws-ide union (client.ts:708-783).
///
/// `auth_token` is the IDE-specific field TS sends as
/// `X-Claude-Code-Ide-Authorization` for ws-ide connections
/// (client.ts:712-714). For regular `ws` the token lives in the
/// `Authorization` header via `session_ingress_token` wiring —
/// not stored on the config.
///
/// # TS shape divergence note
/// TS models `ws` with an optional `headers` map (user-configured
/// via `combinedHeaders`) and `ws-ide` with ONLY `url + authToken`
/// (the wire headers are transport-derived: `User-Agent` +
/// `X-Claude-Code-Ide-Authorization`). The Rust shape is shared,
/// so `ws-ide` configs round-trip a `headers` field TS doesn't
/// recognise. **G18b TODO**: when the real WebSocket transport
/// lands, either split into two structs or ignore `headers` on
/// `ws-ide` at connect time to match TS. Keeping the shared
/// shape for now so config files with a common structure still
/// parse cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpWsServerConfig {
    pub url: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// IDE-authorization token sent as
    /// `X-Claude-Code-Ide-Authorization` on `ws-ide`
    /// connections only.
    #[serde(default)]
    pub auth_token: Option<String>,
}

/// Union of all MCP server config variants.
/// Matches TS `McpServerConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpServerConfig {
    #[serde(rename = "stdio")]
    Stdio(McpStdioServerConfig),
    #[serde(rename = "sse")]
    Sse(McpSseServerConfig),
    #[serde(rename = "http")]
    Http(McpHttpServerConfig),
    /// IDE-scoped SSE transport. Wire-identical to `sse` but
    /// marked distinct so downstream code can apply the
    /// `mcp__ide__*` tool allow-list (G1 `is_included_mcp_tool`)
    /// and the connection ordering TS uses for IDE servers
    /// (`client.ts:678-707`).
    #[serde(rename = "sse-ide")]
    SseIde(McpSseServerConfig),
    /// WebSocket transport (standard `ws://` or `wss://`).
    /// Matches TS `client.ts:735-783`. Transport-level
    /// implementation is G18b scope — config scaffolding lands
    /// here so configs round-trip off disk and downstream
    /// orchestration can classify the server correctly.
    #[serde(rename = "ws")]
    Ws(McpWsServerConfig),
    /// IDE-scoped WebSocket — the same protocol as `ws` plus
    /// the IDE-authorization header from `auth_token`. Same
    /// tool allow-list + ordering semantics as `sse-ide`.
    /// Matches TS `client.ts:708-734`.
    #[serde(rename = "ws-ide")]
    WsIde(McpWsServerConfig),
}

/// For backward compatibility: when `type` is omitted, default to stdio.
impl McpServerConfig {
    /// Parse from a serde_json::Value, defaulting to stdio when "type" is absent.
    pub fn from_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        // If "type" is missing, assume stdio
        if let Some(obj) = value.as_object() {
            if !obj.contains_key("type") {
                let mut obj = obj.clone();
                obj.insert(
                    "type".to_string(),
                    serde_json::Value::String("stdio".to_string()),
                );
                return serde_json::from_value(serde_json::Value::Object(obj));
            }
        }
        serde_json::from_value(value)
    }

    /// Returns the transport type for this config.
    pub fn transport_type(&self) -> TransportType {
        match self {
            McpServerConfig::Stdio(_) => TransportType::Stdio,
            McpServerConfig::Sse(_) => TransportType::Sse,
            McpServerConfig::Http(_) => TransportType::Http,
            McpServerConfig::SseIde(_) => TransportType::SseIde,
            McpServerConfig::Ws(_) | McpServerConfig::WsIde(_) => TransportType::Ws,
        }
    }
}

/// Server config with associated scope metadata.
/// Matches TS `ScopedMcpServerConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopedMcpServerConfig {
    #[serde(flatten)]
    pub config: McpServerConfig,
    pub scope: ConfigScope,
}

/// Server capabilities advertised during initialization.
/// Matches TS `ServerCapabilities`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    #[serde(default)]
    pub tools: Option<ToolsCapability>,
    #[serde(default)]
    pub resources: Option<ResourcesCapability>,
    #[serde(default)]
    pub prompts: Option<PromptsCapability>,
    #[serde(default)]
    pub experimental: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsCapability {
    #[serde(default)]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourcesCapability {
    #[serde(default)]
    pub list_changed: Option<bool>,
    #[serde(default)]
    pub subscribe: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptsCapability {
    #[serde(default)]
    pub list_changed: Option<bool>,
}

/// Information about the connected server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// The connection status of an MCP server.
/// Matches the TS discriminated union `MCPServerConnection`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpConnectionStatus {
    #[serde(rename = "connected")]
    Connected {
        capabilities: ServerCapabilities,
        #[serde(default)]
        server_info: Option<ServerInfo>,
        #[serde(default)]
        instructions: Option<String>,
    },
    #[serde(rename = "failed")]
    Failed {
        #[serde(default)]
        error: Option<String>,
    },
    #[serde(rename = "pending")]
    Pending {
        #[serde(default)]
        reconnect_attempt: Option<u32>,
        #[serde(default)]
        max_reconnect_attempts: Option<u32>,
    },
    #[serde(rename = "disabled")]
    Disabled,
    /// Server requires re-authorization (e.g. OAuth token
    /// expired or revoked). `tengu_mcp_server_needs_auth`
    /// telemetry fires when this status is reached; the
    /// `mcp_auth_cache` suppresses reconnect attempts for 15
    /// minutes so the UI doesn't repeat the same prompt.
    /// Matches TS `'needs-auth'` status at
    /// `services/mcp/client.ts:340-361`.
    #[serde(rename = "needs-auth")]
    NeedsAuth,
}

/// Full server connection state with config.
/// Matches the full TS `MCPServerConnection` with name and config attached.
#[derive(Debug, Clone)]
pub struct McpServerConnection {
    pub name: String,
    pub status: McpConnectionStatus,
    pub config: ScopedMcpServerConfig,
}

impl McpServerConnection {
    pub fn is_connected(&self) -> bool {
        matches!(self.status, McpConnectionStatus::Connected { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self.status, McpConnectionStatus::Failed { .. })
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.status, McpConnectionStatus::Pending { .. })
    }
}

/// An MCP resource.
/// Matches TS `Resource` from the MCP SDK plus the `server` field from `ServerResource`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerResource {
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "mimeType")]
    #[serde(default)]
    pub mime_type: Option<String>,
    pub server: String,
}

/// MCP tool annotations — hints the server sends to describe the
/// tool's side-effect profile. Used by the Rust tool layer to
/// decide concurrency safety, permission prompts, and
/// destructive-action confirmations. Ports the TS
/// `Tool.annotations` shape (MCP SDK `ToolAnnotations`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolAnnotations {
    /// Human-friendly title override for UIs.
    #[serde(default)]
    pub title: Option<String>,
    /// `true` → tool does not mutate server state; safe to run
    /// concurrently with other read-only tools.
    #[serde(default)]
    pub read_only_hint: Option<bool>,
    /// `true` → tool may cause non-recoverable changes; the
    /// UI should prompt before invoking.
    #[serde(default)]
    pub destructive_hint: Option<bool>,
    /// `true` → tool reaches into arbitrary external systems
    /// (Internet, remote APIs). Used by the permission layer.
    #[serde(default)]
    pub open_world_hint: Option<bool>,
    /// `true` → calling with the same args produces the same
    /// result (pure-ish).
    #[serde(default)]
    pub idempotent_hint: Option<bool>,
}

/// An MCP tool definition as received from the server.
/// Matches TS `SerializedTool` / the MCP SDK `Tool` type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, alias = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
    /// Server-provided side-effect hints. Populated by G7 so the
    /// Rust tool layer can propagate `readOnly` / `destructive`
    /// etc. into its permission checks.
    #[serde(default)]
    pub annotations: Option<McpToolAnnotations>,
    /// Arbitrary server-side metadata. MCP spec allows anything
    /// under here; Claude Code uses `anthropic/searchHint` and
    /// `anthropic/alwaysLoad` keys (see `mcp::helpers`).
    #[serde(default, rename = "_meta")]
    pub meta: Option<serde_json::Value>,
}

/// Result content from an MCP tool call.
/// The MCP protocol uses camelCase for field names (e.g. mimeType).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolResultContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// Result from calling an MCP tool.
/// Matches the MCP `CallToolResult`. Uses camelCase per the MCP protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolResult {
    pub content: Vec<McpToolResultContent>,
    #[serde(default)]
    pub is_error: Option<bool>,
}

/// JSON-RPC 2.0 request message for the MCP protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        }
    }
}

/// JSON-RPC 2.0 notification (no id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcNotification {
    pub fn new(method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        }
    }
}

/// JSON-RPC 2.0 response message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// MCP protocol version and client info used during initialization.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
pub const MCP_CLIENT_NAME: &str = "claude-rs";
pub const MCP_CLIENT_VERSION: &str = "0.1.0";

/// Default timeout for MCP connection handshake (30 seconds).
pub const MCP_CONNECTION_TIMEOUT_MS: u64 = 30_000;

/// Default timeout for MCP tool calls (~27.8 hours, matching TS).
pub const MCP_TOOL_TIMEOUT_MS: u64 = 100_000_000;

/// Maximum length for MCP tool descriptions sent to the model (2048 chars, matching TS).
pub const MAX_MCP_DESCRIPTION_LENGTH: usize = 2048;

/// JSON-RPC method names used by the MCP protocol.
pub mod methods {
    pub const INITIALIZE: &str = "initialize";
    pub const INITIALIZED: &str = "notifications/initialized";
    pub const TOOLS_LIST: &str = "tools/list";
    pub const TOOLS_CALL: &str = "tools/call";
    pub const RESOURCES_LIST: &str = "resources/list";
    pub const RESOURCES_READ: &str = "resources/read";
    pub const PROMPTS_LIST: &str = "prompts/list";
    pub const PROMPTS_GET: &str = "prompts/get";
    pub const PING: &str = "ping";
}

// ─── Prompts (G8) ────────────────────────────────────────────────────

/// One named argument of an MCP prompt template. The `required`
/// flag follows the MCP SDK default of `false` when absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// UI-display override for the argument label.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub required: Option<bool>,
}

/// An MCP prompt definition as returned by `prompts/list`. Named
/// templates the server exposes as slash-commandable bodies the
/// client can render with the provided arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// UI-display override; `name` stays the canonical identifier
    /// so slash-command parsing doesn't trip on embedded spaces.
    /// TS `client.ts:2066-2070`.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub arguments: Option<Vec<McpPromptArgument>>,
    /// Optional icon set surfaced by newer MCP servers (SDK
    /// schema `Prompt.icons`). Kept as raw JSON since consumers
    /// decide the rendering.
    #[serde(default)]
    pub icons: Option<serde_json::Value>,
    /// Arbitrary server-side metadata — the MCP schema's `_meta`
    /// on `Prompt`. Mirrors the same field on McpToolDefinition.
    #[serde(default, rename = "_meta")]
    pub meta: Option<serde_json::Value>,
}

/// One message in the response from `prompts/get`. Content is the
/// raw MCP content block (text/image/resource/etc.); G12b's
/// `transform_result_content` pipeline turns it into the
/// provider-facing shape. Kept as `Value` at the transport
/// boundary — downstream code normalises to `ContentBlock`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptMessage {
    pub role: String,
    pub content: serde_json::Value,
}

/// Full reply to `prompts/get`. Mirrors the MCP `GetPromptResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptResult {
    #[serde(default)]
    pub description: Option<String>,
    pub messages: Vec<McpPromptMessage>,
    /// Arbitrary server-side metadata — MCP schema's `_meta` on
    /// `GetPromptResult`. Preserved so SDK consumers can still
    /// see it.
    #[serde(default, rename = "_meta")]
    pub meta: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remote_mcp_oauth_xaa_config_round_trips_like_ts() {
        let value = json!({
            "type": "http",
            "url": "https://mcp.example.com/mcp",
            "oauth": {
                "clientId": "as-client",
                "callbackPort": 49152,
                "authServerMetadataUrl": "https://as.example.com/.well-known/oauth-authorization-server",
                "xaa": true
            }
        });
        let parsed = McpServerConfig::from_value(value).unwrap();
        let McpServerConfig::Http(http) = parsed else {
            panic!("expected http config");
        };
        let oauth = http.oauth.unwrap();
        assert_eq!(oauth.client_id.as_deref(), Some("as-client"));
        assert_eq!(oauth.callback_port, Some(49152));
        assert_eq!(
            oauth.auth_server_metadata_url.as_deref(),
            Some("https://as.example.com/.well-known/oauth-authorization-server")
        );
        assert_eq!(oauth.xaa, Some(true));

        let serialized = serde_json::to_value(McpServerConfig::Http(McpHttpServerConfig {
            url: http.url,
            headers: None,
            oauth: Some(oauth),
        }))
        .unwrap();
        assert_eq!(serialized["oauth"]["clientId"], "as-client");
        assert_eq!(serialized["oauth"]["xaa"], true);
    }
}
