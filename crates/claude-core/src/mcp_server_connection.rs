//! Plain-data representation of an MCP server connection.
//!
//! Port of TS `services/mcp/types.ts:180-226`
//! (`MCPServerConnection` + its five variants).
//!
//! The TS `ConnectedMCPServer` carries a `client: Client` handle and
//! a `cleanup: () => Promise<void>` callback — both runtime
//! behaviour, not data. The Rust port **drops both** and keeps only
//! the plain-data fields, documented below. Callers that need the
//! live client should thread their own `Arc<McpClient>` alongside
//! this struct; the data variant is what gets serialised to logs
//! and to the event stream.
//!
//! Uses the already-ported `McpServerConfig` from
//! `crate::mcp::types`.

use crate::mcp::types::ScopedMcpServerConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Discriminated union over the 5 MCP connection states. Matches TS
/// `type MCPServerConnection = Connected | Failed | NeedsAuth |
/// Pending | Disabled`.
///
/// Serialises with `type` as the discriminator, matching TS wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum McpServerConnection {
    /// `ConnectedMCPServer` — live session. TS also carries `client`
    /// plus `cleanup` fields; those are runtime behaviour stripped
    /// from this data-layer port.
    Connected(ConnectedMcpServer),
    /// `FailedMCPServer` — connect attempt errored.
    Failed(FailedMcpServer),
    /// `NeedsAuthMCPServer` — caller must run OAuth before a retry
    ///   succeeds.
    #[serde(rename = "needs-auth")]
    NeedsAuth(NeedsAuthMcpServer),
    /// `PendingMCPServer` — mid-reconnect or first-connect in flight.
    Pending(PendingMcpServer),
    /// `DisabledMCPServer` — user-disabled via settings.
    Disabled(DisabledMcpServer),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedMcpServer {
    pub name: String,
    /// MCP server capabilities advertised in initialise response.
    /// TS `ServerCapabilities` from the MCP SDK — stored as `Value`
    /// so this struct stays SDK-version-agnostic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
    /// MCP server self-identification from the initialise handshake.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "serverInfo"
    )]
    pub server_info: Option<ServerInfo>,
    /// Instructions-from-server. TS `instructions?: string`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub config: ScopedMcpServerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedMcpServer {
    pub name: String,
    pub config: ScopedMcpServerConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeedsAuthMcpServer {
    pub name: String,
    pub config: ScopedMcpServerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMcpServer {
    pub name: String,
    pub config: ScopedMcpServerConfig,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "reconnectAttempt"
    )]
    pub reconnect_attempt: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "maxReconnectAttempts"
    )]
    pub max_reconnect_attempts: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisabledMcpServer {
    pub name: String,
    pub config: ScopedMcpServerConfig,
}

impl McpServerConnection {
    /// Return the server name regardless of variant.
    pub fn name(&self) -> &str {
        match self {
            Self::Connected(c) => &c.name,
            Self::Failed(f) => &f.name,
            Self::NeedsAuth(n) => &n.name,
            Self::Pending(p) => &p.name,
            Self::Disabled(d) => &d.name,
        }
    }

    /// Return the scoped config regardless of variant.
    pub fn config(&self) -> &ScopedMcpServerConfig {
        match self {
            Self::Connected(c) => &c.config,
            Self::Failed(f) => &f.config,
            Self::NeedsAuth(n) => &n.config,
            Self::Pending(p) => &p.config,
            Self::Disabled(d) => &d.config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::types::{ConfigScope, McpServerConfig, McpStdioServerConfig};
    use serde_json::json;

    fn stdio_config() -> ScopedMcpServerConfig {
        ScopedMcpServerConfig {
            config: McpServerConfig::Stdio(McpStdioServerConfig {
                command: "/usr/bin/mcp-server".into(),
                args: vec!["--flag".into()],
                env: None,
            }),
            scope: ConfigScope::User,
        }
    }

    #[test]
    fn connected_serialises_with_type_discriminator() {
        let conn = McpServerConnection::Connected(ConnectedMcpServer {
            name: "slack".into(),
            capabilities: Some(json!({ "tools": {} })),
            server_info: Some(ServerInfo {
                name: "slack-mcp".into(),
                version: "1.0.0".into(),
            }),
            instructions: Some("Use for Slack ops.".into()),
            config: stdio_config(),
        });
        let v = serde_json::to_value(&conn).unwrap();
        assert_eq!(v["type"], json!("connected"));
        assert_eq!(v["name"], json!("slack"));
    }

    #[test]
    fn needs_auth_hyphenated_type() {
        // TS variant is literally `'needs-auth'`; the kebab-case
        // rename must preserve the hyphen on the wire.
        let conn = McpServerConnection::NeedsAuth(NeedsAuthMcpServer {
            name: "github".into(),
            config: stdio_config(),
        });
        let v = serde_json::to_value(&conn).unwrap();
        assert_eq!(v["type"], json!("needs-auth"));
    }

    #[test]
    fn pending_optional_reconnect_fields() {
        let conn = McpServerConnection::Pending(PendingMcpServer {
            name: "x".into(),
            config: stdio_config(),
            reconnect_attempt: Some(2),
            max_reconnect_attempts: Some(5),
        });
        let v = serde_json::to_value(&conn).unwrap();
        assert_eq!(v["type"], json!("pending"));
        assert_eq!(v["reconnectAttempt"], json!(2));
        assert_eq!(v["maxReconnectAttempts"], json!(5));
    }

    #[test]
    fn pending_without_reconnect_fields_omits_them() {
        let conn = McpServerConnection::Pending(PendingMcpServer {
            name: "x".into(),
            config: stdio_config(),
            reconnect_attempt: None,
            max_reconnect_attempts: None,
        });
        let v = serde_json::to_value(&conn).unwrap();
        assert!(v.as_object().unwrap().get("reconnectAttempt").is_none());
    }

    #[test]
    fn failed_with_error() {
        let conn = McpServerConnection::Failed(FailedMcpServer {
            name: "x".into(),
            config: stdio_config(),
            error: Some("connection refused".into()),
        });
        let v = serde_json::to_value(&conn).unwrap();
        assert_eq!(v["type"], json!("failed"));
        assert_eq!(v["error"], json!("connection refused"));
    }

    #[test]
    fn disabled_has_minimal_shape() {
        let conn = McpServerConnection::Disabled(DisabledMcpServer {
            name: "x".into(),
            config: stdio_config(),
        });
        let v = serde_json::to_value(&conn).unwrap();
        assert_eq!(v["type"], json!("disabled"));
        // No error / instructions / capabilities keys.
        assert!(v.as_object().unwrap().get("error").is_none());
        assert!(v.as_object().unwrap().get("instructions").is_none());
    }

    #[test]
    fn name_accessor_works_on_every_variant() {
        for conn in [
            McpServerConnection::Connected(ConnectedMcpServer {
                name: "c".into(),
                capabilities: None,
                server_info: None,
                instructions: None,
                config: stdio_config(),
            }),
            McpServerConnection::Failed(FailedMcpServer {
                name: "f".into(),
                config: stdio_config(),
                error: None,
            }),
            McpServerConnection::NeedsAuth(NeedsAuthMcpServer {
                name: "n".into(),
                config: stdio_config(),
            }),
            McpServerConnection::Pending(PendingMcpServer {
                name: "p".into(),
                config: stdio_config(),
                reconnect_attempt: None,
                max_reconnect_attempts: None,
            }),
            McpServerConnection::Disabled(DisabledMcpServer {
                name: "d".into(),
                config: stdio_config(),
            }),
        ] {
            let n = conn.name();
            assert_eq!(n.len(), 1);
        }
    }
}
