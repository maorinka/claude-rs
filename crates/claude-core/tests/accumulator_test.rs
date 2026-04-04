use claude_core::api::accumulator::*;
use claude_core::api::sse::*;
use claude_core::types::content::ContentBlock;

#[test]
fn test_accumulate_text_block() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::Text);
    acc.on_delta(0, ContentDelta::TextDelta { text: "Hello".into() });
    acc.on_delta(0, ContentDelta::TextDelta { text: " world".into() });
    let block = acc.on_stop(0).unwrap();
    match block {
        ContentBlock::Text { text } => assert_eq!(text, "Hello world"),
        _ => panic!("Expected Text"),
    }
}

#[test]
fn test_accumulate_tool_use_block() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::ToolUse { id: "tu_1".into(), name: "Bash".into() });
    acc.on_delta(0, ContentDelta::InputJsonDelta { partial_json: r#"{"command""#.into() });
    acc.on_delta(0, ContentDelta::InputJsonDelta { partial_json: r#": "ls -la"}"#.into() });
    let block = acc.on_stop(0).unwrap();
    match block {
        ContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "tu_1");
            assert_eq!(name, "Bash");
            assert_eq!(input["command"], "ls -la");
        }
        _ => panic!("Expected ToolUse"),
    }
}

#[test]
fn test_accumulate_thinking_block() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::Thinking);
    acc.on_delta(0, ContentDelta::ThinkingDelta { thinking: "Let me think...".into() });
    acc.on_delta(0, ContentDelta::SignatureDelta { signature: "sig_abc".into() });
    let block = acc.on_stop(0).unwrap();
    match block {
        ContentBlock::Thinking { thinking, signature } => {
            assert_eq!(thinking, "Let me think...");
            assert_eq!(signature, "sig_abc");
        }
        _ => panic!("Expected Thinking"),
    }
}

#[test]
fn test_accumulate_multiple_blocks() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::Text);
    acc.on_start(1, ContentBlockStart::ToolUse { id: "tu_1".into(), name: "Read".into() });
    acc.on_delta(0, ContentDelta::TextDelta { text: "Let me read that.".into() });
    acc.on_delta(1, ContentDelta::InputJsonDelta { partial_json: r#"{"file_path": "/tmp/x"}"#.into() });
    let b0 = acc.on_stop(0).unwrap();
    let b1 = acc.on_stop(1).unwrap();
    assert!(matches!(b0, ContentBlock::Text { .. }));
    assert!(matches!(b1, ContentBlock::ToolUse { .. }));
}

/// Bug #21: Non-object JSON types should be rejected as tool input.
#[test]
fn test_tool_input_array_rejected() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::ToolUse { id: "tu_1".into(), name: "Bash".into() });
    acc.on_delta(0, ContentDelta::InputJsonDelta { partial_json: r#"[1, 2, 3]"#.into() });
    let result = acc.on_stop(0);
    assert!(result.is_err(), "Array JSON should be rejected as tool input");
    assert!(
        result.unwrap_err().to_string().contains("Tool input must be a JSON object"),
        "Error message should mention JSON object requirement"
    );
}

/// Bug #21: String JSON should be rejected as tool input.
#[test]
fn test_tool_input_string_rejected() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::ToolUse { id: "tu_1".into(), name: "Bash".into() });
    acc.on_delta(0, ContentDelta::InputJsonDelta { partial_json: r#""just a string""#.into() });
    let result = acc.on_stop(0);
    assert!(result.is_err(), "String JSON should be rejected as tool input");
}

/// Bug #21: Number JSON should be rejected as tool input.
#[test]
fn test_tool_input_number_rejected() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::ToolUse { id: "tu_1".into(), name: "Bash".into() });
    acc.on_delta(0, ContentDelta::InputJsonDelta { partial_json: "42".into() });
    let result = acc.on_stop(0);
    assert!(result.is_err(), "Number JSON should be rejected as tool input");
}

/// Bug #21: Null JSON should be rejected as tool input.
#[test]
fn test_tool_input_null_rejected() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::ToolUse { id: "tu_1".into(), name: "Bash".into() });
    acc.on_delta(0, ContentDelta::InputJsonDelta { partial_json: "null".into() });
    let result = acc.on_stop(0);
    assert!(result.is_err(), "Null JSON should be rejected as tool input");
}

/// Bug #21: Valid object JSON should still be accepted.
#[test]
fn test_tool_input_object_accepted() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::ToolUse { id: "tu_1".into(), name: "Bash".into() });
    acc.on_delta(0, ContentDelta::InputJsonDelta { partial_json: r#"{"key": "value"}"#.into() });
    let result = acc.on_stop(0);
    assert!(result.is_ok(), "Object JSON should be accepted as tool input");
}

/// Bug #21: Empty tool input should default to empty object.
#[test]
fn test_tool_input_empty_defaults_to_object() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::ToolUse { id: "tu_1".into(), name: "Bash".into() });
    // No deltas — empty input_json
    let result = acc.on_stop(0);
    assert!(result.is_ok(), "Empty input should default to empty object");
    match result.unwrap() {
        ContentBlock::ToolUse { input, .. } => {
            assert!(input.is_object(), "Default input should be an object");
            assert!(input.as_object().unwrap().is_empty(), "Default input should be empty object");
        }
        _ => panic!("Expected ToolUse block"),
    }
}
