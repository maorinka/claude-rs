use claude_core::api::normalize::*;
use serde_json::json;

#[test]
fn test_normalize_filters_system_messages() {
    let messages = vec![
        json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
        json!({"role": "system", "content": [{"type": "text", "text": "system msg"}]}),
        json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
    ];
    let result = normalize_messages(&messages);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0]["role"], "user");
    assert_eq!(result[1]["role"], "assistant");
}

#[test]
fn test_normalize_filters_empty_content() {
    let messages = vec![
        json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
        json!({"role": "assistant", "content": []}),
    ];
    let result = normalize_messages(&messages);
    assert_eq!(result.len(), 1);
}

#[test]
fn test_repair_orphaned_tool_use() {
    let messages = vec![
        json!({"role": "user", "content": [{"type": "text", "text": "do something"}]}),
        json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "tu_1", "name": "Bash", "input": {"command": "ls"}}
        ]}),
        // Missing tool_result for tu_1
    ];
    let result = normalize_messages(&messages);
    // Should have synthetic error tool_result appended
    assert!(result.len() >= 3);
    let last = result.last().unwrap();
    assert_eq!(last["role"], "user");
    let content = last["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "tool_result");
    assert_eq!(content[0]["tool_use_id"], "tu_1");
    assert_eq!(content[0]["is_error"], true);
}

#[test]
fn test_paired_tool_use_not_repaired() {
    let messages = vec![
        json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "tu_1", "name": "Read", "input": {}}
        ]}),
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "tu_1", "content": [{"type": "text", "text": "ok"}]}
        ]}),
    ];
    let result = normalize_messages(&messages);
    assert_eq!(result.len(), 2); // No synthetic message added
}

#[test]
fn test_add_cache_markers() {
    let mut messages = vec![
        json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
        json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
    ];
    add_cache_markers(&mut messages);
    // Last message's last block should have cache_control
    let last_content = messages[1]["content"].as_array().unwrap();
    assert!(last_content[0].get("cache_control").is_some());
}

#[test]
fn test_cache_markers_skip_thinking() {
    let mut messages = vec![json!({"role": "assistant", "content": [
        {"type": "text", "text": "answer"},
        {"type": "thinking", "thinking": "hmm", "signature": "sig"}
    ]})];
    add_cache_markers(&mut messages);
    // Should NOT add cache_control to thinking block
    let content = messages[0]["content"].as_array().unwrap();
    assert!(content[1].get("cache_control").is_none());
}
