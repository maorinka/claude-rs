use claude_core::compact::compactor::*;
use claude_core::compact::prompt::*;
use serde_json::json;

#[test]
fn test_should_compact_below_threshold() {
    let messages = vec![json!({"role": "user", "content": [{"type": "text", "text": "hello"}]})];
    assert!(!should_compact(&messages, 200_000));
}

#[test]
fn test_should_compact_above_threshold() {
    // Create a message that's very large
    let big_text = "x".repeat(800_000); // ~200K tokens
    let messages = vec![json!({"role": "user", "content": [{"type": "text", "text": big_text}]})];
    assert!(should_compact(&messages, 200_000));
}

#[test]
fn test_compact_prompt_not_empty() {
    let prompt = compact_prompt();
    assert!(prompt.contains("summary"));
    assert!(prompt.contains("Do NOT call any tools"));
    // Verify the <analysis>/<summary> XML structure instructions
    assert!(prompt.contains("<analysis>"));
    assert!(prompt.contains("<summary>"));
    assert!(prompt.contains("REMINDER: Do NOT call any tools"));
}

#[test]
fn test_format_compact_user_message() {
    let msg = format_compact_user_message_simple("Test summary content");
    assert!(msg.contains("Test summary content"));
    assert!(msg.contains("continued from a previous conversation"));
}

#[test]
fn test_format_compact_user_message_with_options() {
    let msg = format_compact_user_message(&CompactUserMessageOptions {
        summary: "Test summary",
        transcript_path: Some("/tmp/transcript.jsonl"),
        recent_messages_preserved: true,
        suppress_follow_up_questions: true,
        proactive_mode: true,
    });
    assert!(msg.contains("Test summary"));
    assert!(msg.contains("/tmp/transcript.jsonl"));
    assert!(msg.contains("Recent messages are preserved verbatim"));
    assert!(msg.contains("Resume directly"));
    assert!(msg.contains("autonomous/proactive mode"));
}

#[test]
fn test_compact_prompt_structure() {
    // Verify the prompt starts with NO_TOOLS_PREAMBLE and ends with NO_TOOLS_TRAILER
    let prompt = compact_prompt();
    assert!(prompt.starts_with("CRITICAL: Respond with TEXT ONLY."));
    assert!(prompt.ends_with("Tool calls will be rejected and you will fail the task."));
    assert!(prompt.len() > 100);
}

#[test]
fn test_compact_prompt_includes_custom_instructions_before_trailer() {
    let prompt = compact_prompt_with_instructions(Some("preserve hook output"));
    let instructions = prompt
        .find("Additional Instructions:\npreserve hook output")
        .unwrap();
    let trailer = prompt.find("REMINDER: Do NOT call any tools").unwrap();
    assert!(instructions < trailer);
}

#[test]
fn test_partial_compact_prompt_omits_blank_custom_instructions() {
    let prompt = partial_compact_prompt_with_instructions(Some("  \n "));
    assert!(!prompt.contains("Additional Instructions:"));
}

#[test]
fn test_default_context_window() {
    assert_eq!(default_context_window(), 200_000);
}
