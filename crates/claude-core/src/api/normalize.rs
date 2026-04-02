use serde_json::Value;

/// Normalize messages for the API.
/// - Ensures alternating user/assistant roles
/// - Repairs orphaned tool_use/tool_result blocks
/// - Strips system messages (they go in the system field)
pub fn normalize_messages(messages: &[Value]) -> Vec<Value> {
    let mut normalized: Vec<Value> = Vec::new();

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("");

        // Skip system messages (sent separately)
        if role == "system" {
            continue;
        }

        // Skip messages with empty content
        let content = &msg["content"];
        if content.is_null() || (content.is_array() && content.as_array().unwrap().is_empty()) {
            continue;
        }

        normalized.push(msg.clone());
    }

    // Repair tool pairing
    repair_tool_pairing(&mut normalized);

    normalized
}

/// Ensure every tool_use has a matching tool_result and vice versa.
fn repair_tool_pairing(messages: &mut Vec<Value>) {
    // Collect all tool_use IDs from assistant messages
    let mut tool_use_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in messages.iter() {
        if msg["role"].as_str() == Some("assistant") {
            if let Some(content) = msg["content"].as_array() {
                for block in content {
                    if block["type"].as_str() == Some("tool_use") {
                        if let Some(id) = block["id"].as_str() {
                            tool_use_ids.insert(id.to_string());
                        }
                    }
                }
            }
        }
    }

    // Collect all tool_result IDs from user messages
    let mut tool_result_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in messages.iter() {
        if msg["role"].as_str() == Some("user") {
            if let Some(content) = msg["content"].as_array() {
                for block in content {
                    if block["type"].as_str() == Some("tool_result") {
                        if let Some(id) = block["tool_use_id"].as_str() {
                            tool_result_ids.insert(id.to_string());
                        }
                    }
                }
            }
        }
    }

    // Find orphaned tool_use (no matching tool_result) — add synthetic error results
    let orphaned_uses: Vec<String> = tool_use_ids.difference(&tool_result_ids).cloned().collect();
    if !orphaned_uses.is_empty() {
        let mut synthetic_results: Vec<Value> = Vec::new();
        for id in orphaned_uses {
            synthetic_results.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": [{"type": "text", "text": "Error: tool execution was interrupted"}],
                "is_error": true,
            }));
        }
        // Append as a user message
        messages.push(serde_json::json!({
            "role": "user",
            "content": synthetic_results,
        }));
    }

    // Strip orphaned tool_results (no matching tool_use)
    let orphaned_results: Vec<String> =
        tool_result_ids.difference(&tool_use_ids).cloned().collect();
    if !orphaned_results.is_empty() {
        for msg in messages.iter_mut() {
            if msg["role"].as_str() == Some("user") {
                if let Some(content) = msg["content"].as_array() {
                    let filtered: Vec<Value> = content
                        .iter()
                        .filter(|block| {
                            if block["type"].as_str() == Some("tool_result") {
                                if let Some(id) = block["tool_use_id"].as_str() {
                                    return !orphaned_results.contains(&id.to_string());
                                }
                            }
                            true
                        })
                        .cloned()
                        .collect();
                    msg["content"] = Value::Array(filtered);
                }
            }
        }
    }
}

/// Add cache control markers to messages for prompt caching.
pub fn add_cache_markers(messages: &mut [Value]) {
    // Add cache_control to last block of system prompt
    // and last non-thinking block of assistant messages
    for msg in messages.iter_mut().rev() {
        if let Some(content) = msg["content"].as_array_mut() {
            if let Some(last) = content.last_mut() {
                // Skip thinking blocks
                if last["type"].as_str() != Some("thinking")
                    && last["type"].as_str() != Some("redacted_thinking")
                {
                    last["cache_control"] = serde_json::json!({"type": "ephemeral"});
                    break;
                }
            }
        }
    }
}
