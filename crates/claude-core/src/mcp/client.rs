use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};

use super::types::*;

/// An MCP client that communicates with an MCP server via stdio transport.
///
/// The MCP protocol uses JSON-RPC 2.0 over stdin/stdout of a spawned subprocess.
/// The client handles the initialization handshake, sending requests, and
/// receiving responses.
pub struct McpClient {
    /// Name of this server (for logging and identification).
    name: String,
    /// The spawned server process (if stdio transport).
    process: Option<Child>,
    /// Stdin writer to the server process, wrapped in a mutex for shared access.
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    /// Pending requests awaiting responses, keyed by request ID.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    /// Monotonically increasing request ID counter.
    next_id: Arc<AtomicU64>,
    /// Server capabilities received during initialization.
    capabilities: Option<ServerCapabilities>,
    /// Server info received during initialization.
    server_info: Option<ServerInfo>,
    /// Server instructions received during initialization.
    instructions: Option<String>,
    /// Handle for the reader task.
    reader_handle: Option<tokio::task::JoinHandle<()>>,
}

impl McpClient {
    /// Connect to an MCP server using stdio transport.
    ///
    /// Spawns the server process, starts the reader task, and performs the
    /// MCP initialization handshake.
    pub async fn connect_stdio(
        name: &str,
        config: &McpStdioServerConfig,
    ) -> Result<Self> {
        debug!(server = name, command = %config.command, "Connecting to MCP server via stdio");

        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set environment variables if provided
        if let Some(env) = &config.env {
            for (key, value) in env {
                cmd.env(key, value);
            }
        }

        let mut child = cmd.spawn().with_context(|| {
            format!(
                "Failed to spawn MCP server '{}': command '{}'",
                name, config.command
            )
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to capture stdout of MCP server '{}'", name))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to capture stdin of MCP server '{}'", name))?;

        let writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> =
            Arc::new(Mutex::new(Some(Box::new(stdin))));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let next_id = Arc::new(AtomicU64::new(1));

        // Start reader task to process responses from the server
        let pending_clone = pending.clone();
        let server_name = name.to_string();
        let reader_handle = tokio::task::spawn_blocking(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        let line = line.trim().to_string();
                        if line.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<JsonRpcResponse>(&line) {
                            Ok(response) => {
                                let pending = pending_clone.clone();
                                // Use try_lock to avoid blocking; if we can't acquire the lock
                                // immediately, spawn a task to complete the operation.
                                let rt = tokio::runtime::Handle::try_current();
                                if let Ok(rt) = rt {
                                    rt.spawn(async move {
                                        let mut pending = pending.lock().await;
                                        if let Some(sender) = pending.remove(&response.id) {
                                            let _ = sender.send(response);
                                        }
                                    });
                                }
                            }
                            Err(e) => {
                                // Might be a notification (no id field) - that's ok
                                debug!(
                                    server = server_name,
                                    "Non-response message from MCP server: {} (parse error: {})",
                                    line,
                                    e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        debug!(server = server_name, "MCP server stdout read error: {}", e);
                        break;
                    }
                }
            }
            debug!(server = server_name, "MCP server reader task ended");
        });

        let mut client = Self {
            name: name.to_string(),
            process: Some(child),
            writer,
            pending,
            next_id,
            capabilities: None,
            server_info: None,
            instructions: None,
            reader_handle: Some(reader_handle),
        };

        // Perform initialization handshake
        client.initialize().await?;

        Ok(client)
    }

    /// Perform the MCP initialization handshake.
    ///
    /// Sends `initialize` request and then `notifications/initialized` notification.
    async fn initialize(&mut self) -> Result<()> {
        let init_params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": MCP_CLIENT_NAME,
                "version": MCP_CLIENT_VERSION
            }
        });

        let response = self
            .send_request(methods::INITIALIZE, Some(init_params))
            .await
            .context("MCP initialization handshake failed")?;

        if let Some(result) = response.result {
            // Parse capabilities
            if let Some(caps) = result.get("capabilities") {
                self.capabilities = serde_json::from_value(caps.clone()).ok();
            }

            // Parse server info
            if let Some(info) = result.get("serverInfo") {
                self.server_info = serde_json::from_value(info.clone()).ok();
            }

            // Parse instructions
            if let Some(instructions) = result.get("instructions") {
                self.instructions = instructions.as_str().map(|s| s.to_string());
            }
        } else if let Some(err) = response.error {
            return Err(anyhow!(
                "MCP initialization error from '{}': {} (code: {})",
                self.name,
                err.message,
                err.code
            ));
        }

        // Send initialized notification
        self.send_notification(methods::INITIALIZED, None).await?;

        debug!(
            server = self.name,
            capabilities = ?self.capabilities,
            server_info = ?self.server_info,
            "MCP initialization complete"
        );

        Ok(())
    }

    /// Send a JSON-RPC request and wait for the response.
    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, method, params);

        let (tx, rx) = oneshot::channel();

        // Register the pending request
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        // Serialize and send
        let msg = serde_json::to_string(&request)?;
        {
            let mut writer_guard = self.writer.lock().await;
            if let Some(writer) = writer_guard.as_mut() {
                writeln!(writer, "{}", msg)
                    .with_context(|| format!("Failed to write to MCP server '{}'", self.name))?;
                writer
                    .flush()
                    .with_context(|| format!("Failed to flush MCP server '{}'", self.name))?;
            } else {
                return Err(anyhow!("MCP server '{}' writer is closed", self.name));
            }
        }

        debug!(server = self.name, method = method, id = id, "Sent MCP request");

        // Wait for response with timeout
        let timeout = Duration::from_millis(MCP_TOOL_TIMEOUT_MS);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(anyhow!(
                "MCP server '{}' response channel closed for request {}",
                self.name,
                id
            )),
            Err(_) => {
                // Remove the pending request on timeout
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
                Err(anyhow!(
                    "MCP request to '{}' timed out (method: {})",
                    self.name,
                    method
                ))
            }
        }
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<()> {
        let notification = JsonRpcNotification::new(method, params);
        let msg = serde_json::to_string(&notification)?;

        let mut writer_guard = self.writer.lock().await;
        if let Some(writer) = writer_guard.as_mut() {
            writeln!(writer, "{}", msg)
                .with_context(|| format!("Failed to write notification to MCP server '{}'", self.name))?;
            writer
                .flush()
                .with_context(|| format!("Failed to flush MCP server '{}'", self.name))?;
        } else {
            return Err(anyhow!("MCP server '{}' writer is closed", self.name));
        }

        debug!(server = self.name, method = method, "Sent MCP notification");
        Ok(())
    }

    /// List all tools available from this MCP server.
    ///
    /// Sends a `tools/list` request and parses the response into tool definitions.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDefinition>> {
        let response = self
            .send_request(methods::TOOLS_LIST, Some(serde_json::json!({})))
            .await?;

        if let Some(result) = response.result {
            if let Some(tools) = result.get("tools") {
                let tools: Vec<McpToolDefinition> = serde_json::from_value(tools.clone())
                    .context("Failed to parse MCP tools list")?;
                debug!(server = self.name, count = tools.len(), "Listed MCP tools");
                return Ok(tools);
            }
        }

        if let Some(err) = response.error {
            return Err(anyhow!(
                "MCP tools/list error from '{}': {} (code: {})",
                self.name,
                err.message,
                err.code
            ));
        }

        Ok(vec![])
    }

    /// Call a tool on this MCP server.
    ///
    /// Sends a `tools/call` request with the given tool name and arguments.
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<McpToolResult> {
        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments
        });

        let response = self
            .send_request(methods::TOOLS_CALL, Some(params))
            .await?;

        if let Some(result) = response.result {
            let tool_result: McpToolResult = serde_json::from_value(result)
                .context("Failed to parse MCP tool call result")?;
            return Ok(tool_result);
        }

        if let Some(err) = response.error {
            return Err(anyhow!(
                "MCP tools/call error from '{}' (tool: {}): {} (code: {})",
                self.name,
                tool_name,
                err.message,
                err.code
            ));
        }

        Err(anyhow!(
            "MCP tools/call returned empty response from '{}' (tool: {})",
            self.name,
            tool_name
        ))
    }

    /// List all resources available from this MCP server.
    ///
    /// Sends a `resources/list` request.
    pub async fn list_resources(&self) -> Result<Vec<ServerResource>> {
        let response = self
            .send_request(methods::RESOURCES_LIST, Some(serde_json::json!({})))
            .await?;

        if let Some(result) = response.result {
            if let Some(resources) = result.get("resources") {
                // Parse resources and attach server name
                let raw: Vec<Value> = serde_json::from_value(resources.clone())
                    .context("Failed to parse MCP resources list")?;

                let resources = raw
                    .into_iter()
                    .filter_map(|r| {
                        Some(ServerResource {
                            uri: r.get("uri")?.as_str()?.to_string(),
                            name: r.get("name")?.as_str()?.to_string(),
                            description: r
                                .get("description")
                                .and_then(|d| d.as_str())
                                .map(|s| s.to_string()),
                            mime_type: r
                                .get("mimeType")
                                .and_then(|m| m.as_str())
                                .map(|s| s.to_string()),
                            server: self.name.clone(),
                        })
                    })
                    .collect();

                return Ok(resources);
            }
        }

        if let Some(err) = response.error {
            return Err(anyhow!(
                "MCP resources/list error from '{}': {} (code: {})",
                self.name,
                err.message,
                err.code
            ));
        }

        Ok(vec![])
    }

    /// Read a specific resource from this MCP server.
    ///
    /// Sends a `resources/read` request with the given URI.
    pub async fn read_resource(&self, uri: &str) -> Result<Value> {
        let params = serde_json::json!({
            "uri": uri
        });

        let response = self
            .send_request(methods::RESOURCES_READ, Some(params))
            .await?;

        if let Some(result) = response.result {
            return Ok(result);
        }

        if let Some(err) = response.error {
            return Err(anyhow!(
                "MCP resources/read error from '{}' (uri: {}): {} (code: {})",
                self.name,
                uri,
                err.message,
                err.code
            ));
        }

        Err(anyhow!(
            "MCP resources/read returned empty response from '{}' (uri: {})",
            self.name,
            uri
        ))
    }

    /// Ping the MCP server.
    pub async fn ping(&self) -> Result<()> {
        let response = self
            .send_request(methods::PING, None)
            .await?;

        if let Some(err) = response.error {
            return Err(anyhow!(
                "MCP ping error from '{}': {} (code: {})",
                self.name,
                err.message,
                err.code
            ));
        }

        Ok(())
    }

    /// Get the server's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the server's capabilities (set after initialization).
    pub fn capabilities(&self) -> Option<&ServerCapabilities> {
        self.capabilities.as_ref()
    }

    /// Get the server info (set after initialization).
    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server_info.as_ref()
    }

    /// Get the server instructions (set after initialization).
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// Disconnect from the MCP server.
    ///
    /// Closes the writer, kills the server process, and waits for the reader task.
    pub async fn disconnect(&mut self) {
        debug!(server = self.name, "Disconnecting from MCP server");

        // Close the writer
        {
            let mut writer = self.writer.lock().await;
            *writer = None;
        }

        // Kill the process
        if let Some(ref mut child) = self.process {
            match child.kill() {
                Ok(()) => {
                    debug!(server = self.name, "Killed MCP server process");
                }
                Err(e) => {
                    warn!(server = self.name, error = %e, "Failed to kill MCP server process");
                }
            }
            // Wait for the process to avoid zombies
            let _ = child.wait();
        }

        // Wait for the reader task to finish
        if let Some(handle) = self.reader_handle.take() {
            let _ = handle.await;
        }

        debug!(server = self.name, "MCP server disconnected");
    }

    /// Check if the MCP server process is still running.
    pub fn is_alive(&mut self) -> bool {
        if let Some(ref mut child) = self.process {
            match child.try_wait() {
                Ok(Some(_)) => false, // Process has exited
                Ok(None) => true,     // Still running
                Err(_) => false,      // Error checking status
            }
        } else {
            false
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Best-effort cleanup: kill the process if still running
        if let Some(ref mut child) = self.process {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Build the normalized MCP tool name: "mcp__{server}__{tool}".
/// Matches the TS `buildMcpToolName` function.
pub fn build_mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    let normalized_server = normalize_mcp_name(server_name);
    let normalized_tool = normalize_mcp_name(tool_name);
    format!("mcp__{}__{}", normalized_server, normalized_tool)
}

/// Normalize a name for MCP by replacing non-alphanumeric characters with underscores.
/// Matches the TS `normalizeNameForMCP` function.
pub fn normalize_mcp_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_mcp_tool_name() {
        assert_eq!(
            build_mcp_tool_name("my-server", "my-tool"),
            "mcp__my_server__my_tool"
        );
        assert_eq!(
            build_mcp_tool_name("server", "tool"),
            "mcp__server__tool"
        );
        assert_eq!(
            build_mcp_tool_name("my.server.name", "tool.name"),
            "mcp__my_server_name__tool_name"
        );
    }

    #[test]
    fn test_normalize_mcp_name() {
        assert_eq!(normalize_mcp_name("simple"), "simple");
        assert_eq!(normalize_mcp_name("with-dashes"), "with_dashes");
        assert_eq!(normalize_mcp_name("with.dots"), "with_dots");
        assert_eq!(normalize_mcp_name("with spaces"), "with_spaces");
        assert_eq!(normalize_mcp_name("UPPER"), "UPPER");
        assert_eq!(normalize_mcp_name("mix123"), "mix123");
    }
}
