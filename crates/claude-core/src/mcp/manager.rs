use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use super::client::{build_mcp_tool_name, McpClient};
use super::types::*;

/// Manages multiple MCP server connections.
///
/// Handles loading configs, connecting to servers, tracking connection status,
/// merging MCP tools into the tool registry, and providing a unified interface
/// for tool calls across all connected servers.
pub struct McpManager {
    /// Connected MCP clients keyed by server name.
    clients: Arc<RwLock<HashMap<String, McpClient>>>,
    /// Connection status for all configured servers.
    connections: Arc<RwLock<HashMap<String, McpServerConnection>>>,
    /// Cached tool definitions from all connected servers.
    /// Maps normalized tool name -> (server_name, original_tool_name).
    tool_map: Arc<RwLock<HashMap<String, (String, String)>>>,
    /// Cached tool definitions for all connected servers.
    tool_definitions: Arc<RwLock<Vec<McpToolInfo>>>,
}

/// Extended tool info including server association.
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    /// Normalized tool name: mcp__{server}__{tool}
    pub name: String,
    /// Original tool name from the MCP server.
    pub original_name: String,
    /// Server this tool belongs to.
    pub server_name: String,
    /// Tool description.
    pub description: Option<String>,
    /// Input schema.
    pub input_schema: Option<Value>,
}

impl McpManager {
    /// Create a new MCP manager with no servers.
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
            tool_map: Arc::new(RwLock::new(HashMap::new())),
            tool_definitions: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Connect to all configured MCP servers.
    ///
    /// Takes a map of server name -> config. Attempts to connect to each server
    /// and tracks the connection status.
    pub async fn connect_all(
        &self,
        configs: HashMap<String, ScopedMcpServerConfig>,
    ) -> Vec<McpServerConnection> {
        let mut results = Vec::new();

        for (name, scoped_config) in configs {
            let result = self.connect_server(&name, scoped_config).await;
            results.push(result);
        }

        // Refresh tool definitions after connecting
        if let Err(e) = self.refresh_tools().await {
            warn!("Failed to refresh MCP tools after connecting: {}", e);
        }

        results
    }

    /// Connect to a single MCP server.
    pub async fn connect_server(
        &self,
        name: &str,
        scoped_config: ScopedMcpServerConfig,
    ) -> McpServerConnection {
        debug!(server = name, "Connecting to MCP server");

        // Set pending status
        let pending = McpServerConnection {
            name: name.to_string(),
            status: McpConnectionStatus::Pending {
                reconnect_attempt: None,
                max_reconnect_attempts: None,
            },
            config: scoped_config.clone(),
        };
        {
            let mut connections = self.connections.write().await;
            connections.insert(name.to_string(), pending);
        }

        match &scoped_config.config {
            McpServerConfig::Stdio(stdio_config) => {
                match McpClient::connect_stdio(name, stdio_config).await {
                    Ok(client) => {
                        let capabilities = client.capabilities().cloned().unwrap_or_default();
                        let server_info = client.server_info().cloned();
                        let instructions = client.instructions().map(|s| s.to_string());

                        let conn = McpServerConnection {
                            name: name.to_string(),
                            status: McpConnectionStatus::Connected {
                                capabilities,
                                server_info,
                                instructions,
                            },
                            config: scoped_config,
                        };

                        {
                            let mut clients = self.clients.write().await;
                            clients.insert(name.to_string(), client);
                        }
                        {
                            let mut connections = self.connections.write().await;
                            connections.insert(name.to_string(), conn.clone());
                        }

                        info!(server = name, "MCP server connected");
                        conn
                    }
                    Err(e) => {
                        let error_msg = format!("{:#}", e);
                        error!(server = name, error = %error_msg, "Failed to connect to MCP server");

                        let conn = McpServerConnection {
                            name: name.to_string(),
                            status: McpConnectionStatus::Failed {
                                error: Some(error_msg),
                            },
                            config: scoped_config,
                        };

                        {
                            let mut connections = self.connections.write().await;
                            connections.insert(name.to_string(), conn.clone());
                        }

                        conn
                    }
                }
            }
            McpServerConfig::Sse(sse_config) => {
                match McpClient::connect_sse(name, sse_config).await {
                    Ok(client) => {
                        let capabilities = client.capabilities().cloned().unwrap_or_default();
                        let server_info = client.server_info().cloned();
                        let instructions = client.instructions().map(|s| s.to_string());

                        let conn = McpServerConnection {
                            name: name.to_string(),
                            status: McpConnectionStatus::Connected {
                                capabilities,
                                server_info,
                                instructions,
                            },
                            config: scoped_config,
                        };

                        {
                            let mut clients = self.clients.write().await;
                            clients.insert(name.to_string(), client);
                        }
                        {
                            let mut connections = self.connections.write().await;
                            connections.insert(name.to_string(), conn.clone());
                        }

                        info!(server = name, "MCP SSE server connected");
                        conn
                    }
                    Err(e) => {
                        let error_msg = format!("{:#}", e);
                        error!(server = name, error = %error_msg, "Failed to connect to MCP SSE server");

                        let conn = McpServerConnection {
                            name: name.to_string(),
                            status: McpConnectionStatus::Failed {
                                error: Some(error_msg),
                            },
                            config: scoped_config,
                        };

                        {
                            let mut connections = self.connections.write().await;
                            connections.insert(name.to_string(), conn.clone());
                        }

                        conn
                    }
                }
            }
            McpServerConfig::Http(http_config) => {
                match McpClient::connect_http(name, http_config).await {
                    Ok(client) => {
                        let capabilities = client.capabilities().cloned().unwrap_or_default();
                        let server_info = client.server_info().cloned();
                        let instructions = client.instructions().map(|s| s.to_string());

                        let conn = McpServerConnection {
                            name: name.to_string(),
                            status: McpConnectionStatus::Connected {
                                capabilities,
                                server_info,
                                instructions,
                            },
                            config: scoped_config,
                        };

                        {
                            let mut clients = self.clients.write().await;
                            clients.insert(name.to_string(), client);
                        }
                        {
                            let mut connections = self.connections.write().await;
                            connections.insert(name.to_string(), conn.clone());
                        }

                        info!(server = name, "MCP HTTP server connected");
                        conn
                    }
                    Err(e) => {
                        let error_msg = format!("{:#}", e);
                        error!(server = name, error = %error_msg, "Failed to connect to MCP HTTP server");

                        let conn = McpServerConnection {
                            name: name.to_string(),
                            status: McpConnectionStatus::Failed {
                                error: Some(error_msg),
                            },
                            config: scoped_config,
                        };

                        {
                            let mut connections = self.connections.write().await;
                            connections.insert(name.to_string(), conn.clone());
                        }

                        conn
                    }
                }
            }
        }
    }

    /// Refresh the cached tool definitions from all connected servers.
    pub async fn refresh_tools(&self) -> Result<()> {
        let mut all_tools = Vec::new();
        let mut tool_map = HashMap::new();

        let clients = self.clients.read().await;
        for (server_name, client) in clients.iter() {
            match client.list_tools().await {
                Ok(tools) => {
                    for tool in tools {
                        let normalized_name = build_mcp_tool_name(server_name, &tool.name);

                        // Truncate description if too long
                        let description = tool.description.map(|d| {
                            if d.len() > MAX_MCP_DESCRIPTION_LENGTH {
                                format!("{}...", &d[..MAX_MCP_DESCRIPTION_LENGTH])
                            } else {
                                d
                            }
                        });

                        tool_map.insert(
                            normalized_name.clone(),
                            (server_name.clone(), tool.name.clone()),
                        );

                        all_tools.push(McpToolInfo {
                            name: normalized_name,
                            original_name: tool.name,
                            server_name: server_name.clone(),
                            description,
                            input_schema: tool.input_schema,
                        });
                    }
                }
                Err(e) => {
                    warn!(
                        server = server_name,
                        error = %e,
                        "Failed to list tools from MCP server"
                    );
                }
            }
        }

        debug!(count = all_tools.len(), "Refreshed MCP tools");

        {
            let mut tm = self.tool_map.write().await;
            *tm = tool_map;
        }
        {
            let mut td = self.tool_definitions.write().await;
            *td = all_tools;
        }

        Ok(())
    }

    /// Call a tool on an MCP server by its normalized name.
    ///
    /// Looks up the server and original tool name, then delegates to the client.
    pub async fn call_tool(
        &self,
        normalized_name: &str,
        arguments: Value,
    ) -> Result<McpToolResult> {
        let (server_name, original_tool_name) = {
            let tool_map = self.tool_map.read().await;
            tool_map
                .get(normalized_name)
                .cloned()
                .ok_or_else(|| anyhow!("Unknown MCP tool: {}", normalized_name))?
        };

        let clients = self.clients.read().await;
        let client = clients
            .get(&server_name)
            .ok_or_else(|| anyhow!("MCP server '{}' is not connected", server_name))?;

        client.call_tool(&original_tool_name, arguments).await
    }

    /// List resources from all connected servers.
    pub async fn list_resources(&self) -> Result<Vec<ServerResource>> {
        let mut all_resources = Vec::new();

        let clients = self.clients.read().await;
        for (server_name, client) in clients.iter() {
            match client.list_resources().await {
                Ok(resources) => {
                    all_resources.extend(resources);
                }
                Err(e) => {
                    warn!(
                        server = server_name,
                        error = %e,
                        "Failed to list resources from MCP server"
                    );
                }
            }
        }

        Ok(all_resources)
    }

    /// Read a resource by URI. Tries to determine the server from registered resources.
    pub async fn read_resource(&self, server_name: &str, uri: &str) -> Result<Value> {
        let clients = self.clients.read().await;
        let client = clients
            .get(server_name)
            .ok_or_else(|| anyhow!("MCP server '{}' is not connected", server_name))?;

        client.read_resource(uri).await
    }

    /// Get all tool definitions from connected servers.
    pub async fn tool_definitions(&self) -> Vec<McpToolInfo> {
        self.tool_definitions.read().await.clone()
    }

    /// Get all server connection statuses.
    pub async fn connections(&self) -> Vec<McpServerConnection> {
        self.connections.read().await.values().cloned().collect()
    }

    /// Get a specific server's connection status.
    pub async fn connection(&self, name: &str) -> Option<McpServerConnection> {
        self.connections.read().await.get(name).cloned()
    }

    /// Get the names of all connected servers.
    pub async fn connected_server_names(&self) -> Vec<String> {
        self.clients.read().await.keys().cloned().collect()
    }

    /// Check if a specific server is connected.
    pub async fn is_connected(&self, name: &str) -> bool {
        self.clients.read().await.contains_key(name)
    }

    /// Disconnect a specific server.
    pub async fn disconnect_server(&self, name: &str) {
        {
            let mut clients = self.clients.write().await;
            if let Some(mut client) = clients.remove(name) {
                client.disconnect().await;
            }
        }
        {
            let mut connections = self.connections.write().await;
            connections.remove(name);
        }

        // Refresh tools after disconnection
        if let Err(e) = self.refresh_tools().await {
            warn!("Failed to refresh MCP tools after disconnect: {}", e);
        }

        info!(server = name, "MCP server disconnected");
    }

    /// Disconnect all servers and clean up.
    pub async fn disconnect_all(&self) {
        let server_names: Vec<String> = {
            let clients = self.clients.read().await;
            clients.keys().cloned().collect()
        };

        for name in server_names {
            self.disconnect_server(&name).await;
        }
    }

    /// Check if there are any connected servers.
    pub async fn has_connections(&self) -> bool {
        !self.clients.read().await.is_empty()
    }

    /// Get the count of connected servers.
    pub async fn connected_count(&self) -> usize {
        self.clients.read().await.len()
    }

    /// Try to resolve an MCP tool name to its server and original name.
    pub async fn resolve_tool(&self, normalized_name: &str) -> Option<(String, String)> {
        self.tool_map.read().await.get(normalized_name).cloned()
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_manager() {
        let manager = McpManager::new();
        assert!(!manager.has_connections().await);
        assert_eq!(manager.connected_count().await, 0);
        assert!(manager.connections().await.is_empty());
        assert!(manager.tool_definitions().await.is_empty());
    }

    #[tokio::test]
    async fn test_unknown_tool_call() {
        let manager = McpManager::new();
        let result = manager
            .call_tool("mcp__unknown__tool", serde_json::json!({}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown MCP tool"));
    }

    #[tokio::test]
    async fn test_disconnect_nonexistent() {
        let manager = McpManager::new();
        // Should not panic
        manager.disconnect_server("nonexistent").await;
    }

    #[tokio::test]
    async fn test_resolve_tool_empty() {
        let manager = McpManager::new();
        assert!(manager.resolve_tool("mcp__server__tool").await.is_none());
    }
}
