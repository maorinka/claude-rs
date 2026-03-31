use serde::{Deserialize, Serialize};

/// Messages exchanged between the CLI and an IDE extension over the bridge.
///
/// Modeled after the TypeScript `SDKMessage` discriminated union and the
/// `DirectConnectSessionManager` protocol: each variant maps to a wire-level
/// JSON object whose `"type"` field selects the variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum BridgeMessage {
    /// Notification that a file changed on disk.
    #[serde(rename = "file_changed")]
    FileChanged {
        path: String,
        content: Option<String>,
    },

    /// A diff to be applied to a file.
    #[serde(rename = "diff")]
    Diff { path: String, diff: String },

    /// IDE asks the user (or policy engine) whether a tool invocation is allowed.
    /// Mirrors the TS `SDKControlPermissionRequest`.
    #[serde(rename = "permission_request")]
    PermissionRequest {
        tool: String,
        input: serde_json::Value,
    },

    /// Response to a permission request.
    /// Mirrors the TS `RemotePermissionResponse` with `behavior` field.
    #[serde(rename = "permission_response")]
    PermissionResponse { tool: String, allowed: bool },

    /// A user prompt sent from the IDE to the CLI.
    #[serde(rename = "prompt")]
    Prompt { text: String },

    /// A response (assistant message) sent from the CLI to the IDE.
    #[serde(rename = "response")]
    Response { text: String },

    /// Status update about the CLI's current state.
    /// Maps to the TS `SessionState` lifecycle (`starting`, `running`, etc.).
    #[serde(rename = "status")]
    Status {
        state: String,
        message: Option<String>,
    },

    /// An error that occurred during processing.
    #[serde(rename = "error")]
    Error { message: String },
}

/// Configuration for the bridge server.
///
/// Follows the shape of the TS `BridgeConfig` / `ServerConfig`, trimmed to the
/// subset needed for the TCP listener that IDE extensions connect to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    /// Port to listen on.  `None` means bind to any available port (port 0).
    pub port: Option<u16>,

    /// Host/address to bind to.
    pub host: String,

    /// Which IDE family is connecting.
    pub ide: IdeType,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            port: None,
            host: "127.0.0.1".to_string(),
            ide: IdeType::Other("unknown".to_string()),
        }
    }
}

/// The kind of IDE on the other end of the bridge.
///
/// The TS codebase distinguishes worker types (`claude_code`, `claude_code_assistant`)
/// and spawn modes; on the Rust side we keep it simple with an enum that the
/// IDE extension sends during the initial handshake.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IdeType {
    VSCode,
    JetBrains,
    Other(String),
}

/// Session lifecycle states, mirroring the TS `SessionState` union.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    Running,
    Detached,
    Stopping,
    Stopped,
}

/// Metadata about a connected session, loosely following the TS `SessionInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub status: SessionState,
    pub created_at: i64,
    pub work_dir: String,
}
