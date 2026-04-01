use anyhow::Result;
use tokio::net::TcpListener;

use super::protocol::{BridgeRequest, BridgeResponse};
use super::types::BridgeConfig;

/// A TCP server that accepts IDE bridge connections.
///
/// Each connection speaks newline-delimited JSON: one `BridgeRequest` per line
/// in, one `BridgeResponse` per line out. This mirrors the framing used by the
/// TS `DirectConnectSessionManager` (which also line-splits WebSocket frames)
/// and the `StructuredIO` stdin/stdout protocol.
pub struct BridgeServer {
    config: BridgeConfig,
}

impl BridgeServer {
    pub fn new(config: BridgeConfig) -> Self {
        Self { config }
    }

    /// Bind to the configured address and accept connections in a loop.
    ///
    /// Each connection is handled on its own Tokio task.  The method returns
    /// only if the listener itself fails.
    pub async fn start(&self) -> Result<()> {
        let addr = format!("{}:{}", self.config.host, self.config.port.unwrap_or(0));
        let listener = TcpListener::bind(&addr).await?;
        let local_addr = listener.local_addr()?;
        tracing::info!("Bridge server listening on {}", local_addr);

        loop {
            let (stream, addr) = listener.accept().await?;
            tracing::info!("IDE connected from {}", addr);
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream).await {
                    tracing::warn!("Bridge connection error: {}", e);
                }
            });
        }
    }

    /// Bind, accept **one** connection, and return the local address.
    ///
    /// Useful for integration tests that need to know which port was assigned
    /// and want to drive the connection themselves.
    pub async fn start_once(&self) -> Result<std::net::SocketAddr> {
        let addr = format!("{}:{}", self.config.host, self.config.port.unwrap_or(0));
        let listener = TcpListener::bind(&addr).await?;
        let local_addr = listener.local_addr()?;
        tracing::info!("Bridge server (one-shot) listening on {}", local_addr);

        tokio::spawn(async move {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    tracing::info!("IDE connected from {}", addr);
                    if let Err(e) = handle_connection(stream).await {
                        tracing::warn!("Bridge connection error: {}", e);
                    }
                }
                Err(e) => {
                    tracing::warn!("Bridge accept error: {}", e);
                }
            }
        });

        Ok(local_addr)
    }
}

/// Shared state for the bridge server, holding pending prompts and file-change
/// notifications so the main query loop can pick them up.
pub struct BridgeState {
    /// Prompts queued by IDE `prompt` requests, consumed by the engine.
    pub pending_prompts: std::collections::VecDeque<PendingPrompt>,
    /// File change notifications from the IDE.
    pub file_changes: Vec<FileChange>,
    /// Current session status.
    pub session_state: String,
    /// Model name.
    pub model: String,
    /// Number of messages in the conversation.
    pub message_count: usize,
    /// Whether the engine is currently processing.
    pub engine_busy: bool,
}

impl Default for BridgeState {
    fn default() -> Self {
        Self {
            pending_prompts: std::collections::VecDeque::new(),
            file_changes: Vec::new(),
            session_state: "ready".to_string(),
            model: String::new(),
            message_count: 0,
            engine_busy: false,
        }
    }
}

/// A prompt submitted by the IDE.
pub struct PendingPrompt {
    pub text: String,
    pub request_id: String,
}

/// A file-change notification from the IDE.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub change_type: String,
}

/// Route a single request to the appropriate handler and produce a response.
///
/// Supports: ping, status, get_status, prompt, file_changed, get_diagnostics.
pub fn dispatch_request(
    request: &BridgeRequest,
    state: Option<&std::sync::Arc<std::sync::Mutex<BridgeState>>>,
) -> BridgeResponse {
    match request.method.as_str() {
        "ping" => BridgeResponse::success(
            request.id.clone(),
            serde_json::json!({"pong": true}),
        ),

        "status" | "get_status" => {
            if let Some(state) = state {
                if let Ok(s) = state.lock() {
                    return BridgeResponse::success(
                        request.id.clone(),
                        serde_json::json!({
                            "state": s.session_state,
                            "model": s.model,
                            "message_count": s.message_count,
                            "engine_busy": s.engine_busy,
                        }),
                    );
                }
            }
            BridgeResponse::success(
                request.id.clone(),
                serde_json::json!({"state": "ready"}),
            )
        }

        "prompt" => {
            let text = request.params.get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if text.is_empty() {
                return BridgeResponse::error(
                    request.id.clone(),
                    -2,
                    "Missing 'text' parameter".to_string(),
                );
            }

            if let Some(state) = state {
                if let Ok(mut s) = state.lock() {
                    s.pending_prompts.push_back(PendingPrompt {
                        text: text.clone(),
                        request_id: request.id.clone(),
                    });
                    return BridgeResponse::success(
                        request.id.clone(),
                        serde_json::json!({"queued": true, "prompt": text}),
                    );
                }
            }
            BridgeResponse::error(
                request.id.clone(),
                -3,
                "Bridge state not available".to_string(),
            )
        }

        "file_changed" => {
            let path = request.params.get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let change_type = request.params.get("change_type")
                .and_then(|v| v.as_str())
                .unwrap_or("modified")
                .to_string();

            if path.is_empty() {
                return BridgeResponse::error(
                    request.id.clone(),
                    -2,
                    "Missing 'path' parameter".to_string(),
                );
            }

            if let Some(state) = state {
                if let Ok(mut s) = state.lock() {
                    s.file_changes.push(FileChange {
                        path: path.clone(),
                        change_type: change_type.clone(),
                    });
                    return BridgeResponse::success(
                        request.id.clone(),
                        serde_json::json!({"acknowledged": true, "path": path}),
                    );
                }
            }
            BridgeResponse::error(
                request.id.clone(),
                -3,
                "Bridge state not available".to_string(),
            )
        }

        "get_diagnostics" => {
            BridgeResponse::success(
                request.id.clone(),
                serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "uptime_secs": 0,
                    "supported_methods": [
                        "ping", "status", "get_status", "prompt",
                        "file_changed", "get_diagnostics"
                    ],
                }),
            )
        }

        _ => BridgeResponse::error(
            request.id.clone(),
            -1,
            format!("Unknown method: {}", request.method),
        ),
    }
}

/// Backwards-compatible dispatch without state (used by existing callers).
pub fn dispatch_request_stateless(request: &BridgeRequest) -> BridgeResponse {
    dispatch_request(request, None)
}

/// Handle a single bridge TCP connection (newline-delimited JSON).
async fn handle_connection(stream: tokio::net::TcpStream) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let request: BridgeRequest = serde_json::from_str(&line)?;
        let response = dispatch_request_stateless(&request);
        let json = serde_json::to_string(&response)?;
        writer.write_all(json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
    }

    Ok(())
}
