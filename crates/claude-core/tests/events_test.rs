use claude_core::types::events::*;
use claude_core::types::message::StopReason;

#[test]
fn test_stream_event_text_delta() {
    let event = StreamEvent::TextDelta {
        text: "Hello".into(),
    };
    match event {
        StreamEvent::TextDelta { text } => assert_eq!(text, "Hello"),
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_stream_event_tool_start() {
    let event = StreamEvent::ToolStart {
        tool_use_id: "tu_1".into(),
        name: "Bash".into(),
        input: serde_json::json!({"command": "ls"}),
    };
    match event {
        StreamEvent::ToolStart { name, .. } => assert_eq!(name, "Bash"),
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_stream_event_done() {
    let event = StreamEvent::Done {
        stop_reason: StopReason::EndTurn,
    };
    match event {
        StreamEvent::Done { stop_reason } => assert_eq!(stop_reason, StopReason::EndTurn),
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_tool_progress_bash() {
    let progress = ToolProgressData::BashProgress {
        stdout: "output".into(),
        stderr: "".into(),
    };
    match progress {
        ToolProgressData::BashProgress { stdout, .. } => assert_eq!(stdout, "output"),
        _ => panic!("Wrong variant"),
    }
}
