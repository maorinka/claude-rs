use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};

use super::types::*;

/// An LSP client that communicates with a language server via stdio JSON-RPC.
///
/// Uses Content-Length framed JSON-RPC messages (standard LSP base protocol).
/// The client spawns the language server process, performs the initialization
/// handshake, and provides methods for sending LSP requests and notifications.
pub struct LspClient {
    /// Name of this server (for logging and identification).
    name: String,
    /// The spawned server process.
    process: Option<Child>,
    /// Stdin writer to the server process.
    writer: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    /// Pending requests awaiting responses, keyed by request ID.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    /// Monotonically increasing request ID counter.
    next_id: Arc<AtomicU64>,
    /// Whether the client has been initialized.
    initialized: bool,
    /// Handle for the reader task.
    reader_handle: Option<tokio::task::JoinHandle<()>>,
    /// Document version counters for didChange notifications.
    document_versions: HashMap<String, i64>,
}

impl LspClient {
    /// Start an LSP server process and prepare for communication.
    ///
    /// Spawns the server process with the given command and args, then starts
    /// the reader task to process incoming messages. Does NOT perform the
    /// initialization handshake -- call `initialize()` after this.
    pub async fn start(name: &str, command: &str, args: &[String]) -> Result<Self> {
        debug!(server = name, command = command, "Starting LSP server process");

        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().with_context(|| {
            format!(
                "Failed to spawn LSP server '{}': command '{}'",
                name, command
            )
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to capture stdout of LSP server '{}'", name))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to capture stdin of LSP server '{}'", name))?;

        // Capture stderr for logging
        let stderr = child.stderr.take();
        if let Some(stderr) = stderr {
            let server_name = name.to_string();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(server = %server_name, "[LSP STDERR] {}", line);
                }
            });
        }

        let writer = Arc::new(Mutex::new(Some(stdin)));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let next_id = Arc::new(AtomicU64::new(1));

        // Start reader task to process responses from the server using
        // Content-Length framed messages.
        let pending_clone = pending.clone();
        let server_name = name.to_string();
        let reader_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();

            loop {
                // Read more data into the buffer
                let mut tmp = [0u8; 4096];
                match reader.read(&mut tmp).await {
                    Ok(0) => {
                        debug!(server = %server_name, "LSP server stdout closed");
                        break;
                    }
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    Err(e) => {
                        debug!(server = %server_name, "LSP server read error: {}", e);
                        break;
                    }
                }

                // Process all complete messages in the buffer
                loop {
                    let Some((content_length, header_end)) = parse_content_length(&buf) else {
                        break;
                    };

                    let total_len = header_end + content_length;
                    if buf.len() < total_len {
                        // Not enough data yet for the full message body
                        break;
                    }

                    // Extract the message body
                    let body = &buf[header_end..total_len];
                    if let Ok(body_str) = std::str::from_utf8(body) {
                        // Try to parse as a response (has an "id" field)
                        if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(body_str) {
                            let mut pending = pending_clone.lock().await;
                            if let Some(sender) = pending.remove(&response.id) {
                                let _ = sender.send(response);
                            }
                        } else {
                            // Might be a notification (e.g. publishDiagnostics) - log it
                            debug!(
                                server = %server_name,
                                "LSP server notification/message: {}",
                                &body_str[..body_str.len().min(200)]
                            );
                        }
                    }

                    // Remove the processed message from the buffer
                    buf = buf[total_len..].to_vec();
                }
            }

            debug!(server = %server_name, "LSP reader task ended");
        });

        Ok(Self {
            name: name.to_string(),
            process: Some(child),
            writer,
            pending,
            next_id,
            initialized: false,
            reader_handle: Some(reader_handle),
            document_versions: HashMap::new(),
        })
    }

    /// Perform the LSP initialization handshake.
    ///
    /// Sends `initialize` request with client capabilities and workspace info,
    /// then sends `initialized` notification. Matches the TS InitializeParams.
    pub async fn initialize(&mut self, root_uri: &str) -> Result<Value> {
        let init_params = serde_json::json!({
            "processId": std::process::id(),
            "initializationOptions": {},
            "workspaceFolders": [{
                "uri": root_uri,
                "name": root_uri.rsplit('/').next().unwrap_or("workspace")
            }],
            "rootUri": root_uri,
            "capabilities": {
                "workspace": {
                    "configuration": false,
                    "workspaceFolders": false
                },
                "textDocument": {
                    "synchronization": {
                        "dynamicRegistration": false,
                        "willSave": false,
                        "willSaveWaitUntil": false,
                        "didSave": true
                    },
                    "publishDiagnostics": {
                        "relatedInformation": true,
                        "tagSupport": { "valueSet": [1, 2] },
                        "versionSupport": false,
                        "codeDescriptionSupport": true,
                        "dataSupport": false
                    },
                    "hover": {
                        "dynamicRegistration": false,
                        "contentFormat": ["markdown", "plaintext"]
                    },
                    "definition": {
                        "dynamicRegistration": false,
                        "linkSupport": true
                    },
                    "references": {
                        "dynamicRegistration": false
                    },
                    "documentSymbol": {
                        "dynamicRegistration": false,
                        "hierarchicalDocumentSymbolSupport": true
                    },
                    "callHierarchy": {
                        "dynamicRegistration": false
                    }
                },
                "general": {
                    "positionEncodings": ["utf-16"]
                }
            }
        });

        let response = self
            .send_request(methods::INITIALIZE, Some(init_params))
            .await
            .context("LSP initialization handshake failed")?;

        let result = if let Some(result) = response.result {
            result
        } else if let Some(err) = response.error {
            return Err(anyhow!(
                "LSP initialization error from '{}': {} (code: {})",
                self.name,
                err.message,
                err.code
            ));
        } else {
            return Err(anyhow!(
                "LSP initialization returned empty response from '{}'",
                self.name
            ));
        };

        // Send initialized notification
        self.send_notification(methods::INITIALIZED, None).await?;

        self.initialized = true;
        debug!(server = self.name, "LSP initialization complete");

        Ok(result)
    }

    /// Notify the server that a text document was opened.
    ///
    /// Sends `textDocument/didOpen` notification with the full document content.
    pub async fn text_document_did_open(
        &mut self,
        uri: &str,
        language_id: &str,
        text: &str,
    ) -> Result<()> {
        self.document_versions.insert(uri.to_string(), 1);

        self.send_notification(
            methods::TEXT_DOCUMENT_DID_OPEN,
            Some(serde_json::json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text
                }
            })),
        )
        .await
    }

    /// Notify the server that a text document was changed.
    ///
    /// Sends `textDocument/didChange` notification with full document sync
    /// (sends complete new content). Increments the document version counter.
    pub async fn text_document_did_change(
        &mut self,
        uri: &str,
        text: &str,
    ) -> Result<()> {
        let version = self
            .document_versions
            .entry(uri.to_string())
            .or_insert(0);
        *version += 1;
        let current_version = *version;

        self.send_notification(
            methods::TEXT_DOCUMENT_DID_CHANGE,
            Some(serde_json::json!({
                "textDocument": {
                    "uri": uri,
                    "version": current_version
                },
                "contentChanges": [{
                    "text": text
                }]
            })),
        )
        .await
    }

    /// Request diagnostics for a text document.
    ///
    /// Sends `textDocument/diagnostic` request (LSP 3.17+ pull diagnostics).
    /// Falls back to returning an empty list if the server does not support
    /// pull diagnostics (diagnostics will arrive via publishDiagnostics notifications).
    pub async fn text_document_diagnostics(
        &self,
        uri: &str,
    ) -> Result<Vec<Diagnostic>> {
        let params = serde_json::json!({
            "textDocument": { "uri": uri }
        });

        let response = self
            .send_request("textDocument/diagnostic", Some(params))
            .await;

        match response {
            Ok(resp) => {
                if let Some(result) = resp.result {
                    // Parse diagnostics from the response.
                    // The pull diagnostic response has { items: Diagnostic[] }
                    if let Some(items) = result.get("items") {
                        let diagnostics: Vec<Diagnostic> =
                            serde_json::from_value(items.clone()).unwrap_or_default();
                        return Ok(diagnostics);
                    }
                    // Some servers may return a different format
                    if let Ok(diagnostics) = serde_json::from_value::<Vec<Diagnostic>>(result) {
                        return Ok(diagnostics);
                    }
                }
                Ok(vec![])
            }
            Err(e) => {
                // Server may not support pull diagnostics - that's fine
                debug!(
                    server = self.name,
                    "Pull diagnostics not supported, relying on push: {}", e
                );
                Ok(vec![])
            }
        }
    }

    /// Send an arbitrary LSP request and return the raw response value.
    ///
    /// Used for operations like textDocument/definition, textDocument/hover, etc.
    pub async fn send_lsp_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Option<Value>> {
        let response = self.send_request(method, params).await?;

        if let Some(err) = response.error {
            return Err(anyhow!(
                "LSP request '{}' error from '{}': {} (code: {})",
                method,
                self.name,
                err.message,
                err.code
            ));
        }

        Ok(response.result)
    }

    /// Gracefully shut down the LSP server.
    ///
    /// Sends `shutdown` request followed by `exit` notification, then kills the process.
    pub async fn shutdown(&mut self) -> Result<()> {
        if !self.initialized {
            // Nothing to shut down
            self.kill_process().await;
            return Ok(());
        }

        debug!(server = self.name, "Shutting down LSP server");

        // Send shutdown request (best-effort)
        let shutdown_result = self.send_request(methods::SHUTDOWN, None).await;
        if let Err(e) = &shutdown_result {
            warn!(
                server = self.name,
                "LSP shutdown request failed: {}", e
            );
        }

        // Send exit notification (best-effort)
        let _ = self.send_notification(methods::EXIT, None).await;

        self.initialized = false;
        self.kill_process().await;

        debug!(server = self.name, "LSP server shut down");
        Ok(())
    }

    /// Get the server name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Check if the client has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Send a JSON-RPC request via Content-Length framed stdio and wait for the response.
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

        // Serialize and send with Content-Length framing
        let body = serde_json::to_string(&request)?;
        let message = encode_message(&body);

        {
            let mut writer_guard = self.writer.lock().await;
            if let Some(writer) = writer_guard.as_mut() {
                writer
                    .write_all(&message)
                    .await
                    .with_context(|| format!("Failed to write to LSP server '{}'", self.name))?;
                writer
                    .flush()
                    .await
                    .with_context(|| format!("Failed to flush LSP server '{}'", self.name))?;
            } else {
                return Err(anyhow!("LSP server '{}' writer is closed", self.name));
            }
        }

        debug!(server = self.name, method = method, id = id, "Sent LSP request");

        // Wait for response with timeout
        let timeout = Duration::from_millis(LSP_REQUEST_TIMEOUT_MS);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(anyhow!(
                "LSP server '{}' response channel closed for request {}",
                self.name,
                id
            )),
            Err(_) => {
                let mut pending = self.pending.lock().await;
                pending.remove(&id);
                Err(anyhow!(
                    "LSP request to '{}' timed out (method: {})",
                    self.name,
                    method
                ))
            }
        }
    }

    /// Send a JSON-RPC notification (no response expected) with Content-Length framing.
    async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<()> {
        let notification = JsonRpcNotification::new(method, params);
        let body = serde_json::to_string(&notification)?;
        let message = encode_message(&body);

        {
            let mut writer_guard = self.writer.lock().await;
            if let Some(writer) = writer_guard.as_mut() {
                writer
                    .write_all(&message)
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to write notification to LSP server '{}'",
                            self.name
                        )
                    })?;
                writer
                    .flush()
                    .await
                    .with_context(|| format!("Failed to flush LSP server '{}'", self.name))?;
            } else {
                return Err(anyhow!("LSP server '{}' writer is closed", self.name));
            }
        }

        debug!(server = self.name, method = method, "Sent LSP notification");
        Ok(())
    }

    /// Kill the server process and clean up.
    async fn kill_process(&mut self) {
        // Close the writer
        {
            let mut writer = self.writer.lock().await;
            *writer = None;
        }

        // Kill the process
        if let Some(ref mut child) = self.process {
            let _ = child.kill().await;
        }
        self.process = None;

        // Abort the reader task
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Best-effort cleanup: the process is set to kill_on_drop already,
        // but we also abort the reader task to avoid leaked tasks.
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_message_construction() {
        let req = JsonRpcRequest::new(
            42,
            methods::INITIALIZE,
            Some(serde_json::json!({"processId": 1234})),
        );

        let body = serde_json::to_string(&req).unwrap();
        let message = encode_message(&body);
        let message_str = String::from_utf8(message).unwrap();

        // Verify Content-Length header
        assert!(message_str.starts_with("Content-Length: "));
        assert!(message_str.contains("\r\n\r\n"));

        // Verify the body is valid JSON-RPC
        let parts: Vec<&str> = message_str.splitn(2, "\r\n\r\n").collect();
        assert_eq!(parts.len(), 2);
        let parsed: JsonRpcRequest = serde_json::from_str(parts[1]).unwrap();
        assert_eq!(parsed.id, 42);
        assert_eq!(parsed.method, "initialize");
    }

    #[test]
    fn test_notification_message_construction() {
        let notif = JsonRpcNotification::new(
            methods::TEXT_DOCUMENT_DID_OPEN,
            Some(serde_json::json!({
                "textDocument": {
                    "uri": "file:///test.rs",
                    "languageId": "rust",
                    "version": 1,
                    "text": "fn main() {}"
                }
            })),
        );

        let body = serde_json::to_string(&notif).unwrap();
        let message = encode_message(&body);
        let message_str = String::from_utf8(message).unwrap();

        // Verify Content-Length framing
        let (content_length, header_end) =
            parse_content_length(message_str.as_bytes()).unwrap();
        assert_eq!(content_length, body.len());

        // Verify the extracted body matches
        let extracted = &message_str[header_end..header_end + content_length];
        assert_eq!(extracted, body);
    }

    #[test]
    fn test_did_open_params_structure() {
        let params = serde_json::json!({
            "textDocument": {
                "uri": "file:///home/user/test.rs",
                "languageId": "rust",
                "version": 1,
                "text": "fn main() { println!(\"hello\"); }"
            }
        });

        let td = params.get("textDocument").unwrap();
        assert_eq!(td["uri"], "file:///home/user/test.rs");
        assert_eq!(td["languageId"], "rust");
        assert_eq!(td["version"], 1);
        assert!(td["text"].as_str().unwrap().contains("println"));
    }

    #[test]
    fn test_did_change_params_structure() {
        let version = 3;
        let params = serde_json::json!({
            "textDocument": {
                "uri": "file:///test.rs",
                "version": version
            },
            "contentChanges": [{
                "text": "fn main() { /* changed */ }"
            }]
        });

        assert_eq!(params["textDocument"]["version"], 3);
        assert_eq!(params["contentChanges"][0]["text"], "fn main() { /* changed */ }");
    }

    #[test]
    fn test_initialize_params_structure() {
        let root_uri = "file:///home/user/project";
        let params = serde_json::json!({
            "processId": 1234,
            "initializationOptions": {},
            "workspaceFolders": [{
                "uri": root_uri,
                "name": "project"
            }],
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "hover": {
                        "dynamicRegistration": false,
                        "contentFormat": ["markdown", "plaintext"]
                    }
                }
            }
        });

        assert_eq!(params["processId"], 1234);
        assert_eq!(params["rootUri"], root_uri);
        assert_eq!(params["workspaceFolders"][0]["name"], "project");
    }
}
