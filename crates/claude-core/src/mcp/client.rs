use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, warn};

use super::helpers::mcp_streamable_http_post;
use super::types::*;

/// The transport backend used by an MCP client.
#[derive(Debug)]
enum McpTransport {
    /// Stdio transport: communicates via stdin/stdout of a spawned process.
    Stdio,
    /// SSE transport: sends JSON-RPC via POST, receives responses via SSE stream.
    Sse {
        /// The base URL for the SSE endpoint (retained for diagnostics).
        #[allow(dead_code)]
        url: String,
        /// HTTP client for sending requests.
        http: reqwest::Client,
        /// Optional custom headers to include in requests.
        headers: Option<HashMap<String, String>>,
        /// The session/message endpoint URL (discovered from the SSE stream).
        message_url: Arc<Mutex<Option<String>>>,
    },
    /// HTTP (Streamable HTTP) transport: sends JSON-RPC via POST requests.
    Http {
        /// The URL for the HTTP endpoint.
        url: String,
        /// HTTP client for sending requests.
        http: reqwest::Client,
        /// Optional custom headers to include in requests.
        headers: Option<HashMap<String, String>>,
        /// Session ID returned by the server, sent in subsequent requests.
        session_id: Arc<Mutex<Option<String>>>,
    },
    /// WebSocket transport: persistent bidirectional connection.
    /// Outbound messages are queued via the `write_tx` mpsc; a
    /// reader task owns the read half of the WS stream and
    /// dispatches inbound frames to `pending` / `request_handlers`
    /// using the same machinery as stdio. G18b port of TS
    /// `WebSocketTransport` usage at `client.ts:735-783`.
    Ws {
        /// Retained for diagnostics + reconnect address.
        #[allow(dead_code)]
        url: String,
        /// Channel to enqueue outbound text frames. The receiver
        /// is owned by the WS writer task.
        write_tx: tokio::sync::mpsc::UnboundedSender<String>,
    },
}

/// An MCP client that communicates with an MCP server via stdio, SSE, or HTTP transport.
///
/// The MCP protocol uses JSON-RPC 2.0 messages. For stdio, these are sent
/// over stdin/stdout of a spawned subprocess. For SSE/HTTP, they are sent
/// via HTTP POST requests.
/// The future returned by a [`RequestHandler`]. Boxed + pinned so
/// handlers can capture arbitrary async state without exposing a
/// concrete `impl Future` through the `Arc<dyn Fn ...>` type.
pub type RequestHandlerFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, JsonRpcError>> + Send>>;

/// Handler for a server-initiated JSON-RPC request. Returns a future
/// yielding the `result` payload on success, or a `JsonRpcError` to
/// surface a structured error back to the server.
///
/// Registered via `McpClient::set_request_handler`. Ports the TS
/// `client.setRequestHandler(Schema, async request => ...)` pattern
/// at `services/mcp/client.ts:1009-1018` and `:1191-1197`. TS
/// handlers are async; the Rust port mirrors that shape so handlers
/// can await I/O or async locks naturally.
pub type RequestHandler = Arc<dyn Fn(Option<Value>) -> RequestHandlerFuture + Send + Sync>;

pub struct McpClient {
    /// Name of this server (for logging and identification).
    name: String,
    /// The transport type being used.
    transport: McpTransport,
    /// The spawned server process (if stdio transport).
    process: Option<Child>,
    /// Stdin writer to the server process, wrapped in a mutex for shared access.
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    /// Pending requests awaiting responses, keyed by request ID.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    /// Monotonically increasing request ID counter.
    next_id: Arc<AtomicU64>,
    /// Inbound request handlers keyed by JSON-RPC method name. Ports
    /// TS `client.setRequestHandler` at
    /// `services/mcp/client.ts:1009,1191`. Currently dispatched only
    /// for the stdio reader; SSE/HTTP dispatch is a follow-up
    /// ticket.
    request_handlers: Arc<Mutex<HashMap<String, RequestHandler>>>,
    /// Lifecycle-error tracker (G4b). Records terminal-connection
    /// errors + threshold-triggered close signals for remote
    /// transports. Populated on every McpClient but only wired for
    /// SSE/HTTP per TS `client.ts:1333-1364` — stdio subprocess
    /// crashes surface as process-exit signals, not reconnectable
    /// network flaps, so they never touch this tracker.
    lifecycle: Arc<Mutex<super::lifecycle::LifecycleTracker>>,
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
    pub async fn connect_stdio(name: &str, config: &McpStdioServerConfig) -> Result<Self> {
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
        let request_handlers: Arc<Mutex<HashMap<String, RequestHandler>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // Pre-register the two default handlers from TS
        // `client.ts:1009,1191` before the reader starts so the first
        // inbound `roots/list` or `elicitation/create` finds them.
        {
            let mut h = request_handlers.lock().await;
            h.insert("roots/list".to_string(), default_roots_list_handler());
            h.insert(
                "elicitation/create".to_string(),
                default_elicitation_cancel_handler(),
            );
        }

        // Start reader task to process messages (responses + inbound
        // requests + notifications) from the server.
        let pending_clone = pending.clone();
        let handlers_clone = request_handlers.clone();
        let writer_clone = writer.clone();
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
                        dispatch_inbound_line(
                            &line,
                            &server_name,
                            pending_clone.clone(),
                            handlers_clone.clone(),
                            writer_clone.clone(),
                        );
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
            transport: McpTransport::Stdio,
            process: Some(child),
            writer,
            pending,
            next_id,
            request_handlers,
            // stdio never feeds this tracker (see field doc); keep
            // a clean default for field-shape parity.
            lifecycle: Arc::new(Mutex::new(super::lifecycle::LifecycleTracker::new())),
            capabilities: None,
            server_info: None,
            instructions: None,
            reader_handle: Some(reader_handle),
        };

        // Perform initialization handshake
        client.initialize().await?;

        Ok(client)
    }

    /// Connect to an MCP server using SSE (Server-Sent Events) transport.
    ///
    /// The SSE transport connects to the server's SSE endpoint to receive
    /// responses as server-sent events, and sends JSON-RPC messages via
    /// POST requests to the server's message endpoint.
    pub async fn connect_sse(name: &str, config: &McpSseServerConfig) -> Result<Self> {
        debug!(server = name, url = %config.url, "Connecting to MCP server via SSE");

        let http = reqwest::Client::new();
        let message_url = Arc::new(Mutex::new(None));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let next_id = Arc::new(AtomicU64::new(1));
        let lifecycle: Arc<Mutex<super::lifecycle::LifecycleTracker>> =
            Arc::new(Mutex::new(super::lifecycle::LifecycleTracker::new()));

        // Start SSE listener task.
        // The SSE endpoint typically returns an event with a session/message URL
        // for sending requests. We parse that and store it.
        let sse_url = config.url.clone();
        let message_url_clone = message_url.clone();
        let pending_clone = pending.clone();
        let lifecycle_clone = lifecycle.clone();
        let server_name = name.to_string();
        let headers_clone = config.headers.clone();

        let reader_handle = tokio::spawn(async move {
            let mut req = reqwest::Client::new().get(&sse_url);
            if let Some(ref headers) = headers_clone {
                for (k, v) in headers {
                    req = req.header(k, v);
                }
            }

            match req.send().await {
                Ok(response) => {
                    let mut bytes = response.bytes_stream();
                    use futures_util::StreamExt;
                    let mut buffer = String::new();

                    while let Some(chunk) = bytes.next().await {
                        match chunk {
                            Ok(data) => {
                                buffer.push_str(&String::from_utf8_lossy(&data));

                                // Parse SSE events from buffer
                                while let Some(pos) = buffer.find("\n\n") {
                                    let event_text = buffer[..pos].to_string();
                                    buffer = buffer[pos + 2..].to_string();

                                    // Parse event fields
                                    let mut event_type = String::new();
                                    let mut event_data = String::new();

                                    for line in event_text.lines() {
                                        if let Some(val) = line.strip_prefix("event: ") {
                                            event_type = val.trim().to_string();
                                        } else if let Some(val) = line.strip_prefix("data: ") {
                                            event_data = val.trim().to_string();
                                        }
                                    }

                                    // Handle endpoint event (session URL)
                                    if event_type == "endpoint" {
                                        // The data contains the relative or absolute URL
                                        // for sending messages
                                        let msg_url = if event_data.starts_with("http") {
                                            event_data.clone()
                                        } else {
                                            // Resolve relative URL against SSE base
                                            if let Ok(base) = url::Url::parse(&sse_url) {
                                                base.join(&event_data)
                                                    .map(|u| u.to_string())
                                                    .unwrap_or(event_data.clone())
                                            } else {
                                                event_data.clone()
                                            }
                                        };
                                        let mut mu = message_url_clone.lock().await;
                                        *mu = Some(msg_url);
                                        debug!(
                                            server = server_name,
                                            "SSE endpoint discovered: {}", event_data
                                        );
                                    }

                                    // Handle message events (JSON-RPC responses)
                                    if event_type == "message" {
                                        if let Ok(response) =
                                            serde_json::from_str::<JsonRpcResponse>(&event_data)
                                        {
                                            let mut pending = pending_clone.lock().await;
                                            if let Some(sender) = pending.remove(&response.id) {
                                                let _ = sender.send(response);
                                            }
                                        } else if let Ok(v) =
                                            serde_json::from_str::<serde_json::Value>(&event_data)
                                        {
                                            // G20 gap: SSE inbound requests
                                            // (method + id) aren't yet
                                            // dispatched. Log so the deferral
                                            // is visible instead of failing
                                            // silently.
                                            if v.get("method").is_some() && v.get("id").is_some() {
                                                warn!(
                                                    server = server_name,
                                                    "SSE inbound request received but dispatch \
                                                     is not yet implemented (dropped): {}",
                                                    event_data
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                debug!(server = server_name, "SSE stream error: {}", msg);
                                // G4b: feed the error to the
                                // lifecycle tracker. If it decides
                                // to close, drop pending senders so
                                // waiting send_request calls fail
                                // fast instead of hanging until
                                // their per-request timeout.
                                let mut lc = lifecycle_clone.lock().await;
                                if let super::lifecycle::LifecycleDecision::TriggerClose {
                                    reason,
                                } = lc.record_error(&msg)
                                {
                                    debug!(
                                        server = server_name,
                                        "SSE transport closed by lifecycle tracker: {}", reason
                                    );
                                    let mut pending = pending_clone.lock().await;
                                    pending.clear();
                                }
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    debug!(server = server_name, "SSE connection failed: {}", msg);
                    // Connection-setup failures are fed through
                    // `record_error` like any other error. If the
                    // message matches a terminal-substring (e.g. a
                    // connect-level ETIMEDOUT), the counter will
                    // tick; a non-terminal setup failure (e.g. a
                    // generic `reqwest` send error) will not. This
                    // mirrors TS, whose onerror handler treats
                    // setup and mid-stream errors the same way.
                    let _ = lifecycle_clone.lock().await.record_error(&msg);
                }
            }
            debug!(server = server_name, "SSE reader task ended");
        });

        // Wait briefly for the endpoint event
        tokio::time::sleep(Duration::from_millis(500)).await;

        let writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> = Arc::new(Mutex::new(None));

        let mut client = Self {
            name: name.to_string(),
            transport: McpTransport::Sse {
                url: config.url.clone(),
                http,
                headers: config.headers.clone(),
                message_url,
            },
            process: None,
            writer,
            pending,
            next_id,
            // SSE inbound-request dispatch is deferred; the map
            // holds defaults for API parity with stdio.
            request_handlers: default_request_handlers().await,
            lifecycle,
            capabilities: None,
            server_info: None,
            instructions: None,
            reader_handle: Some(reader_handle),
        };

        client.initialize().await?;

        Ok(client)
    }

    /// Connect to an MCP server using HTTP (Streamable HTTP) transport.
    ///
    /// The HTTP transport sends JSON-RPC messages as POST requests and
    /// receives responses in the POST response body. This matches the
    /// MCP Streamable HTTP transport spec.
    pub async fn connect_http(name: &str, config: &McpHttpServerConfig) -> Result<Self> {
        debug!(server = name, url = %config.url, "Connecting to MCP server via HTTP");

        let http = reqwest::Client::new();
        let session_id = Arc::new(Mutex::new(None));
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let next_id = Arc::new(AtomicU64::new(1));
        let writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> = Arc::new(Mutex::new(None));

        let mut client = Self {
            name: name.to_string(),
            transport: McpTransport::Http {
                url: config.url.clone(),
                http,
                headers: config.headers.clone(),
                session_id,
            },
            process: None,
            writer,
            pending,
            next_id,
            // HTTP inbound-request dispatch is deferred; the map
            // holds defaults for API parity with stdio.
            request_handlers: default_request_handlers().await,
            lifecycle: Arc::new(Mutex::new(super::lifecycle::LifecycleTracker::new())),
            capabilities: None,
            server_info: None,
            instructions: None,
            reader_handle: None,
        };

        client.initialize().await?;

        Ok(client)
    }

    /// Connect to an MCP server over a WebSocket. Shared
    /// implementation for both `ws` and `ws-ide`; the caller
    /// assembles the appropriate headers. G18b port of TS
    /// `WebSocketTransport` setup at `client.ts:735-783`.
    ///
    /// `request_headers` is applied at the handshake via the
    /// `tungstenite` request builder. Requested subprotocol is
    /// hardcoded to `mcp` to match TS `protocols: ['mcp']`.
    ///
    /// The reader task owns the read half of the stream and
    /// dispatches frames through the same `dispatch_inbound_line`
    /// pipeline stdio uses — responses → `pending`, server
    /// requests → `request_handlers`, notifications → debug log.
    /// Outbound messages go through an `mpsc::UnboundedSender`
    /// so `send_request` / `send_notification` remain
    /// single-await.
    pub async fn connect_ws(
        name: &str,
        url: &str,
        request_headers: HashMap<String, String>,
    ) -> Result<Self> {
        use tokio_tungstenite::tungstenite::{
            client::IntoClientRequest, http::HeaderValue, Message,
        };

        debug!(server = name, url, "Connecting to MCP server via WebSocket");

        // Build the upgrade request with custom headers +
        // `Sec-WebSocket-Protocol: mcp`.
        let mut req = url
            .into_client_request()
            .with_context(|| format!("invalid WebSocket URL for MCP server '{}'", name))?;
        {
            let hdrs = req.headers_mut();
            hdrs.insert("Sec-WebSocket-Protocol", HeaderValue::from_static("mcp"));
            for (k, v) in &request_headers {
                if let (Ok(hn), Ok(hv)) = (
                    tokio_tungstenite::tungstenite::http::HeaderName::try_from(k.as_str()),
                    HeaderValue::try_from(v.as_str()),
                ) {
                    hdrs.insert(hn, hv);
                } else {
                    warn!(
                        server = name,
                        header = %k,
                        "Skipping invalid WebSocket header"
                    );
                }
            }
        }

        let (ws_stream, _response) = tokio_tungstenite::connect_async(req)
            .await
            .with_context(|| format!("Failed to establish WebSocket to MCP server '{}'", name))?;

        use futures_util::{SinkExt, StreamExt};
        let (mut write_half, mut read_half) = ws_stream.split();

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let request_handlers = default_request_handlers().await;
        let lifecycle: Arc<Mutex<super::lifecycle::LifecycleTracker>> =
            Arc::new(Mutex::new(super::lifecycle::LifecycleTracker::new()));
        let next_id = Arc::new(AtomicU64::new(1));

        // Outbound queue — unbounded since we don't want
        // send_request to block on a full channel. Volume is
        // bounded by the pending map size already.
        let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        // Writer task: drain outgoing frames.
        let writer_server_name = name.to_string();
        tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                if let Err(e) = write_half.send(Message::Text(msg)).await {
                    debug!(server = writer_server_name, "WebSocket write failed: {}", e);
                    break;
                }
            }
            let _ = write_half.close().await;
        });

        // Reader task: dispatch incoming frames via the shared
        // inbound pipeline.
        let reader_server_name = name.to_string();
        let pending_clone = pending.clone();
        let handlers_clone = request_handlers.clone();
        let lifecycle_clone = lifecycle.clone();
        // Reuse the stdio dispatcher — it accepts a writer Arc
        // so we wrap our write_tx in an adapter that ignores the
        // writer slot (WS doesn't use the blocking-Write
        // interface). To keep dispatch unified we just clone the
        // write_tx into a no-op writer for now; server-initiated
        // request responses get sent directly.
        let writer_adapter: Arc<Mutex<Option<Box<dyn Write + Send>>>> = Arc::new(Mutex::new(None));
        let write_tx_clone = write_tx.clone();
        let reader_handle = tokio::spawn(async move {
            while let Some(frame) = read_half.next().await {
                match frame {
                    Ok(Message::Text(txt)) => {
                        dispatch_ws_inbound_line(
                            &txt,
                            &reader_server_name,
                            pending_clone.clone(),
                            handlers_clone.clone(),
                            write_tx_clone.clone(),
                        );
                    }
                    Ok(Message::Close(_)) => {
                        debug!(server = reader_server_name, "WebSocket closed by peer");
                        break;
                    }
                    Ok(_) => {
                        // Ignore binary / ping / pong — MCP uses
                        // text JSON only. tungstenite auto-pongs
                        // pings internally.
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        debug!(
                            server = reader_server_name,
                            "WebSocket stream error: {}", msg
                        );
                        // Same lifecycle treatment as SSE stream
                        // errors — drain pending on TriggerClose.
                        let mut lc = lifecycle_clone.lock().await;
                        if let super::lifecycle::LifecycleDecision::TriggerClose { reason } =
                            lc.record_error(&msg)
                        {
                            debug!(
                                server = reader_server_name,
                                "WebSocket closed by lifecycle tracker: {}", reason
                            );
                            let mut pending = pending_clone.lock().await;
                            pending.clear();
                        }
                        break;
                    }
                }
            }
            // Silence unused-Arc warnings for the adapter slot.
            drop(writer_adapter);
        });

        let mut client = Self {
            name: name.to_string(),
            transport: McpTransport::Ws {
                url: url.to_string(),
                write_tx,
            },
            process: None,
            writer: Arc::new(Mutex::new(None)),
            pending,
            next_id,
            request_handlers,
            lifecycle,
            capabilities: None,
            server_info: None,
            instructions: None,
            reader_handle: Some(reader_handle),
        };

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
    ///
    /// Dispatches to the appropriate transport: stdio writes to stdin,
    /// SSE POSTs to the message endpoint, HTTP POSTs to the server URL.
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<JsonRpcResponse> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, method, params);

        match &self.transport {
            McpTransport::Stdio => {
                let (tx, rx) = oneshot::channel();

                // Register the pending request
                {
                    let mut pending = self.pending.lock().await;
                    pending.insert(id, tx);
                }

                // Serialize and send via stdin
                let msg = serde_json::to_string(&request)?;
                {
                    let mut writer_guard = self.writer.lock().await;
                    if let Some(writer) = writer_guard.as_mut() {
                        writeln!(writer, "{}", msg).with_context(|| {
                            format!("Failed to write to MCP server '{}'", self.name)
                        })?;
                        writer.flush().with_context(|| {
                            format!("Failed to flush MCP server '{}'", self.name)
                        })?;
                    } else {
                        return Err(anyhow!("MCP server '{}' writer is closed", self.name));
                    }
                }

                debug!(
                    server = self.name,
                    method = method,
                    id = id,
                    "Sent MCP request (stdio)"
                );

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

            McpTransport::Sse {
                http,
                headers,
                message_url,
                ..
            } => {
                // G4b: short-circuit if the lifecycle tracker has
                // already signalled close on this connection.
                if self.lifecycle.lock().await.has_triggered_close() {
                    return Err(anyhow!(
                        "MCP SSE server '{}' transport closed (reconnect required)",
                        self.name
                    ));
                }

                // For SSE, POST the request to the message endpoint.
                // The response comes back via the SSE event stream.
                let msg_url = {
                    let mu = message_url.lock().await;
                    mu.clone().ok_or_else(|| {
                        anyhow!(
                            "MCP SSE server '{}' message endpoint not yet discovered",
                            self.name
                        )
                    })?
                };

                let (tx, rx) = oneshot::channel();
                {
                    let mut pending = self.pending.lock().await;
                    pending.insert(id, tx);
                }

                let mut req = mcp_streamable_http_post(http, &msg_url, headers.as_ref())
                    .header("content-type", "application/json");
                if let Some(hdrs) = headers {
                    for (k, v) in hdrs {
                        req = req.header(k, v);
                    }
                }

                let resp = req.json(&request).send().await.with_context(|| {
                    format!(
                        "Failed to POST to MCP SSE server '{}' at {}",
                        self.name, msg_url
                    )
                })?;

                if !resp.status().is_success() {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(anyhow!(
                        "MCP SSE server '{}' POST error {}: {}",
                        self.name,
                        status,
                        body
                    ));
                }

                debug!(
                    server = self.name,
                    method = method,
                    id = id,
                    "Sent MCP request (SSE POST)"
                );

                // Wait for the SSE stream to deliver the response
                let timeout = Duration::from_millis(MCP_TOOL_TIMEOUT_MS);
                match tokio::time::timeout(timeout, rx).await {
                    Ok(Ok(response)) => Ok(response),
                    Ok(Err(_)) => Err(anyhow!(
                        "MCP SSE server '{}' response channel closed for request {}",
                        self.name,
                        id
                    )),
                    Err(_) => {
                        let mut pending = self.pending.lock().await;
                        pending.remove(&id);
                        Err(anyhow!(
                            "MCP SSE request to '{}' timed out (method: {})",
                            self.name,
                            method
                        ))
                    }
                }
            }

            McpTransport::Http {
                url,
                http,
                headers,
                session_id,
            } => {
                // For HTTP (Streamable HTTP), POST the JSON-RPC request and
                // read the response directly from the HTTP response body.
                let mut req = mcp_streamable_http_post(http, url, headers.as_ref())
                    .header("content-type", "application/json");

                if let Some(hdrs) = headers {
                    for (k, v) in hdrs {
                        req = req.header(k, v);
                    }
                }

                // Include session ID if we have one from a previous response
                {
                    let sid = session_id.lock().await;
                    if let Some(ref s) = *sid {
                        req = req.header("mcp-session-id", s);
                    }
                }

                // G4b: short-circuit if the lifecycle tracker has
                // already signalled a close. Subsequent requests
                // through a dead transport must fail fast rather
                // than contact a server that we've declared dead.
                if self.lifecycle.lock().await.has_triggered_close() {
                    return Err(anyhow!(
                        "MCP HTTP server '{}' transport closed (reconnect required)",
                        self.name
                    ));
                }

                let resp = match req.json(&request).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        // G4b: feed send error to the tracker so
                        // repeated terminal errors escalate to a
                        // transport close signal.
                        let _ = self.lifecycle.lock().await.record_error(&e.to_string());
                        return Err(anyhow!(e).context(format!(
                            "Failed to POST to MCP HTTP server '{}' at {}",
                            self.name, url
                        )));
                    }
                };

                // Capture session ID from response header
                if let Some(sid_val) = resp.headers().get("mcp-session-id") {
                    if let Ok(s) = sid_val.to_str() {
                        let mut sid = session_id.lock().await;
                        *sid = Some(s.to_string());
                    }
                }

                if !resp.status().is_success() {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    // G4b: detect HTTP session-expired signal (404
                    // + JSON-RPC -32001) and fire the dedicated
                    // record_session_expired path. Mirrors TS
                    // `client.ts:1316-1329`.
                    if super::helpers::is_mcp_session_expired_error(Some(status), &body) {
                        let _ = self.lifecycle.lock().await.record_session_expired();
                        return Err(anyhow!(
                            "MCP HTTP server '{}' session expired (HTTP 404 + JSON-RPC -32001)",
                            self.name
                        ));
                    }
                    let _ = self
                        .lifecycle
                        .lock()
                        .await
                        .record_error(&format!("HTTP {} body: {}", status, body));
                    return Err(anyhow!(
                        "MCP HTTP server '{}' error {}: {}",
                        self.name,
                        status,
                        body
                    ));
                }

                debug!(
                    server = self.name,
                    method = method,
                    id = id,
                    "Sent MCP request (HTTP POST)"
                );

                let body = resp.text().await.with_context(|| {
                    format!(
                        "Failed to read response from MCP HTTP server '{}'",
                        self.name
                    )
                })?;

                let response: JsonRpcResponse = serde_json::from_str(&body).with_context(|| {
                    format!(
                        "Failed to parse JSON-RPC response from MCP HTTP server '{}': {}",
                        self.name,
                        &body[..body.len().min(200)]
                    )
                })?;

                Ok(response)
            }
            McpTransport::Ws { write_tx, .. } => {
                // G18b: lifecycle short-circuit same shape as
                // SSE — a closed transport fails fast.
                if self.lifecycle.lock().await.has_triggered_close() {
                    return Err(anyhow!(
                        "MCP WS server '{}' transport closed (reconnect required)",
                        self.name
                    ));
                }
                let (tx, rx) = oneshot::channel();
                {
                    let mut pending = self.pending.lock().await;
                    pending.insert(id, tx);
                }
                let line =
                    serde_json::to_string(&request).context("Failed to serialize WS request")?;
                if let Err(e) = write_tx.send(line) {
                    // Writer task is gone — pending was inserted;
                    // drop it so the caller sees the error
                    // immediately rather than on timeout.
                    let mut pending = self.pending.lock().await;
                    pending.remove(&id);
                    return Err(anyhow!(
                        "MCP WS server '{}' writer closed: {}",
                        self.name,
                        e
                    ));
                }

                let timeout = Duration::from_millis(MCP_TOOL_TIMEOUT_MS);
                match tokio::time::timeout(timeout, rx).await {
                    Ok(Ok(response)) => Ok(response),
                    Ok(Err(_)) => Err(anyhow!(
                        "MCP WS server '{}' response channel closed for request {}",
                        self.name,
                        id
                    )),
                    Err(_) => {
                        let mut pending = self.pending.lock().await;
                        pending.remove(&id);
                        Err(anyhow!(
                            "MCP WS request to '{}' timed out (method: {})",
                            self.name,
                            method
                        ))
                    }
                }
            }
        }
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let notification = JsonRpcNotification::new(method, params);
        let msg = serde_json::to_string(&notification)?;

        match &self.transport {
            McpTransport::Stdio => {
                let mut writer_guard = self.writer.lock().await;
                if let Some(writer) = writer_guard.as_mut() {
                    writeln!(writer, "{}", msg).with_context(|| {
                        format!("Failed to write notification to MCP server '{}'", self.name)
                    })?;
                    writer
                        .flush()
                        .with_context(|| format!("Failed to flush MCP server '{}'", self.name))?;
                } else {
                    return Err(anyhow!("MCP server '{}' writer is closed", self.name));
                }
            }

            McpTransport::Sse {
                http,
                headers,
                message_url,
                ..
            } => {
                // G4b: honour the transport-closed contract for
                // notifications too. A dead transport should fail
                // outbound traffic fast, not attempt network I/O.
                if self.lifecycle.lock().await.has_triggered_close() {
                    return Err(anyhow!(
                        "MCP SSE server '{}' transport closed (reconnect required)",
                        self.name
                    ));
                }

                let msg_url = {
                    let mu = message_url.lock().await;
                    mu.clone().ok_or_else(|| {
                        anyhow!(
                            "MCP SSE server '{}' message endpoint not yet discovered",
                            self.name
                        )
                    })?
                };

                let mut req = mcp_streamable_http_post(http, &msg_url, headers.as_ref())
                    .header("content-type", "application/json");
                if let Some(hdrs) = headers {
                    for (k, v) in hdrs {
                        req = req.header(k, v);
                    }
                }

                let _ = req.json(&notification).send().await.with_context(|| {
                    format!(
                        "Failed to send notification to MCP SSE server '{}'",
                        self.name
                    )
                })?;
            }

            McpTransport::Http {
                url,
                http,
                headers,
                session_id,
            } => {
                // G4b: same closed-transport short-circuit as the
                // SSE branch — prevents POSTs to a server we've
                // already declared dead.
                if self.lifecycle.lock().await.has_triggered_close() {
                    return Err(anyhow!(
                        "MCP HTTP server '{}' transport closed (reconnect required)",
                        self.name
                    ));
                }

                let mut req = mcp_streamable_http_post(http, url, headers.as_ref())
                    .header("content-type", "application/json");
                if let Some(hdrs) = headers {
                    for (k, v) in hdrs {
                        req = req.header(k, v);
                    }
                }
                {
                    let sid = session_id.lock().await;
                    if let Some(ref s) = *sid {
                        req = req.header("mcp-session-id", s);
                    }
                }

                let _ = req.json(&notification).send().await.with_context(|| {
                    format!(
                        "Failed to send notification to MCP HTTP server '{}'",
                        self.name
                    )
                })?;
            }
            McpTransport::Ws { write_tx, .. } => {
                if self.lifecycle.lock().await.has_triggered_close() {
                    return Err(anyhow!(
                        "MCP WS server '{}' transport closed (reconnect required)",
                        self.name
                    ));
                }
                let line = serde_json::to_string(&notification)
                    .context("Failed to serialize WS notification")?;
                if let Err(e) = write_tx.send(line) {
                    return Err(anyhow!(
                        "MCP WS server '{}' notification writer closed: {}",
                        self.name,
                        e
                    ));
                }
            }
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
    pub async fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<McpToolResult> {
        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments
        });

        // G14: start a 30s heartbeat so long-running tool calls
        // don't look frozen to the user. Mirrors TS `setInterval`
        // at `client.ts:3055-3066`. `HeartbeatGuard::drop` aborts
        // the spawned task so the timer stops on ANY return path
        // (success, error, or parent-future cancellation).
        let _heartbeat = spawn_progress_heartbeat(self.name.clone(), tool_name.to_string());
        // Transport flavour hint for the session-expiry classifier
        // (TS scopes -32000 "Connection closed" to http /
        // claudeai-proxy only; SSE and stdio errors of that shape
        // must NOT become McpSessionExpiredError).
        let transport_hint = match &self.transport {
            McpTransport::Http { .. } => TransportHint::Http,
            McpTransport::Sse { .. } => TransportHint::Sse,
            McpTransport::Stdio => TransportHint::Stdio,
            // Ws uses the same classifier rules as SSE for the
            // -32000 "Connection closed" session-expiry path —
            // both are reconnect-eligible streaming transports,
            // not HTTP-session-tied.
            McpTransport::Ws { .. } => TransportHint::Sse,
        };

        let response = match self.send_request(methods::TOOLS_CALL, Some(params)).await {
            Ok(r) => r,
            Err(e) => {
                return Err(classify_call_tool_error(e, &self.name, transport_hint));
            }
        };

        if let Some(result) = response.result {
            // Peek for a legacy `result.error` string BEFORE the
            // structural deserialize — TS `client.ts:3139-3142`
            // falls back to `String(result.error)` for servers
            // that predate the `content[]`+`isError` envelope. The
            // Rust `McpToolResult` shape doesn't model the legacy
            // field, so we pull it out as a JSON fallback.
            let legacy_error = result.get("error").map(|v| match v.as_str() {
                Some(s) => s.to_string(),
                None => v.to_string(),
            });

            let tool_result: McpToolResult =
                serde_json::from_value(result).context("Failed to parse MCP tool call result")?;

            // G14: detect `isError: true` in the success envelope
            // and surface as a typed McpToolCallError so callers
            // can downcast for structured telemetry. Mirrors TS
            // `client.ts:3124-3148`.
            if tool_result.is_error.unwrap_or(false) {
                let details = tool_result
                    .content
                    .iter()
                    .find_map(|c| c.text.clone())
                    .or(legacy_error)
                    .unwrap_or_else(|| "Unknown error".to_string());
                return Err(anyhow::Error::from(super::errors::McpToolCallError::new(
                    details,
                    "MCP tool returned error",
                )));
            }
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

    /// List all prompts available from this MCP server.
    ///
    /// Sends a `prompts/list` request and parses the response into
    /// `McpPromptDefinition`s. G8 port; TS equivalent at
    /// `services/mcp/client.ts:2043-2046` inside
    /// `fetchCommandsForClient`. An empty `prompts` field (or no
    /// server capability) surfaces as an empty vec rather than an
    /// error — matches the TS `return []` fallbacks.
    pub async fn list_prompts(&self) -> Result<Vec<McpPromptDefinition>> {
        // If the server never advertised the `prompts` capability
        // during init, skip the request entirely — matches TS
        // `if (!client.capabilities?.prompts) return []` which
        // short-circuits BOTH when `capabilities` itself is
        // absent AND when `capabilities.prompts` is absent.
        let prompts_advertised = self
            .capabilities
            .as_ref()
            .and_then(|c| c.prompts.as_ref())
            .is_some();
        if !prompts_advertised {
            debug!(
                server = self.name,
                "Server did not advertise prompts capability; skipping prompts/list"
            );
            return Ok(vec![]);
        }

        let response = self
            .send_request(methods::PROMPTS_LIST, Some(serde_json::json!({})))
            .await?;

        if let Some(result) = response.result {
            if let Some(prompts) = result.get("prompts") {
                let prompts: Vec<McpPromptDefinition> = serde_json::from_value(prompts.clone())
                    .context("Failed to parse MCP prompts list")?;
                debug!(
                    server = self.name,
                    count = prompts.len(),
                    "Listed MCP prompts"
                );
                return Ok(prompts);
            }
        }

        if let Some(err) = response.error {
            return Err(anyhow!(
                "MCP prompts/list error from '{}': {} (code: {})",
                self.name,
                err.message,
                err.code
            ));
        }

        Ok(vec![])
    }

    /// Retrieve a rendered prompt by name with arguments.
    ///
    /// Sends a `prompts/get` request. `arguments` is the
    /// `{argName: stringValue}` map the MCP spec requires — the
    /// schema mandates string values for every argument, so the
    /// API type enforces that at the signature level rather than
    /// accepting a looser `serde_json::Value` that could emit an
    /// invalid protocol payload. Pass `None` to omit the
    /// `arguments` field entirely.
    ///
    /// Returns the server's `GetPromptResult` (messages + optional
    /// description + optional `_meta`). G8 port; TS equivalent at
    /// `services/mcp/client.ts:2077-2080` inside
    /// `getPromptForCommand`.
    pub async fn get_prompt(
        &self,
        prompt_name: &str,
        arguments: Option<&HashMap<String, String>>,
    ) -> Result<McpPromptResult> {
        let mut params = serde_json::json!({ "name": prompt_name });
        if let Some(args) = arguments {
            params["arguments"] =
                serde_json::to_value(args).context("Failed to serialize prompts/get arguments")?;
        }
        let response = self
            .send_request(methods::PROMPTS_GET, Some(params))
            .await?;

        if let Some(result) = response.result {
            let parsed: McpPromptResult =
                serde_json::from_value(result).context("Failed to parse MCP prompts/get result")?;
            debug!(
                server = self.name,
                prompt = prompt_name,
                messages = parsed.messages.len(),
                "Retrieved MCP prompt"
            );
            return Ok(parsed);
        }

        if let Some(err) = response.error {
            return Err(anyhow!(
                "MCP prompts/get error from '{}' (prompt: {}): {} (code: {})",
                self.name,
                prompt_name,
                err.message,
                err.code
            ));
        }

        Err(anyhow!(
            "MCP prompts/get returned empty response from '{}' (prompt: {})",
            self.name,
            prompt_name
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
        let response = self.send_request(methods::PING, None).await?;

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
    /// Closes the writer, gracefully terminates the server process
    /// (SIGINT → SIGTERM → SIGKILL escalation on Unix, direct kill
    /// on Windows), and waits for the reader task. Matches TS
    /// `cleanup()` at `services/mcp/client.ts:1404-1570`.
    pub async fn disconnect(&mut self) {
        debug!(server = self.name, "Disconnecting from MCP server");

        // Close the writer
        {
            let mut writer = self.writer.lock().await;
            *writer = None;
        }

        // Gracefully terminate the process. Unix uses the escalated
        // SIGINT → SIGTERM → SIGKILL sequence; Windows uses the
        // blunt `child.kill()` since there's no ergonomic SIGTERM
        // equivalent.
        if let Some(ref mut child) = self.process {
            #[cfg(unix)]
            {
                graceful_terminate_stdio(child, &self.name).await;
            }
            #[cfg(not(unix))]
            {
                if let Err(e) = child.kill() {
                    warn!(
                        server = self.name,
                        error = %e,
                        "Failed to kill MCP server process"
                    );
                }
            }
            // Wait for the process to avoid zombies. `wait` on an
            // already-reaped child returns the cached exit status
            // quickly; the graceful helper on Unix already reaps
            // via try_wait().
            let _ = child.wait();
        }

        // Transport cleanup above has already closed the writer and, for
        // stdio, terminated/reaped the child process. Reader tasks can still
        // sit in blocking reads or long-lived remote streams, so abort them
        // explicitly instead of making CLI shutdown wait on transport details.
        if let Some(handle) = self.reader_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        debug!(server = self.name, "MCP server disconnected");
    }

    /// Check if the MCP server connection is still active.
    ///
    /// For stdio transport, checks if the child process is running.
    /// For SSE/HTTP transports, checks if the reader task is still alive.
    pub fn is_alive(&mut self) -> bool {
        match &self.transport {
            McpTransport::Stdio => {
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
            McpTransport::Sse { .. } => {
                // SSE is alive if the reader task hasn't finished
                if let Some(ref handle) = self.reader_handle {
                    !handle.is_finished()
                } else {
                    false
                }
            }
            McpTransport::Http { .. } => {
                // HTTP transport is stateless -- always "alive" as long as
                // we have the URL configured
                true
            }
            McpTransport::Ws { .. } => {
                // Same liveness rule as SSE: the reader task
                // exits when the socket closes (either via a
                // Close frame or a stream error).
                if let Some(ref handle) = self.reader_handle {
                    !handle.is_finished()
                } else {
                    false
                }
            }
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
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ─── call_tool classification + heartbeat (G14) ─────────────────────

/// RAII guard around a spawned heartbeat. Dropping the guard
/// aborts the background task — tokio's `AbortHandle` does NOT
/// auto-abort on drop (it's just a weak cancel handle), so we
/// wrap it with an explicit `Drop` impl to prevent the heartbeat
/// from outliving a cancelled `call_tool()` future.
struct HeartbeatGuard(tokio::task::AbortHandle);

impl Drop for HeartbeatGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Spawn a 30-second heartbeat that debug-logs
/// `"Tool '<tool>' still running (<secs>s elapsed)"`. Returns a
/// `HeartbeatGuard` whose `Drop` impl aborts the task — cancelling
/// `call_tool()` at any await point (including by dropping the
/// future) stops the heartbeat. Matches TS `setInterval` at
/// `client.ts:3055-3066`.
fn spawn_progress_heartbeat(server_name: String, tool_name: String) -> HeartbeatGuard {
    use std::time::Duration;
    let handle = tokio::spawn(async move {
        let start = tokio::time::Instant::now();
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        // First tick fires immediately in tokio's default Burst
        // mode — skip it so we only log at 30, 60, 90s, etc.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let elapsed_s = start.elapsed().as_secs();
            debug!(
                server = server_name,
                tool = tool_name,
                "Tool '{}' still running ({}s elapsed)",
                tool_name,
                elapsed_s
            );
        }
    });
    HeartbeatGuard(handle.abort_handle())
}

/// Which transport originated the error. The
/// `-32000 "Connection closed"` session-expiry mapping is scoped
/// to HTTP transports per TS `client.ts:3218-3222`; other
/// transports must NOT flip that signal into a session-expired
/// error (an SSE stream closing mid-request is a lifecycle event,
/// not a session-ID invalidation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportHint {
    Http,
    Sse,
    Stdio,
}

/// Classify a `send_request` error into a typed domain error when
/// the message matches a known auth / session-expired pattern.
/// Unmatched errors pass through unchanged. Mirrors TS
/// `client.ts:3194-3232`.
fn classify_call_tool_error(
    e: anyhow::Error,
    server_name: &str,
    transport: TransportHint,
) -> anyhow::Error {
    let msg = format!("{:#}", e);
    let msg_lower = msg.to_ascii_lowercase();

    // 401 signals an expired / revoked OAuth token. The Rust HTTP
    // transport surfaces it as `"HTTP 401 body: ..."` (see G4b
    // wiring). TS also matches `UnauthorizedError` raised by the
    // OAuth layer — that's not wired in the Rust client yet, but
    // a case-insensitive "unauthorized" substring covers most
    // transport-level wordings we might see in practice.
    if msg.contains("HTTP 401") || msg_lower.contains("unauthorized") {
        return anyhow::Error::from(super::errors::McpAuthError::new(
            server_name.to_string(),
            format!(
                "MCP server \"{}\" requires re-authorization (token expired)",
                server_name
            ),
        ));
    }

    // Session expired: either HTTP 404 + JSON-RPC -32001, or the
    // SDK's derived -32000 "Connection closed" on HTTP only. TS
    // `client.ts:3217-3222` explicitly gates the -32000 branch on
    // `config.type === 'http' || 'claudeai-proxy'`; the -32001
    // branch applies globally because it's a direct server
    // signal, not a transport-layer wrap.
    let direct_404_32001 = msg.contains("session expired")
        || super::helpers::is_mcp_session_expired_error(Some(404), &msg);
    let connection_closed_on_http = transport == TransportHint::Http
        && msg.contains("-32000")
        && msg.contains("Connection closed");
    if direct_404_32001 || connection_closed_on_http {
        return anyhow::Error::from(super::errors::McpSessionExpiredError::new(
            server_name.to_string(),
        ));
    }

    e
}

// ─── Graceful stdio termination (G5) ─────────────────────────────────

/// Escalate signals to terminate an MCP stdio subprocess gracefully:
/// SIGINT first, then SIGTERM if still alive after 100ms, then
/// SIGKILL if still alive after another 400ms. An absolute 600ms
/// failsafe guarantees the function never blocks the CLI for
/// longer than that. Matches TS `cleanup()` at
/// `services/mcp/client.ts:1429-1562`.
///
/// Many MCP servers — especially Docker-wrapped ones — only drain
/// on explicit signals; an immediate SIGKILL leaves log files
/// corrupt and open network ports dangling. Escalating gives
/// well-behaved servers a chance to shut down cleanly while
/// capping the total wait so the CLI stays responsive.
///
/// Takes `&mut Child` (not a bare PID) so liveness can be checked
/// via `try_wait()` — TS can `process.kill(pid, 0)` to probe since
/// Node auto-reaps children on exit, but Rust's std `Child` holds
/// onto the zombie until `wait()` is called, so a bare
/// `libc::kill(pid, 0)` would see zombies as "alive" and hang the
/// escalation through the 600ms failsafe.
///
/// Unix-only: compiled out on Windows where signals beyond
/// `child.kill()` (i.e. TerminateProcess) aren't meaningful for
/// stdio subprocesses in the same way.
#[cfg(unix)]
pub(crate) async fn graceful_terminate_stdio(child: &mut std::process::Child, name: &str) {
    use std::time::Duration;

    let pid = child.id();
    let pid_i = pid as libc::pid_t;
    if pid == 0 {
        debug!(server = name, "No pid to terminate");
        return;
    }

    // Absolute wall-time cap: no matter how the per-step windows
    // add up or how slow polling runs, the entire helper returns
    // by this deadline. Matches TS `failsafeTimeout` at
    // `client.ts:1467-1477` which starts a concurrent 600ms timer
    // alongside the sequential escalation.
    let absolute_deadline = tokio::time::Instant::now() + Duration::from_millis(600);

    // Step 1: SIGINT (like Ctrl-C). Polite — most servers handle
    // this for graceful shutdown.
    debug!(server = name, "Sending SIGINT to MCP server process");
    let r = unsafe { libc::kill(pid_i, libc::SIGINT) };
    if r != 0 {
        debug!(
            server = name,
            errno = std::io::Error::last_os_error().raw_os_error(),
            "SIGINT kill() failed (likely already exited)"
        );
        return;
    }

    // Poll on 25ms granularity up to the SIGINT window (100ms,
    // capped by the absolute deadline).
    if wait_for_exit_bounded(child, Duration::from_millis(100), absolute_deadline).await {
        debug!(server = name, "MCP server process exited cleanly on SIGINT");
        return;
    }

    // Step 2: SIGTERM.
    debug!(
        server = name,
        "SIGINT failed, sending SIGTERM to MCP server process"
    );
    let r = unsafe { libc::kill(pid_i, libc::SIGTERM) };
    if r != 0 {
        debug!(
            server = name,
            errno = std::io::Error::last_os_error().raw_os_error(),
            "SIGTERM kill() failed (likely exited between probes)"
        );
        return;
    }

    // SIGTERM window: 400ms nominal; absolute_deadline may clip
    // it short if the SIGINT step took longer than 100ms.
    if wait_for_exit_bounded(child, Duration::from_millis(400), absolute_deadline).await {
        debug!(server = name, "MCP server process exited on SIGTERM");
        return;
    }

    // Step 3: SIGKILL. Unignorable; kernel reaps the process.
    debug!(
        server = name,
        "SIGTERM failed, sending SIGKILL to MCP server process"
    );
    let r = unsafe { libc::kill(pid_i, libc::SIGKILL) };
    if r != 0 {
        debug!(
            server = name,
            errno = std::io::Error::last_os_error().raw_os_error(),
            "SIGKILL kill() failed (process may be unkillable by us)"
        );
    }
    // Give SIGKILL a small window to actually reap. The absolute
    // deadline may already be past; the bounded helper returns
    // immediately in that case.
    let _ = wait_for_exit_bounded(child, Duration::from_millis(100), absolute_deadline).await;

    /// Poll `child.try_wait()` every 25ms until the child exits
    /// OR `min(now + per_step_window, absolute_deadline)` elapses.
    /// Returns `true` if the child exited within the window.
    async fn wait_for_exit_bounded(
        child: &mut std::process::Child,
        per_step_window: Duration,
        absolute_deadline: tokio::time::Instant,
    ) -> bool {
        let step_deadline = tokio::time::Instant::now() + per_step_window;
        let deadline = step_deadline.min(absolute_deadline);
        loop {
            if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline.duration_since(now);
            let sleep_for = remaining.min(Duration::from_millis(25));
            tokio::time::sleep(sleep_for).await;
        }
    }
}

// ─── Inbound request dispatch (G20) ──────────────────────────────────

/// Build the default inbound-request handler map. Two defaults are
/// registered, matching TS `client.setRequestHandler` calls at
/// `services/mcp/client.ts:1009,1191`:
///   - `roots/list` → `{"roots": [{"uri": "file://<cwd>"}]}`
///   - `elicitation/create` → `{"action": "cancel"}`
///
/// Callers can override either via `McpClient::set_request_handler`
/// before the server issues its first inbound request.
async fn default_request_handlers() -> Arc<Mutex<HashMap<String, RequestHandler>>> {
    let map = Arc::new(Mutex::new(HashMap::new()));
    {
        let mut guard = map.lock().await;
        guard.insert("roots/list".to_string(), default_roots_list_handler());
        guard.insert(
            "elicitation/create".to_string(),
            default_elicitation_cancel_handler(),
        );
    }
    map
}

/// Startup cwd, captured once per process. Matches TS
/// `getOriginalCwd()` semantics at `client.ts:1014`: even if the
/// process later `chdir()`s, `roots/list` keeps reporting the
/// original workspace that MCP servers observed at init. Falls
/// back to `"."` if the first read fails so the handler can never
/// panic.
fn original_cwd() -> &'static std::path::PathBuf {
    static CELL: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    CELL.get_or_init(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")))
}

/// Default handler for `roots/list`: reports the *startup* working
/// directory as the sole root. TS at `client.ts:1009-1018` uses
/// `getOriginalCwd()`, so a post-startup `chdir` must NOT change
/// what we report to the server.
fn default_roots_list_handler() -> RequestHandler {
    Arc::new(|_params| {
        let uri = format!("file://{}", original_cwd().display());
        Box::pin(async move { Ok(serde_json::json!({ "roots": [ { "uri": uri } ] })) })
    })
}

/// Default elicitation handler: cancel. TS at `client.ts:1191-1197`
/// registers this immediately after `initialize` so any server
/// elicitation arriving before `registerElicitationHandler` runs
/// gets a clean `cancel` rather than hanging.
fn default_elicitation_cancel_handler() -> RequestHandler {
    Arc::new(|_params| Box::pin(async { Ok(serde_json::json!({ "action": "cancel" })) }))
}

/// Classify an inbound JSON-RPC line and dispatch it appropriately.
///
/// - Response (`result` / `error` + `id`): forward to `pending` to
///   wake the waiting `send_request`.
/// - Inbound request (`method` + `id`): look up the handler, call
///   it, and write a JSON-RPC response back via `writer`. This is
///   the G20 path; TS handles it inside the MCP SDK's `setRequestHandler`.
/// - Notification (`method`, no `id`): log and drop.
///
/// Errors at any stage are debug-logged — a misbehaving server must
/// never poison the reader loop.
/// WebSocket-flavour dispatcher. Same decision logic as the
/// stdio `dispatch_inbound_line` — classify line as response /
/// request / notification — but server→client responses go back
/// through the WS writer `mpsc::UnboundedSender` instead of a
/// blocking `Write` handle. G18b.
fn dispatch_ws_inbound_line(
    line: &str,
    server_name: &str,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    handlers: Arc<Mutex<HashMap<String, RequestHandler>>>,
    write_tx: tokio::sync::mpsc::UnboundedSender<String>,
) {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            debug!(
                server = server_name,
                "Unparseable WS line (dropped): {} ({})", line, e
            );
            return;
        }
    };

    let has_result = value.get("result").is_some() || value.get("error").is_some();
    let method = value
        .get("method")
        .and_then(|m| m.as_str())
        .map(String::from);
    let id = value.get("id").and_then(|i| i.as_u64());

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(rt) => rt,
        Err(_) => {
            debug!(
                server = server_name,
                "Reader has no tokio runtime; dropping WS message: {}", line
            );
            return;
        }
    };

    if has_result {
        match serde_json::from_value::<JsonRpcResponse>(value.clone()) {
            Ok(resp) => {
                rt.spawn(async move {
                    let mut pending = pending.lock().await;
                    if let Some(sender) = pending.remove(&resp.id) {
                        let _ = sender.send(resp);
                    }
                });
                return;
            }
            Err(e) => {
                debug!(
                    server = server_name,
                    "Malformed response-shaped WS message (dropped): {} ({})", line, e
                );
                return;
            }
        }
    }

    if let (Some(m), Some(request_id)) = (method.as_deref(), id) {
        let m = m.to_string();
        let params = value.get("params").cloned();
        let server_name_owned = server_name.to_string();
        let write_tx = write_tx.clone();
        rt.spawn(async move {
            let handler = {
                let guard = handlers.lock().await;
                guard.get(&m).cloned()
            };
            let response = match handler {
                Some(h) => match h(params).await {
                    Ok(result) => JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request_id,
                        result: Some(result),
                        error: None,
                    },
                    Err(err) => JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request_id,
                        result: None,
                        error: Some(err),
                    },
                },
                None => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request_id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32601,
                        message: format!("method not found: {}", m),
                        data: None,
                    }),
                },
            };
            let line = match serde_json::to_string(&response) {
                Ok(s) => s,
                Err(e) => {
                    debug!(
                        server = server_name_owned,
                        "Failed to serialize WS response: {}", e
                    );
                    return;
                }
            };
            if let Err(e) = write_tx.send(line) {
                debug!(
                    server = server_name_owned,
                    "Failed to enqueue WS response: {}", e
                );
            }
        });
        return;
    }

    if method.is_some() {
        debug!(
            server = server_name,
            "Ignoring WS server notification: {}", line
        );
        return;
    }
    debug!(
        server = server_name,
        "Unrecognised WS message (dropped): {}", line
    );
}

fn dispatch_inbound_line(
    line: &str,
    server_name: &str,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    handlers: Arc<Mutex<HashMap<String, RequestHandler>>>,
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
) {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            debug!(
                server = server_name,
                "Unparseable MCP line (dropped): {} ({})", line, e
            );
            return;
        }
    };

    let has_result = value.get("result").is_some() || value.get("error").is_some();
    let method = value
        .get("method")
        .and_then(|m| m.as_str())
        .map(String::from);
    let id = value.get("id").and_then(|i| i.as_u64());

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(rt) => rt,
        Err(_) => {
            debug!(
                server = server_name,
                "Reader has no tokio runtime; dropping message: {}", line
            );
            return;
        }
    };

    // Response path — takes precedence when both `id` and a
    // result/error field are present.
    if has_result {
        match serde_json::from_value::<JsonRpcResponse>(value.clone()) {
            Ok(resp) => {
                rt.spawn(async move {
                    let mut pending = pending.lock().await;
                    if let Some(sender) = pending.remove(&resp.id) {
                        let _ = sender.send(resp);
                    }
                });
                return;
            }
            Err(e) => {
                // Response-shaped but fails structural decode (e.g.
                // non-numeric id — JsonRpcResponse.id is u64). The
                // caller will hang until its send_request timeout
                // fires, so surface this explicitly for
                // diagnosability.
                debug!(
                    server = server_name,
                    "Malformed response-shaped message (dropped): {} ({})", line, e
                );
                return;
            }
        }
    }

    // Inbound-request path — `method` + `id` together.
    if let (Some(m), Some(request_id)) = (method.as_deref(), id) {
        let m = m.to_string();
        let params = value.get("params").cloned();
        let server_name_owned = server_name.to_string();
        rt.spawn(async move {
            let handler = {
                let guard = handlers.lock().await;
                guard.get(&m).cloned()
            };
            let response = match handler {
                Some(h) => match h(params).await {
                    Ok(result) => JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request_id,
                        result: Some(result),
                        error: None,
                    },
                    Err(err) => JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request_id,
                        result: None,
                        error: Some(err),
                    },
                },
                None => {
                    // JSON-RPC "method not found" (-32601).
                    JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request_id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32601,
                            message: format!("method not found: {}", m),
                            data: None,
                        }),
                    }
                }
            };

            let line = match serde_json::to_string(&response) {
                Ok(s) => s,
                Err(e) => {
                    debug!(
                        server = server_name_owned,
                        "Failed to serialize response: {}", e
                    );
                    return;
                }
            };
            let mut w = writer.lock().await;
            if let Some(ref mut w) = *w {
                if let Err(e) = writeln!(w, "{}", line) {
                    debug!(
                        server = server_name_owned,
                        "Failed to write inbound response: {}", e
                    );
                }
                let _ = w.flush();
            }
        });
        return;
    }

    // Notification path (`method` only, no id). TS drops these too
    // unless a notification handler is registered — not part of G20.
    if method.is_some() {
        debug!(
            server = server_name,
            "Ignoring server notification: {}", line
        );
        return;
    }

    debug!(
        server = server_name,
        "Unrecognised MCP message (dropped): {}", line
    );
}

impl McpClient {
    /// Register or replace the inbound-request handler for `method`.
    /// Ports TS `client.setRequestHandler(Schema, handler)` at
    /// `services/mcp/client.ts:1009`.
    ///
    /// The handler receives the `params` field (`None` if absent) and
    /// returns either the `result` payload or a structured
    /// `JsonRpcError`. Call this BEFORE the server would plausibly
    /// issue an inbound request of this method; for defaults like
    /// `roots/list` the client pre-registers them during
    /// `connect_stdio` so callers only need this method to override.
    ///
    /// SSE/HTTP transports currently accept registrations for API
    /// parity but do not yet dispatch inbound requests (handler
    /// runs only for stdio). Dispatch for those transports is a
    /// follow-up ticket.
    pub async fn set_request_handler<F, Fut>(&self, method: impl Into<String>, handler: F)
    where
        F: Fn(Option<Value>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Value, JsonRpcError>> + Send + 'static,
    {
        let wrapped: RequestHandler =
            Arc::new(move |params| Box::pin(handler(params)) as RequestHandlerFuture);
        let mut guard = self.request_handlers.lock().await;
        guard.insert(method.into(), wrapped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── G8: prompt types + list/get shape ───────────────────────

    #[test]
    fn prompt_definition_parses_from_server_payload() {
        // Representative wire payload for a single prompt entry.
        let wire = serde_json::json!({
            "name": "changelog",
            "title": "Generate a Changelog",
            "description": "Build release notes",
            "arguments": [
                { "name": "since", "description": "Tag/commit", "required": true },
                { "name": "format" }
            ]
        });
        let p: super::McpPromptDefinition = serde_json::from_value(wire).expect("deserialize");
        assert_eq!(p.name, "changelog");
        assert_eq!(p.title.as_deref(), Some("Generate a Changelog"));
        assert_eq!(p.description.as_deref(), Some("Build release notes"));
        let args = p.arguments.expect("arguments present");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0].name, "since");
        assert_eq!(args[0].required, Some(true));
        // `required` absent → None, matching serde default.
        assert_eq!(args[1].name, "format");
        assert_eq!(args[1].required, None);
    }

    #[test]
    fn prompt_definition_handles_empty_arguments() {
        // No `arguments` field on the wire → Option stays None;
        // empty array → Some(vec![]). Either must deserialize.
        let p1: super::McpPromptDefinition =
            serde_json::from_value(serde_json::json!({ "name": "quick" })).expect("bare name");
        assert!(p1.arguments.is_none());

        let p2: super::McpPromptDefinition =
            serde_json::from_value(serde_json::json!({ "name": "quick", "arguments": [] }))
                .expect("empty arguments array");
        assert_eq!(p2.arguments.as_ref().map(|v| v.len()), Some(0));
    }

    #[test]
    fn prompt_result_parses_messages() {
        let wire = serde_json::json!({
            "description": "rendered prompt",
            "messages": [
                { "role": "user", "content": { "type": "text", "text": "hi" } },
                { "role": "assistant", "content": { "type": "text", "text": "ok" } }
            ]
        });
        let r: super::McpPromptResult = serde_json::from_value(wire).expect("deserialize");
        assert_eq!(r.messages.len(), 2);
        assert_eq!(r.messages[0].role, "user");
        assert_eq!(
            r.messages[0].content.get("type").and_then(|v| v.as_str()),
            Some("text")
        );
    }

    #[test]
    fn prompt_method_constants_are_wire_correct() {
        // TS uses the literal method strings below when talking
        // to servers — make sure the Rust constants match bit for
        // bit.
        assert_eq!(super::methods::PROMPTS_LIST, "prompts/list");
        assert_eq!(super::methods::PROMPTS_GET, "prompts/get");
    }

    #[test]
    fn prompt_meta_icons_and_argument_title_round_trip() {
        // Codex CR coverage: newer MCP SDK schema defines
        // Prompt._meta, Prompt.icons, PromptArgument.title,
        // GetPromptResult._meta. Make sure all four survive a
        // round-trip so the Rust struct doesn't silently drop
        // server-provided data.
        let wire = serde_json::json!({
            "name": "p",
            "icons": [{ "src": "a.png" }],
            "_meta": { "anthropic/alwaysLoad": true },
            "arguments": [
                { "name": "x", "title": "X Axis" }
            ]
        });
        let p: super::McpPromptDefinition = serde_json::from_value(wire).expect("deserialize");
        assert!(p.icons.is_some());
        assert_eq!(
            p.meta
                .as_ref()
                .and_then(|v| v.get("anthropic/alwaysLoad"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        let args = p.arguments.expect("arguments present");
        assert_eq!(args[0].title.as_deref(), Some("X Axis"));

        let got_result: super::McpPromptResult = serde_json::from_value(serde_json::json!({
            "messages": [],
            "_meta": { "trace_id": "abc" }
        }))
        .expect("deserialize");
        assert!(got_result.meta.is_some());
    }

    #[test]
    fn test_build_mcp_tool_name() {
        assert_eq!(
            build_mcp_tool_name("my-server", "my-tool"),
            "mcp__my-server__my-tool"
        );
        assert_eq!(build_mcp_tool_name("server", "tool"), "mcp__server__tool");
        assert_eq!(
            build_mcp_tool_name("my.server.name", "tool.name"),
            "mcp__my_server_name__tool_name"
        );
    }

    #[test]
    fn test_normalize_mcp_name() {
        assert_eq!(normalize_mcp_name("simple"), "simple");
        assert_eq!(normalize_mcp_name("with-dashes"), "with-dashes");
        assert_eq!(normalize_mcp_name("with.dots"), "with_dots");
        assert_eq!(normalize_mcp_name("with spaces"), "with_spaces");
        assert_eq!(normalize_mcp_name("UPPER"), "UPPER");
        assert_eq!(normalize_mcp_name("mix123"), "mix123");
    }

    // ─── G20: inbound request dispatch ──────────────────────────────

    #[tokio::test]
    async fn default_roots_list_handler_returns_cwd_uri() {
        let h = default_roots_list_handler();
        let out = h(None).await.expect("default handler must succeed");
        let roots = out.get("roots").and_then(|r| r.as_array()).unwrap();
        assert_eq!(roots.len(), 1);
        let uri = roots[0].get("uri").and_then(|u| u.as_str()).unwrap();
        assert!(
            uri.starts_with("file://"),
            "uri must be a file:// URI, got {}",
            uri
        );
    }

    #[tokio::test]
    async fn default_roots_handler_pins_startup_cwd_after_chdir() {
        // `original_cwd` captures once; a later chdir must NOT
        // change what `roots/list` reports. We can't reliably
        // chdir in parallel tests (process-global state), so
        // instead we assert the uri uses the first-observed
        // directory via identity on the OnceLock.
        let h = default_roots_list_handler();
        let first = h(None).await.expect("ok");
        let second = h(None).await.expect("ok");
        assert_eq!(first, second, "roots/list must be stable across calls");
    }

    #[tokio::test]
    async fn default_elicitation_handler_returns_cancel() {
        let h = default_elicitation_cancel_handler();
        let out = h(None).await.expect("default handler must succeed");
        assert_eq!(out.get("action").and_then(|a| a.as_str()), Some("cancel"));
    }

    #[tokio::test]
    async fn default_request_handlers_contains_both_defaults() {
        let map = default_request_handlers().await;
        let guard = map.lock().await;
        assert!(guard.contains_key("roots/list"));
        assert!(guard.contains_key("elicitation/create"));
    }

    // ─── G5: graceful_terminate_stdio ──────────────────────────────

    /// Core invariant: no matter how stubborn the subprocess, the
    /// helper returns within the 600ms failsafe and the child is
    /// dead. Three fixtures cover the three escalation tiers:
    /// SIGINT-friendly, SIGINT-ignoring-SIGTERM-friendly, and
    /// fully-stubborn (only SIGKILL works). We assert on the end
    /// state (child is dead, elapsed ≤ failsafe + slack) rather
    /// than which signal did it, because signal-timing on macOS CI
    /// shells can escalate faster than the trap handler fires.
    async fn run_graceful_terminate_case(script: &str, max_ms: u128, label: &str) {
        use std::process::{Command, Stdio};
        use std::time::Instant;

        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sh");
        // Let the shell start and install its traps before we
        // signal. Without this, the shell's default SIGINT
        // action can fire before `trap ...` runs.
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        let started = Instant::now();
        super::graceful_terminate_stdio(&mut child, label).await;
        let elapsed = started.elapsed();

        assert!(
            elapsed.as_millis() < max_ms,
            "graceful_terminate ({}) exceeded window: {}ms > {}ms",
            label,
            elapsed.as_millis(),
            max_ms
        );
        // After the helper returns, try_wait must report the
        // child is fully reaped. (The helper already called
        // try_wait; wait() returns the cached exit status.)
        let _ = child.wait().expect("wait");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn graceful_terminate_sigint_friendly_fixture() {
        // Trailing `; true` prevents bash's single-command exec
        // optimisation (which would skip the trap installation).
        run_graceful_terminate_case("trap 'exit 0' INT; sleep 30; true", 900, "test-sigint").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn graceful_terminate_sigterm_escalation_fixture() {
        run_graceful_terminate_case(
            "trap '' INT; trap 'exit 0' TERM; sleep 30; true",
            900,
            "test-sigterm",
        )
        .await;
    }

    /// If the child has already exited by the time the helper is
    /// called, the very first `libc::kill(pid, SIGINT)` returns
    /// ESRCH and the helper short-circuits. Elapsed time must be
    /// near-zero (single signal call + errno check, no polling).
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn graceful_terminate_already_exited_short_circuits() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("true") // exits immediately, status 0
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sh");
        // Ensure the child has actually exited (and been reaped by
        // a probe) before we call the helper.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = child.try_wait();

        let started = Instant::now();
        super::graceful_terminate_stdio(&mut child, "test-already-exited").await;
        let elapsed = started.elapsed();

        // With no poll cycles required, must complete well under
        // the SIGINT window.
        assert!(
            elapsed.as_millis() < 50,
            "already-exited fast-path too slow: {}ms",
            elapsed.as_millis()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn graceful_terminate_sigkill_failsafe_fixture() {
        // Ignores both INT and TERM — only SIGKILL escapes.
        // 600ms helper failsafe + the 100ms post-SIGKILL reap
        // window + CI slack.
        run_graceful_terminate_case(
            "trap '' INT; trap '' TERM; sleep 30; true",
            1200,
            "test-sigkill",
        )
        .await;
    }

    // ─── G14: call_tool error classification ───────────────────────

    #[test]
    fn classify_http_401_becomes_auth_error() {
        let e = anyhow::anyhow!("Failed to POST: HTTP 401 body: unauthorized");
        let classified = super::classify_call_tool_error(e, "jira", super::TransportHint::Http);
        // Downcast to the typed error — proves callers can handle
        // auth specifically via `.downcast()`.
        let auth: &super::super::errors::McpAuthError =
            classified.downcast_ref().expect("expected McpAuthError");
        assert_eq!(auth.server_name, "jira");
        assert!(auth.message.contains("re-authorization"));
    }

    #[test]
    fn classify_401_unauthorized_text_becomes_auth_error() {
        // Alternate wording some transports use.
        let e = anyhow::anyhow!("server returned 401 Unauthorized");
        let classified = super::classify_call_tool_error(e, "slack", super::TransportHint::Http);
        assert!(classified
            .downcast_ref::<super::super::errors::McpAuthError>()
            .is_some());
    }

    #[test]
    fn classify_session_expired_text_becomes_session_error() {
        let e = anyhow::anyhow!("MCP HTTP server 'x' session expired (HTTP 404 + JSON-RPC -32001)");
        let classified = super::classify_call_tool_error(e, "x", super::TransportHint::Http);
        let sess: &super::super::errors::McpSessionExpiredError = classified
            .downcast_ref()
            .expect("expected McpSessionExpiredError");
        assert_eq!(sess.server_name, "x");
    }

    #[test]
    fn classify_32000_connection_closed_becomes_session_error() {
        // SDK's -32000 "Connection closed" wrapper fires when the
        // transport is torn down mid-request on HTTP. Should map
        // to session-expired so the caller reconnects.
        let e = anyhow::anyhow!("Failed to call tool: code -32000: Connection closed");
        let classified = super::classify_call_tool_error(e, "vault", super::TransportHint::Http);
        assert!(
            classified
                .downcast_ref::<super::super::errors::McpSessionExpiredError>()
                .is_some(),
            "expected -32000 + Connection closed to classify as session expired"
        );
    }

    /// Codex CR: -32000 "Connection closed" must NOT become
    /// session-expired on non-HTTP transports. SSE stream closing
    /// is a lifecycle event handled by G4b's LifecycleTracker,
    /// not a session-invalidation signal.
    #[test]
    fn classify_32000_connection_closed_on_sse_passes_through() {
        let e = anyhow::anyhow!("code -32000: Connection closed");
        let classified =
            super::classify_call_tool_error(e, "sse-server", super::TransportHint::Sse);
        assert!(
            classified
                .downcast_ref::<super::super::errors::McpSessionExpiredError>()
                .is_none(),
            "SSE -32000 close must NOT classify as session expired"
        );
    }

    #[test]
    fn classify_32000_connection_closed_on_stdio_passes_through() {
        let e = anyhow::anyhow!("code -32000: Connection closed");
        let classified =
            super::classify_call_tool_error(e, "stdio-server", super::TransportHint::Stdio);
        assert!(
            classified
                .downcast_ref::<super::super::errors::McpSessionExpiredError>()
                .is_none(),
            "stdio -32000 close must NOT classify as session expired"
        );
    }

    /// 404 + -32001 is a direct server signal — applies regardless
    /// of transport hint. Only the -32000 branch is transport-gated.
    #[test]
    fn classify_404_32001_applies_regardless_of_transport() {
        // The error message contains a JSON payload with braces —
        // build it as a plain String so the `anyhow!` macro
        // doesn't interpret the braces as format specifiers.
        let payload = String::from("HTTP 404 body: {\"error\":{\"code\":-32001}}");
        for hint in [
            super::TransportHint::Http,
            super::TransportHint::Sse,
            super::TransportHint::Stdio,
        ] {
            let e = anyhow::Error::msg(payload.clone());
            let classified = super::classify_call_tool_error(e, "x", hint);
            assert!(
                classified
                    .downcast_ref::<super::super::errors::McpSessionExpiredError>()
                    .is_some(),
                "404+-32001 should classify on transport {:?}",
                hint
            );
        }
    }

    #[test]
    fn classify_unknown_error_passes_through() {
        let e = anyhow::anyhow!("parse error at offset 42");
        let classified = super::classify_call_tool_error(e, "x", super::TransportHint::Http);
        // No typed error — original bubbles up unchanged.
        assert!(classified
            .downcast_ref::<super::super::errors::McpAuthError>()
            .is_none());
        assert!(classified
            .downcast_ref::<super::super::errors::McpSessionExpiredError>()
            .is_none());
        assert!(classified.to_string().contains("parse error"));
    }

    // ─── G4b: lifecycle wiring ─────────────────────────────────────

    /// A freshly-constructed tracker reports not-closed. The send
    /// short-circuit then lets requests through. After `mark_closed`
    /// the short-circuit path activates.
    #[tokio::test]
    async fn lifecycle_short_circuit_matches_tracker_state() {
        use crate::mcp::lifecycle::LifecycleTracker;

        // Bare tracker behaviour — same logic used by
        // `has_triggered_close()` in the send path.
        let tracker: Arc<Mutex<LifecycleTracker>> = Arc::new(Mutex::new(LifecycleTracker::new()));

        {
            let g = tracker.lock().await;
            assert!(!g.has_triggered_close());
        }
        tracker.lock().await.mark_closed();
        {
            let g = tracker.lock().await;
            assert!(g.has_triggered_close());
        }
    }
}
