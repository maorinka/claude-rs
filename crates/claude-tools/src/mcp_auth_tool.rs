use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// ─── Auth state store ────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum AuthState {
    LoggedOut,
    LoggedIn,
}

static AUTH_STORE: Lazy<Mutex<HashMap<String, AuthState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Get the auth state for a server.
pub fn get_auth_state(server: &str) -> AuthState {
    let store = AUTH_STORE.lock().unwrap();
    store
        .get(server)
        .cloned()
        .unwrap_or(AuthState::LoggedOut)
}

/// Set the auth state for a server.
pub fn set_auth_state(server: &str, state: AuthState) {
    let mut store = AUTH_STORE.lock().unwrap();
    store.insert(server.to_string(), state);
}

/// Clear all auth state (for testing).
#[cfg(test)]
pub fn clear_auth_state() {
    let mut store = AUTH_STORE.lock().unwrap();
    store.clear();
}

// ─── McpAuthTool ─────────────────────────────────────────────────────────────

pub struct McpAuthTool;

#[async_trait]
impl ToolExecutor for McpAuthTool {
    fn name(&self) -> &str {
        "McpAuth"
    }

    fn description(&self) -> String {
        "Manages MCP server authentication. Use this tool to log in, log out, or check \
         the authentication status of an MCP server."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server_name": {
                    "type": "string",
                    "description": "The name of the MCP server to authenticate with."
                },
                "action": {
                    "type": "string",
                    "enum": ["login", "logout", "status"],
                    "description": "The authentication action to perform."
                }
            },
            "required": ["server_name", "action"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let server_name = match input.get("server_name").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: server_name" }),
                    is_error: true,
                });
            }
        };

        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: action" }),
                    is_error: true,
                });
            }
        };

        match action {
            "login" => {
                set_auth_state(server_name, AuthState::LoggedIn);
                Ok(ToolResultData {
                    data: json!({
                        "server_name": server_name,
                        "action": "login",
                        "status": "logged_in",
                        "message": format!(
                            "Successfully authenticated with MCP server '{}'. The server's tools should now be available.",
                            server_name
                        )
                    }),
                    is_error: false,
                })
            }
            "logout" => {
                set_auth_state(server_name, AuthState::LoggedOut);
                Ok(ToolResultData {
                    data: json!({
                        "server_name": server_name,
                        "action": "logout",
                        "status": "logged_out",
                        "message": format!(
                            "Logged out from MCP server '{}'. The server's tools will no longer be available.",
                            server_name
                        )
                    }),
                    is_error: false,
                })
            }
            "status" => {
                let state = get_auth_state(server_name);
                let status_str = match state {
                    AuthState::LoggedIn => "logged_in",
                    AuthState::LoggedOut => "logged_out",
                };
                Ok(ToolResultData {
                    data: json!({
                        "server_name": server_name,
                        "action": "status",
                        "status": status_str,
                        "message": format!(
                            "MCP server '{}' authentication status: {}",
                            server_name, status_str
                        )
                    }),
                    is_error: false,
                })
            }
            _ => Ok(ToolResultData {
                data: json!({
                    "error": format!(
                        "Invalid action '{}'. Must be one of: login, logout, status",
                        action
                    )
                }),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())),
        }
    }

    #[tokio::test]
    async fn mcp_auth_missing_fields() {
        let tool = McpAuthTool;
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool
            .call(&json!({}), &ctx, cancel.clone(), None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("server_name"));

        let result = tool
            .call(
                &json!({ "server_name": "test" }),
                &ctx,
                cancel,
                None,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("action"));
    }

    #[tokio::test]
    async fn mcp_auth_invalid_action() {
        let tool = McpAuthTool;
        let input = json!({ "server_name": "test", "action": "invalid" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("Invalid action"));
    }

    #[tokio::test]
    async fn mcp_auth_login_logout_status_cycle() {
        clear_auth_state();
        let tool = McpAuthTool;
        let ctx = make_ctx();

        // Check initial status (should be logged out)
        let result = tool
            .call(
                &json!({ "server_name": "my-server", "action": "status" }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["status"].as_str().unwrap(), "logged_out");

        // Login
        let result = tool
            .call(
                &json!({ "server_name": "my-server", "action": "login" }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["status"].as_str().unwrap(), "logged_in");

        // Status check (should be logged in)
        let result = tool
            .call(
                &json!({ "server_name": "my-server", "action": "status" }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["status"].as_str().unwrap(), "logged_in");

        // Logout
        let result = tool
            .call(
                &json!({ "server_name": "my-server", "action": "logout" }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["status"].as_str().unwrap(), "logged_out");

        clear_auth_state();
    }
}
