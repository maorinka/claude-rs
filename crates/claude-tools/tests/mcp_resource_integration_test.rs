use claude_core::mcp::manager::McpManager;
use claude_core::mcp::types::{
    ConfigScope, McpConnectionStatus, McpServerConfig, McpStdioServerConfig, ScopedMcpServerConfig,
};
use claude_tools::mcp_resource_tools::{ListMcpResourcesTool, ReadMcpResourceTool};
use claude_tools::registry::{PermissionMode, ReadFileState, ToolExecutor, ToolUseContext};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

fn make_ctx() -> ToolUseContext {
    ToolUseContext::for_test(
        std::env::temp_dir(),
        Arc::new(Mutex::new(ReadFileState::new())),
        PermissionMode::Default,
    )
}

fn python3_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok()
}

#[tokio::test]
async fn mcp_resource_tools_read_from_live_stdio_server() {
    if !python3_available() {
        eprintln!("skipping live MCP resource test: python3 not found");
        return;
    }

    let temp = tempdir().unwrap();
    let server_path = temp.path().join("fake_mcp_server.py");
    std::fs::write(
        &server_path,
        r#"
import json
import sys

def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    if not line.strip():
        continue
    request = json.loads(line)
    method = request.get("method")
    if "id" not in request:
        continue
    request_id = request["id"]
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"resources": {}},
                "serverInfo": {"name": "fake-resources", "version": "1.0.0"}
            }
        })
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": request_id, "result": {"tools": []}})
    elif method == "resources/list":
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "resources": [{
                    "uri": "file:///fake.txt",
                    "name": "fake.txt",
                    "description": "Fake resource",
                    "mimeType": "text/plain"
                }]
            }
        })
    elif method == "resources/read":
        uri = request.get("params", {}).get("uri")
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "contents": [{
                    "uri": uri,
                    "mimeType": "text/plain",
                    "text": "hello from resource"
                }]
            }
        })
    else:
        send({
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32601, "message": "method not found"}
        })
"#,
    )
    .unwrap();

    let manager = Arc::new(RwLock::new(McpManager::new()));
    let config = ScopedMcpServerConfig {
        config: McpServerConfig::Stdio(McpStdioServerConfig {
            command: "python3".to_string(),
            args: vec![server_path.display().to_string()],
            env: None,
        }),
        scope: ConfigScope::Project,
    };
    let connection = manager.write().await.connect_server("fake", config).await;
    assert!(
        matches!(connection.status, McpConnectionStatus::Connected { .. }),
        "fake MCP server should connect, got {:?}",
        connection.status
    );

    let ctx = make_ctx();
    let cancel = CancellationToken::new();

    let list_tool = ListMcpResourcesTool::new(manager.clone());
    let list_result = list_tool
        .call(&json!({ "server": "fake" }), &ctx, cancel.clone(), None)
        .await
        .unwrap();
    assert!(!list_result.is_error);
    assert_eq!(list_result.data[0]["uri"], "file:///fake.txt");
    assert_eq!(list_result.data[0]["name"], "fake.txt");
    assert_eq!(list_result.data[0]["mimeType"], "text/plain");
    assert_eq!(list_result.data[0]["server"], "fake");

    let read_tool = ReadMcpResourceTool::new(manager.clone());
    let read_result = read_tool
        .call(
            &json!({ "server": "fake", "uri": "file:///fake.txt" }),
            &ctx,
            cancel,
            None,
        )
        .await
        .unwrap();
    assert!(!read_result.is_error);
    assert_eq!(
        read_result.data["contents"][0]["text"],
        "hello from resource"
    );

    manager.write().await.disconnect_server("fake").await;
}
