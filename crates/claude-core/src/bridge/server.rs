use anyhow::Result;
use tokio::net::TcpListener;

use super::protocol::{BridgeError, BridgeRequest, BridgeResponse};
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

/// Route a single request to the appropriate handler and produce a response.
pub fn dispatch_request(request: &BridgeRequest) -> BridgeResponse {
    match request.method.as_str() {
        "status" => BridgeResponse {
            id: request.id.clone(),
            result: Some(serde_json::json!({"state": "ready"})),
            error: None,
        },
        "ping" => BridgeResponse {
            id: request.id.clone(),
            result: Some(serde_json::json!({"pong": true})),
            error: None,
        },
        _ => BridgeResponse {
            id: request.id.clone(),
            result: None,
            error: Some(BridgeError {
                code: -1,
                message: format!("Unknown method: {}", request.method),
            }),
        },
    }
}

/// Handle a single bridge TCP connection (newline-delimited JSON).
async fn handle_connection(stream: tokio::net::TcpStream) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let request: BridgeRequest = serde_json::from_str(&line)?;
        let response = dispatch_request(&request);
        let json = serde_json::to_string(&response)?;
        writer.write_all(json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
    }

    Ok(())
}
