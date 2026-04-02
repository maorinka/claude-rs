use claude_core::api::accumulator::*;
use claude_core::api::sse::*;
use claude_core::types::content::ContentBlock;

#[test]
fn test_accumulate_text_block() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(0, ContentBlockStart::Text);
    acc.on_delta(
        0,
        ContentDelta::TextDelta {
            text: "Hello".into(),
        },
    );
    acc.on_delta(
        0,
        ContentDelta::TextDelta {
            text: " world".into(),
        },
    );
    let block = acc.on_stop(0).unwrap();
    match block {
        ContentBlock::Text { text } => assert_eq!(text, "Hello world"),
        _ => panic!("Expected Text"),
    }
}

#[test]
fn test_accumulate_tool_use_block() {
    let mut acc = ContentBlockAccumulator::new();
    acc.on_start(
        0,
        ContentBlockStart::ToolUse {
            id: "tu_1".into(),
            name: "Bash".into(),
        },
    );
    acc.on_delta(
        0,
        ContentDelta::InputJsonDelta {
            partial_json: r#"{"command""#.into(),
        },
    );
    acc.on_delta(
        0,
        ContentDelta::InputJsonDelta {
            partial_json: r#": "ls -la"}"#.into(),
        },
    );
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
    acc.on_delta(
        0,
        ContentDelta::ThinkingDelta {
            thinking: "Let me think...".into(),
        },
    );
    acc.on_delta(
        0,
        ContentDelta::SignatureDelta {
            signature: "sig_abc".into(),
        },
    );
    let block = acc.on_stop(0).unwrap();
    match block {
        ContentBlock::Thinking {
            thinking,
            signature,
        } => {
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
    acc.on_start(
        1,
        ContentBlockStart::ToolUse {
            id: "tu_1".into(),
            name: "Read".into(),
        },
    );
    acc.on_delta(
        0,
        ContentDelta::TextDelta {
            text: "Let me read that.".into(),
        },
    );
    acc.on_delta(
        1,
        ContentDelta::InputJsonDelta {
            partial_json: r#"{"file_path": "/tmp/x"}"#.into(),
        },
    );
    let b0 = acc.on_stop(0).unwrap();
    let b1 = acc.on_stop(1).unwrap();
    assert!(matches!(b0, ContentBlock::Text { .. }));
    assert!(matches!(b1, ContentBlock::ToolUse { .. }));
}
