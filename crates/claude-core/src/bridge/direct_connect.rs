//! Direct-connect session creation and StructuredIO message framing.
//!
//! Port of the non-UI logic in TS `src/server/createDirectConnectSession.ts`
//! and `src/server/directConnectManager.ts`. The manager is intentionally
//! transport-agnostic for tests: encode/decode helpers model exactly what is
//! sent over the WebSocket, while `create_direct_connect_session` performs the
//! real HTTP session-creation call.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectConnectConfig {
    pub server_url: String,
    pub session_id: String,
    pub ws_url: String,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectConnectSession {
    pub config: DirectConnectConfig,
    pub work_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectConnectError(pub String);

impl fmt::Display for DirectConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DirectConnectError {}

#[derive(Debug, Deserialize)]
struct ConnectResponse {
    session_id: String,
    ws_url: String,
    work_dir: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateSessionBody<'a> {
    cwd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    dangerously_skip_permissions: Option<bool>,
}

pub async fn create_direct_connect_session(
    http: &reqwest::Client,
    server_url: &str,
    auth_token: Option<&str>,
    cwd: &str,
    dangerously_skip_permissions: bool,
) -> std::result::Result<DirectConnectSession, DirectConnectError> {
    let mut req = http
        .post(format!("{}/sessions", server_url.trim_end_matches('/')))
        .header("content-type", "application/json")
        .json(&CreateSessionBody {
            cwd,
            dangerously_skip_permissions: dangerously_skip_permissions.then_some(true),
        });
    if let Some(token) = auth_token {
        req = req.bearer_auth(token);
    }

    let resp = req.send().await.map_err(|err| {
        DirectConnectError(format!(
            "Failed to connect to server at {server_url}: {err}"
        ))
    })?;

    let status = resp.status();
    if !status.is_success() {
        let reason = status.canonical_reason().unwrap_or("");
        return Err(DirectConnectError(format!(
            "Failed to create session: {} {}",
            status.as_u16(),
            reason
        )));
    }

    let data: ConnectResponse = resp
        .json()
        .await
        .map_err(|err| DirectConnectError(format!("Invalid session response: {err}")))?;
    if data.session_id.is_empty() || data.ws_url.is_empty() {
        return Err(DirectConnectError(
            "Invalid session response: missing session_id or ws_url".to_string(),
        ));
    }

    Ok(DirectConnectSession {
        config: DirectConnectConfig {
            server_url: server_url.to_string(),
            session_id: data.session_id,
            ws_url: data.ws_url,
            auth_token: auth_token.map(ToString::to_string),
        },
        work_dir: data.work_dir,
    })
}

pub struct DirectConnectWebSocket {
    write_tx: mpsc::Sender<String>,
}

impl DirectConnectWebSocket {
    pub async fn connect(
        config: &DirectConnectConfig,
        inbound_tx: mpsc::Sender<DirectConnectInbound>,
    ) -> Result<Self> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let mut request = config
            .ws_url
            .as_str()
            .into_client_request()
            .with_context(|| format!("invalid direct-connect WebSocket URL: {}", config.ws_url))?;
        if let Some(token) = config.auth_token.as_deref() {
            request.headers_mut().insert(
                "authorization",
                format!("Bearer {token}")
                    .parse()
                    .context("building authorization header")?,
            );
        }

        let (stream, _response) = tokio_tungstenite::connect_async(request)
            .await
            .with_context(|| format!("connecting direct-connect WebSocket {}", config.ws_url))?;
        let (mut write, mut read) = stream.split();
        let (write_tx, mut write_rx) = mpsc::channel::<String>(32);

        tokio::spawn(async move {
            while let Some(payload) = write_rx.recv().await {
                if write
                    .send(tokio_tungstenite::tungstenite::Message::Text(payload))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                let Ok(msg) = msg else {
                    break;
                };
                if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                    for inbound in parse_inbound_frame(&text) {
                        if inbound_tx.send(inbound).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        Ok(Self { write_tx })
    }

    pub async fn send_message(&self, content: Value) -> Result<bool> {
        let payload = encode_user_message(content)?;
        Ok(self.write_tx.send(payload).await.is_ok())
    }

    pub async fn respond_to_permission_request(
        &self,
        request_id: &str,
        behavior: &str,
        updated_input: Option<Value>,
        message: Option<&str>,
    ) -> Result<bool> {
        let payload = encode_permission_response(request_id, behavior, updated_input, message)?;
        Ok(self.write_tx.send(payload).await.is_ok())
    }

    pub async fn send_interrupt(&self, request_id: &str) -> Result<bool> {
        let payload = encode_interrupt_request(request_id)?;
        Ok(self.write_tx.send(payload).await.is_ok())
    }

    pub async fn send_error_response(&self, request_id: &str, error: &str) -> Result<bool> {
        let payload = encode_error_response(request_id, error)?;
        Ok(self.write_tx.send(payload).await.is_ok())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DirectConnectInbound {
    Message(Value),
    PermissionRequest {
        request: Value,
        request_id: String,
    },
    UnsupportedControlRequest {
        request_id: String,
        subtype: Option<String>,
    },
    Ignored,
}

/// Parse a WebSocket text frame. TS splits each frame by newline and ignores
/// malformed/non-StructuredIO lines.
pub fn parse_inbound_frame(frame: &str) -> Vec<DirectConnectInbound> {
    frame
        .split('\n')
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(parse_stdout_message)
        .collect()
}

fn parse_stdout_message(value: Value) -> Option<DirectConnectInbound> {
    let ty = value.get("type")?.as_str()?;
    if ty == "control_request" {
        let subtype = value
            .pointer("/request/subtype")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let request_id = value
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return if subtype.as_deref() == Some("can_use_tool") {
            Some(DirectConnectInbound::PermissionRequest {
                request: value.get("request").cloned().unwrap_or(Value::Null),
                request_id,
            })
        } else {
            Some(DirectConnectInbound::UnsupportedControlRequest {
                request_id,
                subtype,
            })
        };
    }

    if matches!(
        ty,
        "control_response"
            | "keep_alive"
            | "control_cancel_request"
            | "streamlined_text"
            | "streamlined_tool_use_summary"
    ) || (ty == "system"
        && value.get("subtype").and_then(Value::as_str) == Some("post_turn_summary"))
    {
        return Some(DirectConnectInbound::Ignored);
    }

    Some(DirectConnectInbound::Message(value))
}

/// Encode a remote user message exactly as TS `sendMessage` does.
pub fn encode_user_message(content: Value) -> Result<String> {
    serde_json::to_string(&serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": content,
        },
        "parent_tool_use_id": null,
        "session_id": "",
    }))
    .context("encoding direct-connect user message")
}

pub fn encode_permission_response(
    request_id: &str,
    behavior: &str,
    updated_input: Option<Value>,
    message: Option<&str>,
) -> Result<String> {
    let inner = if behavior == "allow" {
        serde_json::json!({
            "behavior": behavior,
            "updatedInput": updated_input.unwrap_or(Value::Null),
        })
    } else {
        serde_json::json!({
            "behavior": behavior,
            "message": message.unwrap_or_default(),
        })
    };
    serde_json::to_string(&serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": inner,
        },
    }))
    .context("encoding direct-connect permission response")
}

pub fn encode_interrupt_request(request_id: &str) -> Result<String> {
    serde_json::to_string(&serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "interrupt"},
    }))
    .context("encoding direct-connect interrupt request")
}

pub fn encode_error_response(request_id: &str, error: &str) -> Result<String> {
    serde_json::to_string(&serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": request_id,
            "error": error,
        },
    }))
    .context("encoding direct-connect error response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inbound_frame_filters_like_ts_manager() {
        let frame = [
            r#"{"type":"assistant","message":{"content":[]}}"#,
            r#"not json"#,
            r#"{"type":"keep_alive"}"#,
            r#"{"type":"system","subtype":"post_turn_summary"}"#,
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash"}}"#,
            r#"{"type":"control_request","request_id":"r2","request":{"subtype":"other"}}"#,
        ]
        .join("\n");
        let parsed = parse_inbound_frame(&frame);
        assert_eq!(parsed.len(), 5);
        assert!(matches!(parsed[0], DirectConnectInbound::Message(_)));
        assert_eq!(parsed[1], DirectConnectInbound::Ignored);
        assert_eq!(parsed[2], DirectConnectInbound::Ignored);
        assert!(matches!(
            parsed[3],
            DirectConnectInbound::PermissionRequest { ref request_id, .. } if request_id == "r1"
        ));
        assert!(matches!(
            parsed[4],
            DirectConnectInbound::UnsupportedControlRequest {
                ref request_id,
                ref subtype,
            } if request_id == "r2" && subtype.as_deref() == Some("other")
        ));
    }

    #[test]
    fn encodes_user_message_shape_like_ts() {
        let encoded = encode_user_message(json!("hello")).unwrap();
        let value: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["type"], "user");
        assert_eq!(value["message"]["role"], "user");
        assert_eq!(value["message"]["content"], "hello");
        assert!(value["parent_tool_use_id"].is_null());
        assert_eq!(value["session_id"], "");
    }

    #[test]
    fn encodes_permission_and_interrupt_shapes_like_ts() {
        let allow =
            encode_permission_response("req", "allow", Some(json!({"x": 1})), None).unwrap();
        let allow: Value = serde_json::from_str(&allow).unwrap();
        assert_eq!(allow["type"], "control_response");
        assert_eq!(allow["response"]["subtype"], "success");
        assert_eq!(allow["response"]["request_id"], "req");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(allow["response"]["response"]["updatedInput"]["x"], 1);

        let interrupt = encode_interrupt_request("i").unwrap();
        let interrupt: Value = serde_json::from_str(&interrupt).unwrap();
        assert_eq!(interrupt["type"], "control_request");
        assert_eq!(interrupt["request"]["subtype"], "interrupt");
    }
}
