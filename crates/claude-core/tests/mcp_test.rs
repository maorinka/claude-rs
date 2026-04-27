use std::collections::HashMap;

use claude_core::mcp::client::{build_mcp_tool_name, normalize_mcp_name};
use claude_core::mcp::manager::McpManager;
use claude_core::mcp::types::*;

// --- Type serialization tests ---

#[test]
fn test_transport_type_serialization() {
    let stdio = TransportType::Stdio;
    let json = serde_json::to_string(&stdio).unwrap();
    assert_eq!(json, "\"stdio\"");

    let sse = TransportType::Sse;
    let json = serde_json::to_string(&sse).unwrap();
    assert_eq!(json, "\"sse\"");

    let http = TransportType::Http;
    let json = serde_json::to_string(&http).unwrap();
    assert_eq!(json, "\"http\"");
}

#[test]
fn test_transport_type_deserialization() {
    let stdio: TransportType = serde_json::from_str("\"stdio\"").unwrap();
    assert_eq!(stdio, TransportType::Stdio);

    let sse: TransportType = serde_json::from_str("\"sse\"").unwrap();
    assert_eq!(sse, TransportType::Sse);

    let http: TransportType = serde_json::from_str("\"http\"").unwrap();
    assert_eq!(http, TransportType::Http);
}

#[test]
fn test_config_scope_serialization() {
    let scope = ConfigScope::Local;
    let json = serde_json::to_string(&scope).unwrap();
    assert_eq!(json, "\"local\"");

    let scope = ConfigScope::User;
    let json = serde_json::to_string(&scope).unwrap();
    assert_eq!(json, "\"user\"");

    let scope = ConfigScope::ClaudeAi;
    let json = serde_json::to_string(&scope).unwrap();
    assert_eq!(json, "\"claudeai\"");
}

#[test]
fn test_stdio_config_deserialization() {
    let json = r#"{
        "type": "stdio",
        "command": "node",
        "args": ["server.js", "--port", "3000"],
        "env": {"NODE_ENV": "production"}
    }"#;

    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    match config {
        McpServerConfig::Stdio(stdio) => {
            assert_eq!(stdio.command, "node");
            assert_eq!(stdio.args, vec!["server.js", "--port", "3000"]);
            let env = stdio.env.unwrap();
            assert_eq!(env.get("NODE_ENV").unwrap(), "production");
        }
        _ => panic!("Expected stdio config"),
    }
}

#[test]
fn test_stdio_config_minimal() {
    let json = r#"{
        "type": "stdio",
        "command": "my-mcp-server"
    }"#;

    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    match config {
        McpServerConfig::Stdio(stdio) => {
            assert_eq!(stdio.command, "my-mcp-server");
            assert!(stdio.args.is_empty());
            assert!(stdio.env.is_none());
        }
        _ => panic!("Expected stdio config"),
    }
}

#[test]
fn test_stdio_config_from_value_no_type() {
    // When "type" is missing, should default to stdio
    let value = serde_json::json!({
        "command": "my-server",
        "args": ["--verbose"]
    });

    let config = McpServerConfig::from_value(value).unwrap();
    match config {
        McpServerConfig::Stdio(stdio) => {
            assert_eq!(stdio.command, "my-server");
            assert_eq!(stdio.args, vec!["--verbose"]);
        }
        _ => panic!("Expected stdio config"),
    }
}

#[test]
fn test_sse_config_deserialization() {
    let json = r#"{
        "type": "sse",
        "url": "https://example.com/mcp",
        "headers": {"Authorization": "Bearer token123"}
    }"#;

    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    match config {
        McpServerConfig::Sse(sse) => {
            assert_eq!(sse.url, "https://example.com/mcp");
            let headers = sse.headers.unwrap();
            assert_eq!(headers.get("Authorization").unwrap(), "Bearer token123");
        }
        _ => panic!("Expected SSE config"),
    }
}

#[test]
fn test_http_config_deserialization() {
    let json = r#"{
        "type": "http",
        "url": "https://api.example.com/mcp"
    }"#;

    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    match config {
        McpServerConfig::Http(http) => {
            assert_eq!(http.url, "https://api.example.com/mcp");
            assert!(http.headers.is_none());
        }
        _ => panic!("Expected HTTP config"),
    }
}

#[test]
fn test_server_capabilities_deserialization() {
    let json = r#"{
        "tools": {"listChanged": true},
        "resources": {"listChanged": false, "subscribe": true}
    }"#;

    let caps: ServerCapabilities = serde_json::from_str(json).unwrap();
    assert!(caps.tools.is_some());
    assert!(caps.resources.is_some());
    assert!(caps.prompts.is_none());
}

#[test]
fn test_server_capabilities_empty() {
    let json = "{}";
    let caps: ServerCapabilities = serde_json::from_str(json).unwrap();
    assert!(caps.tools.is_none());
    assert!(caps.resources.is_none());
    assert!(caps.prompts.is_none());
    assert!(caps.experimental.is_none());
}

#[test]
fn test_connection_status_serialization() {
    let connected = McpConnectionStatus::Connected {
        capabilities: ServerCapabilities::default(),
        server_info: Some(ServerInfo {
            name: "test-server".to_string(),
            version: "1.0.0".to_string(),
        }),
        instructions: None,
    };
    let json = serde_json::to_value(&connected).unwrap();
    assert_eq!(json["type"], "connected");
    assert_eq!(json["server_info"]["name"], "test-server");

    let failed = McpConnectionStatus::Failed {
        error: Some("connection refused".to_string()),
    };
    let json = serde_json::to_value(&failed).unwrap();
    assert_eq!(json["type"], "failed");
    assert_eq!(json["error"], "connection refused");

    let pending = McpConnectionStatus::Pending {
        reconnect_attempt: Some(2),
        max_reconnect_attempts: Some(5),
    };
    let json = serde_json::to_value(&pending).unwrap();
    assert_eq!(json["type"], "pending");
    assert_eq!(json["reconnect_attempt"], 2);

    let disabled = McpConnectionStatus::Disabled;
    let json = serde_json::to_value(&disabled).unwrap();
    assert_eq!(json["type"], "disabled");
}

#[test]
fn test_server_connection_is_connected() {
    let conn = McpServerConnection {
        name: "test".to_string(),
        status: McpConnectionStatus::Connected {
            capabilities: ServerCapabilities::default(),
            server_info: None,
            instructions: None,
        },
        config: ScopedMcpServerConfig {
            config: McpServerConfig::Stdio(McpStdioServerConfig {
                command: "test".to_string(),
                args: vec![],
                env: None,
            }),
            scope: ConfigScope::Local,
        },
    };
    assert!(conn.is_connected());
    assert!(!conn.is_failed());
    assert!(!conn.is_pending());
}

#[test]
fn test_tool_definition_deserialization() {
    let json = r#"{
        "name": "read_file",
        "description": "Read the contents of a file",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to read"
                }
            },
            "required": ["path"]
        }
    }"#;

    let tool: McpToolDefinition = serde_json::from_str(json).unwrap();
    assert_eq!(tool.name, "read_file");
    assert_eq!(
        tool.description.as_deref(),
        Some("Read the contents of a file")
    );

    let schema = tool.input_schema.unwrap();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["path"].is_object());
}

#[test]
fn test_tool_result_deserialization() {
    let json = r#"{
        "content": [
            {"type": "text", "text": "Hello, world!"}
        ],
        "isError": false
    }"#;

    let result: McpToolResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.content.len(), 1);
    assert_eq!(result.content[0].content_type, "text");
    assert_eq!(result.content[0].text.as_deref(), Some("Hello, world!"));
    assert_eq!(result.is_error, Some(false));
}

#[test]
fn test_tool_result_with_error() {
    let json = r#"{
        "content": [
            {"type": "text", "text": "File not found: /nonexistent"}
        ],
        "isError": true
    }"#;

    let result: McpToolResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(result.content[0]
        .text
        .as_deref()
        .unwrap()
        .contains("not found"));
}

#[test]
fn test_tool_result_with_image() {
    let json = r#"{
        "content": [
            {"type": "image", "data": "base64data==", "mimeType": "image/png"}
        ]
    }"#;

    let result: McpToolResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.content.len(), 1);
    assert_eq!(result.content[0].content_type, "image");
    assert_eq!(result.content[0].data.as_deref(), Some("base64data=="));
    assert_eq!(result.content[0].mime_type.as_deref(), Some("image/png"));
    assert!(result.is_error.is_none());
}

// --- JSON-RPC message construction tests ---

#[test]
fn test_jsonrpc_request_construction() {
    let request = JsonRpcRequest::new(1, "tools/list", Some(serde_json::json!({})));
    assert_eq!(request.jsonrpc, "2.0");
    assert_eq!(request.id, 1);
    assert_eq!(request.method, "tools/list");

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["id"], 1);
    assert_eq!(json["method"], "tools/list");
}

#[test]
fn test_jsonrpc_request_no_params() {
    let request = JsonRpcRequest::new(42, "ping", None);
    let json = serde_json::to_string(&request).unwrap();

    // Should not include "params" key when None
    assert!(!json.contains("params"));
    assert!(json.contains("\"id\":42"));
    assert!(json.contains("\"method\":\"ping\""));
}

#[test]
fn test_jsonrpc_initialize_request() {
    let request = JsonRpcRequest::new(
        1,
        methods::INITIALIZE,
        Some(serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": MCP_CLIENT_NAME,
                "version": MCP_CLIENT_VERSION
            }
        })),
    );

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["method"], "initialize");
    assert_eq!(json["params"]["protocolVersion"], MCP_PROTOCOL_VERSION);
    assert_eq!(json["params"]["clientInfo"]["name"], MCP_CLIENT_NAME);
}

#[test]
fn test_jsonrpc_tools_call_request() {
    let request = JsonRpcRequest::new(
        5,
        methods::TOOLS_CALL,
        Some(serde_json::json!({
            "name": "read_file",
            "arguments": {
                "path": "/tmp/test.txt"
            }
        })),
    );

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["method"], "tools/call");
    assert_eq!(json["params"]["name"], "read_file");
    assert_eq!(json["params"]["arguments"]["path"], "/tmp/test.txt");
}

#[test]
fn test_jsonrpc_notification_construction() {
    let notification = JsonRpcNotification::new(methods::INITIALIZED, None);

    let json = serde_json::to_value(&notification).unwrap();
    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["method"], "notifications/initialized");
    // Notifications should not have an "id" field
    assert!(json.get("id").is_none());
}

#[test]
fn test_jsonrpc_response_deserialization_success() {
    let json = r#"{
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "tools": [
                {"name": "test_tool", "description": "A test tool"}
            ]
        }
    }"#;

    let response: JsonRpcResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.id, 1);
    assert!(response.result.is_some());
    assert!(response.error.is_none());
}

#[test]
fn test_jsonrpc_response_deserialization_error() {
    let json = r#"{
        "jsonrpc": "2.0",
        "id": 2,
        "error": {
            "code": -32601,
            "message": "Method not found"
        }
    }"#;

    let response: JsonRpcResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.id, 2);
    assert!(response.result.is_none());
    let error = response.error.unwrap();
    assert_eq!(error.code, -32601);
    assert_eq!(error.message, "Method not found");
}

// --- Tool schema parsing tests ---

#[test]
fn test_parse_tools_list_response() {
    let response_json = serde_json::json!({
        "tools": [
            {
                "name": "read_file",
                "description": "Read file contents",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path"}
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "write_file",
                "description": "Write to a file",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"]
                }
            }
        ]
    });

    let tools_value = response_json.get("tools").unwrap();
    let tools: Vec<McpToolDefinition> = serde_json::from_value(tools_value.clone()).unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "read_file");
    assert_eq!(tools[1].name, "write_file");

    // Check schema structure
    let schema = tools[0].input_schema.as_ref().unwrap();
    assert_eq!(schema["type"], "object");
    let required = schema["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0], "path");
}

#[test]
fn test_parse_tool_with_no_schema() {
    let json = r#"{"name": "simple_tool"}"#;
    let tool: McpToolDefinition = serde_json::from_str(json).unwrap();
    assert_eq!(tool.name, "simple_tool");
    assert!(tool.description.is_none());
    assert!(tool.input_schema.is_none());
}

#[test]
fn test_parse_tool_complex_schema() {
    let json = r#"{
        "name": "search",
        "description": "Search for items",
        "input_schema": {
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "filters": {
                    "type": "object",
                    "properties": {
                        "category": {"type": "string", "enum": ["a", "b", "c"]},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                    }
                },
                "tags": {
                    "type": "array",
                    "items": {"type": "string"}
                }
            },
            "required": ["query"]
        }
    }"#;

    let tool: McpToolDefinition = serde_json::from_str(json).unwrap();
    assert_eq!(tool.name, "search");
    let schema = tool.input_schema.unwrap();
    assert!(schema["properties"]["filters"]["properties"]["category"]["enum"].is_array());
    assert!(schema["properties"]["tags"]["items"].is_object());
}

// --- Tool name normalization tests ---

#[test]
fn test_build_mcp_tool_name_basic() {
    assert_eq!(build_mcp_tool_name("server", "tool"), "mcp__server__tool");
}

#[test]
fn test_build_mcp_tool_name_with_special_chars() {
    assert_eq!(
        build_mcp_tool_name("my-server", "my-tool"),
        "mcp__my-server__my-tool"
    );
    assert_eq!(
        build_mcp_tool_name("server.v2", "tool.name"),
        "mcp__server_v2__tool_name"
    );
}

#[test]
fn test_normalize_mcp_name_preserves_alphanum() {
    assert_eq!(normalize_mcp_name("abc123"), "abc123");
    assert_eq!(normalize_mcp_name("_already_valid_"), "_already_valid_");
}

#[test]
fn test_normalize_mcp_name_replaces_special() {
    assert_eq!(normalize_mcp_name("a-b.c d"), "a-b_c_d");
    assert_eq!(normalize_mcp_name("@scope/package"), "_scope_package");
}

// --- Manager tests with no servers ---

#[tokio::test]
async fn test_manager_no_servers() {
    let manager = McpManager::new();

    assert!(!manager.has_connections().await);
    assert_eq!(manager.connected_count().await, 0);

    let connections = manager.connections().await;
    assert!(connections.is_empty());

    let tools = manager.tool_definitions().await;
    assert!(tools.is_empty());
}

#[tokio::test]
async fn test_manager_connect_all_empty() {
    let manager = McpManager::new();
    let results = manager.connect_all(HashMap::new()).await;
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_manager_call_unknown_tool() {
    let manager = McpManager::new();
    let result = manager
        .call_tool("mcp__nonexistent__tool", serde_json::json!({"arg": "val"}))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Unknown MCP tool"));
}

#[tokio::test]
async fn test_manager_list_resources_empty() {
    let manager = McpManager::new();
    let resources = manager.list_resources().await.unwrap();
    assert!(resources.is_empty());
}

#[tokio::test]
async fn test_manager_disconnect_all_empty() {
    let manager = McpManager::new();
    // Should not panic or error
    manager.disconnect_all().await;
}

// --- MCP protocol constants tests ---

#[test]
fn test_mcp_constants() {
    assert_eq!(MCP_PROTOCOL_VERSION, "2024-11-05");
    assert_eq!(MCP_CLIENT_NAME, "claude-rs");
    assert!(!MCP_CLIENT_VERSION.is_empty());
    assert_eq!(MCP_CONNECTION_TIMEOUT_MS, 30_000);
    assert_eq!(MCP_TOOL_TIMEOUT_MS, 100_000_000);
    assert_eq!(MAX_MCP_DESCRIPTION_LENGTH, 2048);
}

#[test]
fn test_mcp_method_constants() {
    assert_eq!(methods::INITIALIZE, "initialize");
    assert_eq!(methods::INITIALIZED, "notifications/initialized");
    assert_eq!(methods::TOOLS_LIST, "tools/list");
    assert_eq!(methods::TOOLS_CALL, "tools/call");
    assert_eq!(methods::RESOURCES_LIST, "resources/list");
    assert_eq!(methods::RESOURCES_READ, "resources/read");
    assert_eq!(methods::PING, "ping");
}

// --- Server resource tests ---

#[test]
fn test_server_resource_serialization() {
    let resource = ServerResource {
        uri: "file:///tmp/test.txt".to_string(),
        name: "test.txt".to_string(),
        description: Some("A test file".to_string()),
        mime_type: Some("text/plain".to_string()),
        server: "test-server".to_string(),
    };

    let json = serde_json::to_value(&resource).unwrap();
    assert_eq!(json["uri"], "file:///tmp/test.txt");
    assert_eq!(json["name"], "test.txt");
    assert_eq!(json["description"], "A test file");
    assert_eq!(json["mime_type"], "text/plain");
    assert_eq!(json["server"], "test-server");
}

// --- Config transport_type tests ---

#[test]
fn test_config_transport_type() {
    let stdio = McpServerConfig::Stdio(McpStdioServerConfig {
        command: "test".to_string(),
        args: vec![],
        env: None,
    });
    assert_eq!(stdio.transport_type(), TransportType::Stdio);

    let sse = McpServerConfig::Sse(McpSseServerConfig {
        url: "https://example.com".to_string(),
        headers: None,
    });
    assert_eq!(sse.transport_type(), TransportType::Sse);

    let http = McpServerConfig::Http(McpHttpServerConfig {
        url: "https://example.com".to_string(),
        headers: None,
    });
    assert_eq!(http.transport_type(), TransportType::Http);
}

// --- SSE/HTTP transport config parsing tests ---

#[test]
fn test_sse_config_with_headers_roundtrip() {
    let config = McpServerConfig::Sse(McpSseServerConfig {
        url: "https://mcp.example.com/sse".to_string(),
        headers: Some({
            let mut h = HashMap::new();
            h.insert("Authorization".to_string(), "Bearer tok123".to_string());
            h.insert("X-Custom".to_string(), "value".to_string());
            h
        }),
    });

    assert_eq!(config.transport_type(), TransportType::Sse);
    if let McpServerConfig::Sse(ref sse) = config {
        assert_eq!(sse.url, "https://mcp.example.com/sse");
        let hdrs = sse.headers.as_ref().unwrap();
        assert_eq!(hdrs.get("Authorization").unwrap(), "Bearer tok123");
        assert_eq!(hdrs.get("X-Custom").unwrap(), "value");
    } else {
        panic!("Expected SSE config");
    }
}

#[test]
fn test_http_config_with_headers_roundtrip() {
    let config = McpServerConfig::Http(McpHttpServerConfig {
        url: "https://api.example.com/mcp".to_string(),
        headers: Some({
            let mut h = HashMap::new();
            h.insert("mcp-session-id".to_string(), "sess-abc".to_string());
            h
        }),
    });

    assert_eq!(config.transport_type(), TransportType::Http);
    if let McpServerConfig::Http(ref http) = config {
        assert_eq!(http.url, "https://api.example.com/mcp");
        let hdrs = http.headers.as_ref().unwrap();
        assert_eq!(hdrs.get("mcp-session-id").unwrap(), "sess-abc");
    } else {
        panic!("Expected HTTP config");
    }
}

#[test]
fn test_sse_transport_type_deserialization_from_json() {
    let json = r#"{
        "type": "sse",
        "url": "https://mcp.example.com/events"
    }"#;
    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.transport_type(), TransportType::Sse);
}

#[test]
fn test_http_transport_type_deserialization_from_json() {
    let json = r#"{
        "type": "http",
        "url": "https://mcp.example.com/rpc"
    }"#;
    let config: McpServerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.transport_type(), TransportType::Http);
}

#[test]
fn test_sse_scoped_config() {
    let json = r#"{
        "type": "sse",
        "url": "https://mcp.example.com/sse",
        "scope": "user"
    }"#;

    let scoped: ScopedMcpServerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(scoped.scope, ConfigScope::User);
    assert_eq!(scoped.config.transport_type(), TransportType::Sse);
}

#[tokio::test]
async fn test_manager_connect_sse_fail_no_server() {
    // Connecting to a non-existent SSE server should produce a Failed status
    let manager = McpManager::new();
    let mut configs = HashMap::new();
    configs.insert(
        "test-sse".to_string(),
        ScopedMcpServerConfig {
            config: McpServerConfig::Sse(McpSseServerConfig {
                url: "http://127.0.0.1:1/nonexistent".to_string(),
                headers: None,
            }),
            scope: ConfigScope::Local,
        },
    );

    let results = manager.connect_all(configs).await;
    assert_eq!(results.len(), 1);
    assert!(results[0].is_failed());
}

#[tokio::test]
async fn test_manager_connect_http_fail_no_server() {
    // Connecting to a non-existent HTTP server should produce a Failed status
    let manager = McpManager::new();
    let mut configs = HashMap::new();
    configs.insert(
        "test-http".to_string(),
        ScopedMcpServerConfig {
            config: McpServerConfig::Http(McpHttpServerConfig {
                url: "http://127.0.0.1:1/nonexistent".to_string(),
                headers: None,
            }),
            scope: ConfigScope::Local,
        },
    );

    let results = manager.connect_all(configs).await;
    assert_eq!(results.len(), 1);
    assert!(results[0].is_failed());
}
