use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Transport type for connecting to an MCP server.
/// Matches the TS `Transport` enum: 'stdio' | 'sse' | 'sse-ide' | 'http' | 'ws' | 'sdk'
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportType {
    Stdio,
    Sse,
    SseIde,
    Http,
    Ws,
    Sdk,
}

impl Default for TransportType {
    fn default() -> Self {
        Self::Stdio
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpStdioServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
}

/// SSE server configuration.
/// Matches TS `McpSSEServerConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSseServerConfig {
    pub url: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
}

/// HTTP server configuration.
/// Matches TS `McpHTTPServerConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpHttpServerConfig {
    pub url: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
}

/// Union of all MCP server config variants.
/// Matches TS `McpServerConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpServerConfig {
    #[serde(rename = "stdio")]
    Stdio(McpStdioServerConfig),
    #[serde(rename = "sse")]
    Sse(McpSseServerConfig),
    #[serde(rename = "http")]
    Http(McpHttpServerConfig),
}

/// For backward compatibility: when `type` is omitted, default to stdio.
impl McpServerConfig {
    /// Parse from a serde_json::Value, defaulting to stdio when "type" is absent.
    pub fn from_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        // If "type" is missing, assume stdio
        if let Some(obj) = value.as_object() {
            if !obj.contains_key("type") {
                let mut obj = obj.clone();
                obj.insert("type".to_string(), serde_json::Value::String("stdio".to_string()));
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
    #[serde(default)]
    pub mime_type: Option<String>,
    pub server: String,
}

/// An MCP tool definition as received from the server.
/// Matches TS `SerializedTool` / the MCP SDK `Tool` type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Option<serde_json::Value>,
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
    pub const PING: &str = "ping";
}
