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

    /// Connect to many MCP servers with bounded concurrency, and
    /// skip remote servers whose name is in the needs-auth cache.
    ///
    /// Ports the orchestration shape of TS
    /// `getMcpToolsCommandsAndResources` at
    /// `services/mcp/client.ts:2226-2403`. TS additionally fires
    /// an `onConnectionAttempt` callback per server, fetches the
    /// tools/commands/resources bundle, and handles a
    /// `hasMcpDiscoveryButNoToken` early-skip — all of which
    /// depend on the tool-layer / OAuth subsystems that aren't
    /// wired yet. This method lands the shape that doesn't need
    /// them:
    ///
    /// 1. **Auth-cache short-circuit (G2 wiring).** Remote
    ///    transports (SSE / HTTP) whose `server_name` is present
    ///    in `mcp_auth_cache` within its 15-min TTL skip the
    ///    network round-trip and surface as `NeedsAuth`
    ///    immediately. Matches TS `client.ts:2311-2319`.
    /// 2. **Local vs remote concurrency split.** Local (stdio)
    ///    servers fan out with the lower
    ///    `MCP_SERVER_CONNECTION_BATCH_SIZE` (default 3, honours
    ///    the env override from G1). Remote servers fan out at
    ///    `MCP_REMOTE_SERVER_CONNECTION_BATCH_SIZE` (default
    ///    20). Matches TS `client.ts:2264-2271`.
    /// 3. **Tool-cache refresh at end.** Same best-effort
    ///    `refresh_tools()` call `connect_all` makes.
    ///
    /// Each server's returned `McpServerConnection` reflects its
    /// final status. Order of results is NOT stable — concurrent
    /// completion order wins. TS also relies on the callback
    /// rather than a return shape for the same reason.
    pub async fn connect_all_respecting_auth_cache(
        &self,
        configs: HashMap<String, ScopedMcpServerConfig>,
    ) -> Vec<McpServerConnection> {
        use futures_util::stream::{FuturesUnordered, StreamExt};

        // Partition into local (stdio) and remote (SSE/HTTP) so
        // they can fan out at different concurrency caps.
        let (local, remote): (Vec<_>, Vec<_>) = configs
            .into_iter()
            .partition(|(_, cfg)| super::helpers::is_local_mcp_server(cfg));

        let local_batch = super::helpers::get_mcp_server_connection_batch_size();
        let remote_batch = super::helpers::get_remote_mcp_server_connection_batch_size();

        async fn drive<F, FU>(
            items: Vec<(String, ScopedMcpServerConfig)>,
            concurrency: usize,
            mut each: F,
        ) -> Vec<McpServerConnection>
        where
            F: FnMut(String, ScopedMcpServerConfig) -> FU,
            FU: std::future::Future<Output = McpServerConnection>,
        {
            let mut in_flight = FuturesUnordered::new();
            let mut iter = items.into_iter();
            let mut out = Vec::new();
            // Fill up to `concurrency` slots, then refill as each
            // completes. Zero-cost when items < concurrency.
            let concurrency = concurrency.max(1);
            for _ in 0..concurrency {
                if let Some((n, c)) = iter.next() {
                    in_flight.push(each(n, c));
                } else {
                    break;
                }
            }
            while let Some(result) = in_flight.next().await {
                out.push(result);
                if let Some((n, c)) = iter.next() {
                    in_flight.push(each(n, c));
                }
            }
            out
        }

        // Run local + remote buckets CONCURRENTLY (TS uses
        // `Promise.all([processBatched(local), processBatched(remote)])`
        // at `client.ts:2391-2400` — remote connections overlap
        // with local stdio spawns instead of waiting for the
        // whole local batch to finish first).
        let local_fut = drive(local, local_batch, |name, cfg| {
            let m = self;
            async move { m.connect_server(&name, cfg).await }
        });
        let remote_fut = drive(remote, remote_batch, |name, cfg| {
            let m = self;
            async move {
                // Auth-cache short-circuit: cached needs-auth
                // entries skip the network round-trip. TS
                // `client.ts:2311-2319`. Stdio doesn't reach
                // this branch (partitioned into `local_fut`).
                if super::auth_cache::is_mcp_auth_cached(&name) {
                    debug!(
                        server = name,
                        "Skipping connection (cached needs-auth)"
                    );
                    let conn = McpServerConnection {
                        name: name.clone(),
                        status: McpConnectionStatus::NeedsAuth,
                        config: cfg.clone(),
                    };
                    // Keep manager state consistent with the
                    // short-circuit — otherwise `connection(name)`
                    // would return None while callers think the
                    // server is known.
                    let mut connections = m.connections.write().await;
                    connections.insert(name, conn.clone());
                    conn
                } else {
                    m.connect_server(&name, cfg).await
                }
            }
        });
        let (local_results, remote_results) = tokio::join!(local_fut, remote_fut);

        // Merge + best-effort tool refresh.
        let mut results = local_results;
        results.extend(remote_results);
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
                        // G16 wiring: classify connect-time 401s
                        // as NeedsAuth (populates the auth cache +
                        // fires telemetry) instead of a generic
                        // Failed status. Matches TS's
                        // `handleRemoteAuthFailure` branch at
                        // `client.ts:340-361`.
                        let conn = if looks_like_auth_failure(&error_msg) {
                            super::auth_failure::handle_remote_auth_failure(
                                name,
                                &scoped_config,
                                super::auth_failure::RemoteTransportKind::Sse,
                            )
                        } else {
                            error!(
                                server = name,
                                error = %error_msg,
                                "Failed to connect to MCP SSE server"
                            );
                            McpServerConnection {
                                name: name.to_string(),
                                status: McpConnectionStatus::Failed {
                                    error: Some(error_msg),
                                },
                                config: scoped_config,
                            }
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
                        let conn = if looks_like_auth_failure(&error_msg) {
                            super::auth_failure::handle_remote_auth_failure(
                                name,
                                &scoped_config,
                                super::auth_failure::RemoteTransportKind::Http,
                            )
                        } else {
                            error!(
                                server = name,
                                error = %error_msg,
                                "Failed to connect to MCP HTTP server"
                            );
                            McpServerConnection {
                                name: name.to_string(),
                                status: McpConnectionStatus::Failed {
                                    error: Some(error_msg),
                                },
                                config: scoped_config,
                            }
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

    // `looks_like_auth_failure` is an internal helper used by
    // connect_server to classify a connect-time error as auth
    // failure vs generic failure. Same pattern matching as
    // `classify_call_tool_error` in client.rs, but used during
    // the connect handshake where typed errors aren't flowing
    // through anyhow::downcast.
    // (Defined below, after the impl block, since it has no
    // reason to take `&self`.)

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
        // Remove from BOTH maps before the disconnect await. If
        // `client.disconnect().await` panics, the manager is still
        // left in a consistent state (both maps show the server
        // gone) rather than half-cleared (client removed but
        // connections map still has the stale record).
        let removed_client = {
            let mut clients = self.clients.write().await;
            clients.remove(name)
        };
        {
            let mut connections = self.connections.write().await;
            connections.remove(name);
        }
        if let Some(mut client) = removed_client {
            client.disconnect().await;
        }

        // Refresh tools after disconnection
        if let Err(e) = self.refresh_tools().await {
            warn!("Failed to refresh MCP tools after disconnect: {}", e);
        }

        info!(server = name, "MCP server disconnected");
    }

    /// Reconnect a server: tear down the existing client (if any)
    /// and connect fresh. Refreshes the tool cache afterwards so
    /// subsequent tool lookups see the new session's tool list.
    ///
    /// Ports the orchestration shape of TS `reconnectMcpServerImpl`
    /// at `services/mcp/client.ts:2137-2210`. The TS function
    /// additionally fetches commands / skills / resources and
    /// assembles a bundle result for the UI layer; those pieces
    /// live in the tool layer on the Rust side (G11 orchestration
    /// will wrap this call with the bundle logic once the
    /// downstream caches are ported).
    ///
    /// Behaviour:
    /// - Current client (if any) is disconnected first — mirrors
    ///   TS's `clearServerCache` at `client.ts:2154` which
    ///   triggers `client.cleanup()` via `connectToServer.cache`.
    /// - Connect produces a fresh `McpServerConnection`. Success /
    ///   failure status matches `connect_server`'s contract.
    /// - The returned `McpServerConnection` is also recorded in
    ///   the manager's `connections` map so
    ///   `manager.connection(name)` reflects the new status.
    ///
    /// # Intentional gaps vs TS
    ///
    /// * **Keychain cache invalidation** (TS `clearKeychainCache`
    ///   at `client.ts:2152`) isn't represented here — the auth
    ///   subsystem isn't ported to Rust yet. When it is, this
    ///   method should invalidate it before `disconnect_server`.
    /// * **Command / resource / skill fetch bundle** — G11 work.
    /// * There's a benign TOCTOU window between
    ///   `is_connected(name)` and `disconnect_server(name)`: a
    ///   concurrent task could disconnect in between, turning
    ///   the disconnect call into a no-op. Subsequent
    ///   `connect_server` still runs so the outcome converges
    ///   to "reconnected" either way.
    pub async fn reconnect_server(
        &self,
        name: &str,
        scoped_config: ScopedMcpServerConfig,
    ) -> McpServerConnection {
        debug!(server = name, "Reconnecting MCP server");
        // Disconnect + drop from the map so the next connect runs
        // from a clean slate. `disconnect_server` also refreshes
        // the tool cache, which is redundant here (we refresh
        // again after the reconnect), but it's cheap and keeps
        // the tool map consistent even if the reconnect fails.
        if self.is_connected(name).await {
            self.disconnect_server(name).await;
        }

        let conn = self.connect_server(name, scoped_config).await;

        // Best-effort tool refresh so the new session's tools are
        // immediately visible. Mirrors the TS bundle behaviour
        // that re-fetches tools after reconnect.
        if matches!(conn.status, McpConnectionStatus::Connected { .. }) {
            if let Err(e) = self.refresh_tools().await {
                warn!(
                    server = name,
                    "Failed to refresh MCP tools after reconnect: {}", e
                );
            }
        }

        conn
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

/// Does this connect error look like an authentication failure
/// rather than a generic transport error? Case-insensitive match
/// on `"HTTP 401"` / `"unauthorized"` — same patterns as
/// `classify_call_tool_error` in `mcp::client` uses after a
/// tool-call error. Used by `connect_server` to flip remote
/// transport connect failures into `NeedsAuth` status via G16's
/// `handle_remote_auth_failure`.
fn looks_like_auth_failure(msg: &str) -> bool {
    msg.contains("HTTP 401") || msg.to_ascii_lowercase().contains("unauthorized")
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

    // ─── G10: reconnect_server ────────────────────────────────────

    /// Reconnect against a config that can't spawn (nonexistent
    /// binary) must still return a Failed connection and leave
    /// the manager in a clean state — not panic, not leak a
    /// previous client handle.
    #[tokio::test]
    async fn reconnect_server_failed_config_returns_failed_status() {
        let manager = McpManager::new();
        let cfg = ScopedMcpServerConfig {
            config: McpServerConfig::Stdio(McpStdioServerConfig {
                command: "/definitely/not/a/real/binary/claude_rs_test".into(),
                args: vec![],
                env: None,
            }),
            scope: ConfigScope::Project,
        };

        let conn = manager.reconnect_server("ghost", cfg).await;
        assert!(
            matches!(conn.status, McpConnectionStatus::Failed { .. }),
            "unreachable binary should produce Failed status, got {:?}",
            conn.status
        );
        // Manager should not be holding a client for this server.
        assert!(!manager.is_connected("ghost").await);
    }

    // ─── G11: connect_all_respecting_auth_cache ───────────────────

    /// Empty config map → empty result, no panic, tool refresh
    /// still runs to keep state consistent.
    #[tokio::test]
    async fn connect_all_respecting_auth_cache_empty_configs() {
        let manager = McpManager::new();
        let out = manager
            .connect_all_respecting_auth_cache(HashMap::new())
            .await;
        assert!(out.is_empty());
    }

    /// A remote server name that's in the auth cache short-
    /// circuits to NeedsAuth without hitting the network. Uses
    /// an unreachable URL so network contact would fail loudly.
    #[tokio::test]
    async fn connect_all_respecting_auth_cache_skips_cached_remote() {
        use crate::mcp::auth_cache::{
            clear_mcp_auth_cache, set_mcp_auth_cache_entry, shared_test_lock,
        };
        let _g = shared_test_lock().lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path());
        clear_mcp_auth_cache();
        set_mcp_auth_cache_entry("cached-sse");

        let manager = McpManager::new();
        let mut configs = HashMap::new();
        configs.insert(
            "cached-sse".to_string(),
            ScopedMcpServerConfig {
                config: McpServerConfig::Sse(McpSseServerConfig {
                    // Would fail loudly if we actually connected.
                    url: "https://definitely.invalid/mcp".into(),
                    headers: None,
                }),
                scope: ConfigScope::Project,
            },
        );

        let results = manager
            .connect_all_respecting_auth_cache(configs)
            .await;
        assert_eq!(results.len(), 1);
        assert!(
            matches!(results[0].status, McpConnectionStatus::NeedsAuth),
            "cached-auth remote must short-circuit to NeedsAuth, got {:?}",
            results[0].status
        );
        // And the manager's connections map reflects it.
        let stored = manager.connection("cached-sse").await;
        assert!(matches!(
            stored.expect("connection recorded").status,
            McpConnectionStatus::NeedsAuth
        ));

        clear_mcp_auth_cache();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    /// Stdio servers ignore the auth-cache check — TS
    /// client.ts:2308 only gates remote transports. A cached
    /// entry for an stdio server name must NOT short-circuit.
    #[tokio::test]
    async fn connect_all_respecting_auth_cache_stdio_ignores_cache() {
        use crate::mcp::auth_cache::{
            clear_mcp_auth_cache, set_mcp_auth_cache_entry, shared_test_lock,
        };
        let _g = shared_test_lock().lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path());
        clear_mcp_auth_cache();
        set_mcp_auth_cache_entry("stdio-srv");

        let manager = McpManager::new();
        let mut configs = HashMap::new();
        configs.insert(
            "stdio-srv".to_string(),
            ScopedMcpServerConfig {
                config: McpServerConfig::Stdio(McpStdioServerConfig {
                    command: "/not/a/real/binary/for/mcp".into(),
                    args: vec![],
                    env: None,
                }),
                scope: ConfigScope::Project,
            },
        );

        let results = manager
            .connect_all_respecting_auth_cache(configs)
            .await;
        assert_eq!(results.len(), 1);
        // Stdio doesn't short-circuit — the connect attempt runs
        // and fails on the missing binary, surfacing as Failed.
        assert!(
            matches!(results[0].status, McpConnectionStatus::Failed { .. }),
            "stdio must attempt connect despite cached-auth entry, got {:?}",
            results[0].status
        );

        clear_mcp_auth_cache();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    /// Reconnecting an unknown server is equivalent to a fresh
    /// connect — no panic, no "must exist first" precondition.
    /// Mirrors TS which treats reconnect as a first-class
    /// connect op.
    #[tokio::test]
    async fn reconnect_server_never_connected_proceeds_as_fresh_connect() {
        let manager = McpManager::new();
        let cfg = ScopedMcpServerConfig {
            config: McpServerConfig::Stdio(McpStdioServerConfig {
                command: "/not/a/real/binary".into(),
                args: vec![],
                env: None,
            }),
            scope: ConfigScope::Project,
        };

        // No prior connect_server — reconnect should still work.
        let conn = manager.reconnect_server("brand-new", cfg).await;
        // Unreachable binary → Failed; the key assertion is that
        // the call completed at all.
        assert!(matches!(conn.status, McpConnectionStatus::Failed { .. }));
    }
}
