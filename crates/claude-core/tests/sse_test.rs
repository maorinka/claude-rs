use claude_core::api::sse::*;

#[test]
fn test_parse_message_start_event() {
    let data = r#"{"type":"message_start","message":{"id":"msg_01","model":"claude-sonnet-4-6","role":"assistant","content":[],"stop_reason":null,"usage":{"input_tokens":100,"output_tokens":0}}}"#;
    let event = parse_sse_event("message_start", data).unwrap();
    match event {
        SseEvent::MessageStart { message } => {
            assert_eq!(message.id, "msg_01");
            assert_eq!(message.usage.input_tokens, 100);
        }
        _ => panic!("Expected MessageStart"),
    }
}

#[test]
fn test_parse_content_block_start_text() {
    let data =
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
    let event = parse_sse_event("content_block_start", data).unwrap();
    match event {
        SseEvent::ContentBlockStart { index, block } => {
            assert_eq!(index, 0);
            assert!(matches!(block, ContentBlockStart::Text));
        }
        _ => panic!("Expected ContentBlockStart"),
    }
}

#[test]
fn test_parse_content_block_start_tool_use() {
    let data = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"Bash","input":""}}"#;
    let event = parse_sse_event("content_block_start", data).unwrap();
    match event {
        SseEvent::ContentBlockStart { index, block } => {
            assert_eq!(index, 1);
            match block {
                ContentBlockStart::ToolUse { id, name } => {
                    assert_eq!(id, "tu_1");
                    assert_eq!(name, "Bash");
                }
                _ => panic!("Expected ToolUse"),
            }
        }
        _ => panic!("Expected ContentBlockStart"),
    }
}

#[test]
fn test_parse_text_delta() {
    let data =
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
    let event = parse_sse_event("content_block_delta", data).unwrap();
    match event {
        SseEvent::ContentBlockDelta { index, delta } => {
            assert_eq!(index, 0);
            match delta {
                ContentDelta::TextDelta { text } => assert_eq!(text, "Hello"),
                _ => panic!("Expected TextDelta"),
            }
        }
        _ => panic!("Expected ContentBlockDelta"),
    }
}

#[test]
fn test_parse_input_json_delta() {
    let data = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\""}}"#;
    let event = parse_sse_event("content_block_delta", data).unwrap();
    match event {
        SseEvent::ContentBlockDelta { index, delta } => {
            assert_eq!(index, 1);
            match delta {
                ContentDelta::InputJsonDelta { partial_json } => {
                    assert_eq!(partial_json, r#"{"command""#);
                }
                _ => panic!("Expected InputJsonDelta"),
            }
        }
        _ => panic!("Expected ContentBlockDelta"),
    }
}

#[test]
fn test_parse_content_block_stop() {
    let data = r#"{"type":"content_block_stop","index":0}"#;
    let event = parse_sse_event("content_block_stop", data).unwrap();
    match event {
        SseEvent::ContentBlockStop { index } => assert_eq!(index, 0),
        _ => panic!("Expected ContentBlockStop"),
    }
}

#[test]
fn test_parse_message_delta() {
    let data = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;
    let event = parse_sse_event("message_delta", data).unwrap();
    match event {
        SseEvent::MessageDelta { stop_reason, usage } => {
            assert_eq!(stop_reason.as_deref(), Some("end_turn"));
            assert_eq!(usage.as_ref().unwrap().output_tokens, 42);
        }
        _ => panic!("Expected MessageDelta"),
    }
}

#[test]
fn test_parse_message_stop() {
    let data = r#"{"type":"message_stop"}"#;
    let event = parse_sse_event("message_stop", data).unwrap();
    assert!(matches!(event, SseEvent::MessageStop));
}

#[test]
fn test_parse_sse_lines() {
    let raw = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"model\":\"claude-sonnet-4-6\",\"role\":\"assistant\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n";
    let events = parse_sse_stream(raw);
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], SseEvent::MessageStart { .. }));
}

#[test]
fn test_parse_multiple_sse_events() {
    let raw = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"model\":\"claude-sonnet-4-6\",\"role\":\"assistant\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: ping\ndata: {}\n\n";
    let events = parse_sse_stream(raw);
    assert_eq!(events.len(), 3);
    assert!(matches!(events[0], SseEvent::MessageStart { .. }));
    assert!(matches!(events[1], SseEvent::ContentBlockStart { .. }));
    assert!(matches!(events[2], SseEvent::Ping));
}
