use chrono::Utc;
use claude_core::types::content::*;
use claude_core::types::message::*;
use claude_core::types::usage::*;
use uuid::Uuid;

#[test]
fn test_user_message_serializes_with_type_tag() {
    let msg = Message::User(UserMessage {
        uuid: Uuid::nil(),
        content: vec![ContentBlock::Text {
            text: "hello".into(),
        }],
        timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-31T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    });
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "user");
    assert_eq!(json["content"][0]["type"], "text");
    assert_eq!(json["content"][0]["text"], "hello");
}

#[test]
fn test_assistant_message_with_tool_use() {
    let msg = Message::Assistant(AssistantMessage {
        uuid: Uuid::nil(),
        message: ApiMessage {
            id: "msg_123".into(),
            model: "claude-sonnet-4-6".into(),
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "Let me read that file.".into(),
                },
                ContentBlock::ToolUse {
                    id: "tu_1".into(),
                    name: "Read".into(),
                    input: serde_json::json!({"file_path": "/tmp/test.rs"}),
                },
            ],
            stop_reason: Some(StopReason::ToolUse),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        },
        request_id: Some("req_abc".into()),
        timestamp: Utc::now(),
    });
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["type"], "assistant");
    assert_eq!(json["message"]["content"][1]["type"], "tool_use");
    assert_eq!(json["message"]["content"][1]["name"], "Read");
    assert_eq!(json["message"]["stop_reason"], "tool_use");
}

#[test]
fn test_tool_result_content_block() {
    let block = ContentBlock::ToolResult {
        tool_use_id: "tu_1".into(),
        content: vec![ContentBlock::Text {
            text: "file contents here".into(),
        }],
        is_error: Some(false),
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "tool_result");
    assert_eq!(json["tool_use_id"], "tu_1");
    assert_eq!(json["is_error"], false);
}

#[test]
fn test_thinking_content_block() {
    let block = ContentBlock::Thinking {
        thinking: "I need to consider...".into(),
        signature: "sig_abc123".into(),
    };
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "thinking");
    assert_eq!(json["thinking"], "I need to consider...");
    assert_eq!(json["signature"], "sig_abc123");
}

#[test]
fn test_usage_default_optional_fields() {
    let usage = Usage {
        input_tokens: 500,
        output_tokens: 200,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: Some(100),
    };
    let json = serde_json::to_value(&usage).unwrap();
    assert_eq!(json["input_tokens"], 500);
    assert!(
        json.get("cache_creation_input_tokens").is_none()
            || json["cache_creation_input_tokens"].is_null()
    );
    assert_eq!(json["cache_read_input_tokens"], 100);
}

#[test]
fn test_deserialize_api_response() {
    let raw = r#"{
        "id": "msg_01",
        "model": "claude-sonnet-4-6",
        "role": "assistant",
        "content": [
            {"type": "text", "text": "Hello!"}
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 10,
            "output_tokens": 5
        }
    }"#;
    let msg: ApiMessage = serde_json::from_str(raw).unwrap();
    assert_eq!(msg.id, "msg_01");
    assert_eq!(msg.stop_reason, Some(StopReason::EndTurn));
    assert_eq!(msg.usage.input_tokens, 10);
}
