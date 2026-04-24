use claude_core::bridge::protocol::{BridgeError, BridgeRequest, BridgeResponse};
use claude_core::bridge::server::{dispatch_request_stateless, BridgeServer};
use claude_core::bridge::types::{BridgeConfig, BridgeMessage, IdeType, SessionInfo, SessionState};

// ─── BridgeMessage serde ────────────────────────────────────────────────────

#[test]
fn bridge_message_file_changed_roundtrip() {
    let msg = BridgeMessage::FileChanged {
        path: "src/main.rs".into(),
        content: Some("fn main() {}".into()),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"file_changed""#));
    let decoded: BridgeMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn bridge_message_file_changed_no_content() {
    let json = r#"{"type":"file_changed","path":"foo.rs","content":null}"#;
    let msg: BridgeMessage = serde_json::from_str(json).unwrap();
    match msg {
        BridgeMessage::FileChanged { path, content } => {
            assert_eq!(path, "foo.rs");
            assert!(content.is_none());
        }
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn bridge_message_diff_roundtrip() {
    let msg = BridgeMessage::Diff {
        path: "lib.rs".into(),
        diff: "@@ -1 +1 @@\n-old\n+new".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let decoded: BridgeMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn bridge_message_permission_request_roundtrip() {
    let msg = BridgeMessage::PermissionRequest {
        tool: "bash".into(),
        input: serde_json::json!({"command": "ls"}),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"permission_request""#));
    let decoded: BridgeMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn bridge_message_permission_response_roundtrip() {
    let msg = BridgeMessage::PermissionResponse {
        tool: "bash".into(),
        allowed: true,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let decoded: BridgeMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn bridge_message_prompt_roundtrip() {
    let msg = BridgeMessage::Prompt {
        text: "Hello Claude".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"prompt""#));
    let decoded: BridgeMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn bridge_message_response_roundtrip() {
    let msg = BridgeMessage::Response {
        text: "Here is the answer".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let decoded: BridgeMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn bridge_message_status_with_message() {
    let msg = BridgeMessage::Status {
        state: "running".into(),
        message: Some("Processing...".into()),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let decoded: BridgeMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn bridge_message_status_without_message() {
    let json = r#"{"type":"status","state":"ready","message":null}"#;
    let msg: BridgeMessage = serde_json::from_str(json).unwrap();
    match msg {
        BridgeMessage::Status { state, message } => {
            assert_eq!(state, "ready");
            assert!(message.is_none());
        }
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn bridge_message_error_roundtrip() {
    let msg = BridgeMessage::Error {
        message: "something went wrong".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"error""#));
    let decoded: BridgeMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, msg);
}

// ─── BridgeConfig / IdeType ─────────────────────────────────────────────────

#[test]
fn bridge_config_default() {
    let cfg = BridgeConfig::default();
    assert!(cfg.port.is_none());
    assert_eq!(cfg.host, "127.0.0.1");
    assert_eq!(cfg.ide, IdeType::Other("unknown".to_string()));
}

#[test]
fn bridge_config_serde_roundtrip() {
    let cfg = BridgeConfig {
        port: Some(9123),
        host: "0.0.0.0".into(),
        ide: IdeType::VSCode,
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let decoded: BridgeConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.port, Some(9123));
    assert_eq!(decoded.host, "0.0.0.0");
    assert_eq!(decoded.ide, IdeType::VSCode);
}

#[test]
fn ide_type_jetbrains_serde() {
    let ide = IdeType::JetBrains;
    let json = serde_json::to_string(&ide).unwrap();
    let decoded: IdeType = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, IdeType::JetBrains);
}

// ─── SessionState / SessionInfo ─────────────────────────────────────────────

#[test]
fn session_state_serde() {
    let state = SessionState::Running;
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, r#""running""#);
    let decoded: SessionState = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, SessionState::Running);
}

#[test]
fn session_info_roundtrip() {
    let info = SessionInfo {
        id: "sess-1".into(),
        status: SessionState::Starting,
        created_at: 1700000000,
        work_dir: "/tmp/project".into(),
    };
    let json = serde_json::to_string(&info).unwrap();
    let decoded: SessionInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.id, "sess-1");
    assert_eq!(decoded.status, SessionState::Starting);
}

// ─── BridgeRequest / BridgeResponse serde ───────────────────────────────────

#[test]
fn bridge_request_roundtrip() {
    let req = BridgeRequest {
        id: "req-1".into(),
        method: "ping".into(),
        params: serde_json::json!({}),
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: BridgeRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, req);
}

#[test]
fn bridge_request_missing_params_defaults_to_null() {
    let json = r#"{"id":"1","method":"ping"}"#;
    let req: BridgeRequest = serde_json::from_str(json).unwrap();
    assert!(req.params.is_null());
}

#[test]
fn bridge_response_success() {
    let resp = BridgeResponse::success("r-1".into(), serde_json::json!({"ok": true}));
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());

    let json = serde_json::to_string(&resp).unwrap();
    assert!(!json.contains("error"));
    let decoded: BridgeResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, resp);
}

#[test]
fn bridge_response_error() {
    let resp = BridgeResponse::error("r-2".into(), -1, "bad method".into());
    assert!(resp.result.is_none());
    assert!(resp.error.is_some());

    let json = serde_json::to_string(&resp).unwrap();
    assert!(!json.contains("result"));
    let decoded: BridgeResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, resp);
}

#[test]
fn bridge_error_fields() {
    let err = BridgeError {
        code: -32600,
        message: "Invalid request".into(),
    };
    let json = serde_json::to_string(&err).unwrap();
    let decoded: BridgeError = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.code, -32600);
    assert_eq!(decoded.message, "Invalid request");
}

// ─── dispatch_request (unit, no network) ────────────────────────────────────

#[test]
fn dispatch_ping() {
    let req = BridgeRequest {
        id: "1".into(),
        method: "ping".into(),
        params: serde_json::Value::Null,
    };
    let resp = dispatch_request_stateless(&req);
    assert_eq!(resp.id, "1");
    let result = resp.result.unwrap();
    assert_eq!(result["pong"], true);
    assert!(resp.error.is_none());
}

#[test]
fn dispatch_status() {
    let req = BridgeRequest {
        id: "2".into(),
        method: "status".into(),
        params: serde_json::Value::Null,
    };
    let resp = dispatch_request_stateless(&req);
    assert_eq!(resp.id, "2");
    let result = resp.result.unwrap();
    assert_eq!(result["state"], "ready");
}

#[test]
fn dispatch_unknown_method() {
    let req = BridgeRequest {
        id: "3".into(),
        method: "nonexistent".into(),
        params: serde_json::Value::Null,
    };
    let resp = dispatch_request_stateless(&req);
    assert_eq!(resp.id, "3");
    assert!(resp.result.is_none());
    let err = resp.error.unwrap();
    assert_eq!(err.code, -1);
    assert!(err.message.contains("nonexistent"));
}

// ─── BridgeServer integration (binds + ping over TCP) ───────────────────────

#[tokio::test]
async fn bridge_server_binds_to_port() {
    let config = BridgeConfig {
        port: None, // ephemeral
        host: "127.0.0.1".into(),
        ide: IdeType::VSCode,
    };
    let server = BridgeServer::new(config);
    let addr = server.start_once().await.unwrap();
    assert_ne!(addr.port(), 0);
}

#[tokio::test]
async fn bridge_server_handles_ping() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    let config = BridgeConfig {
        port: None,
        host: "127.0.0.1".into(),
        ide: IdeType::VSCode,
    };
    let server = BridgeServer::new(config);
    let addr = server.start_once().await.unwrap();

    let stream = TcpStream::connect(addr).await.unwrap();
    let (reader, mut writer) = stream.into_split();

    // Send a ping request
    let req = BridgeRequest {
        id: "test-1".into(),
        method: "ping".into(),
        params: serde_json::Value::Null,
    };
    let mut payload = serde_json::to_string(&req).unwrap();
    payload.push('\n');
    writer.write_all(payload.as_bytes()).await.unwrap();

    // Read the response
    let mut lines = BufReader::new(reader).lines();
    let line = lines
        .next_line()
        .await
        .unwrap()
        .expect("expected a response line");
    let resp: BridgeResponse = serde_json::from_str(&line).unwrap();

    assert_eq!(resp.id, "test-1");
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    assert_eq!(result["pong"], true);
}

#[tokio::test]
async fn bridge_server_handles_unknown_method() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    let config = BridgeConfig {
        port: None,
        host: "127.0.0.1".into(),
        ide: IdeType::Other("test".into()),
    };
    let server = BridgeServer::new(config);
    let addr = server.start_once().await.unwrap();

    let stream = TcpStream::connect(addr).await.unwrap();
    let (reader, mut writer) = stream.into_split();

    let req = BridgeRequest {
        id: "test-2".into(),
        method: "bogus".into(),
        params: serde_json::json!({"x": 1}),
    };
    let mut payload = serde_json::to_string(&req).unwrap();
    payload.push('\n');
    writer.write_all(payload.as_bytes()).await.unwrap();

    let mut lines = BufReader::new(reader).lines();
    let line = lines
        .next_line()
        .await
        .unwrap()
        .expect("expected a response line");
    let resp: BridgeResponse = serde_json::from_str(&line).unwrap();

    assert_eq!(resp.id, "test-2");
    assert!(resp.result.is_none());
    let err = resp.error.unwrap();
    assert_eq!(err.code, -1);
    assert!(err.message.contains("bogus"));
}
