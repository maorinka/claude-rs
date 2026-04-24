use anyhow::Result;
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::api::accumulator::ContentBlockAccumulator;
use crate::api::client::ApiClient;
use crate::api::sse::{self, SseEvent};
use crate::types::content::ContentBlock;

/// Token estimation: extract actual text content and divide by ~4 chars per token.
///
/// The TS `roughTokenCountEstimation` uses `content.length / 4` on the raw text,
/// **not** JSON serialisation length. JSON overhead (key names, braces, colons,
/// escaping) inflates the size by ~30-50% and causes over-aggressive compaction.
///
/// This implementation walks each message's content blocks and accumulates the
/// text length of text, tool_use (name + input), tool_result, and thinking blocks,
/// matching the per-block logic in `roughTokenCountEstimationForBlock`.
pub fn estimate_tokens(messages: &[Value]) -> u64 {
    let mut total_chars: u64 = 0;

    for msg in messages {
        if let Some(content) = msg.get("content") {
            total_chars += estimate_content_tokens(content);
        }
    }

    // ~4 chars per token (default ratio in the TS code)
    total_chars / 4
}

/// Estimate token count for a message's content field.
///
/// Content can be a plain string or an array of content blocks.
fn estimate_content_tokens(content: &Value) -> u64 {
    match content {
        Value::String(s) => s.len() as u64,
        Value::Array(blocks) => {
            let mut total: u64 = 0;
            for block in blocks {
                total += estimate_block_tokens(block);
            }
            total
        }
        _ => 0,
    }
}

/// Estimate token count for a single content block.
///
/// Matches the TS `roughTokenCountEstimationForBlock`:
/// - text: `block.text.length`
/// - tool_use: `(name + JSON(input)).length`
/// - tool_result: recursively count nested content
/// - thinking / redacted_thinking: text length
/// - image / document: flat 2000 tokens
/// - other: JSON stringify length
fn estimate_block_tokens(block: &Value) -> u64 {
    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match block_type {
        "text" => block
            .get("text")
            .and_then(|t| t.as_str())
            .map(|s| s.len() as u64)
            .unwrap_or(0),
        "tool_use" => {
            let name_len = block
                .get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            let input_len = block
                .get("input")
                .map(|i| serde_json::to_string(i).unwrap_or_default().len())
                .unwrap_or(0);
            (name_len + input_len) as u64
        }
        "tool_result" => {
            // Recursively count nested content
            block
                .get("content")
                .map(estimate_content_tokens)
                .unwrap_or(0)
        }
        "thinking" => block
            .get("thinking")
            .and_then(|t| t.as_str())
            .map(|s| s.len() as u64)
            .unwrap_or(0),
        "redacted_thinking" => block
            .get("data")
            .and_then(|t| t.as_str())
            .map(|s| s.len() as u64)
            .unwrap_or(0),
        "image" | "document" => {
            // Fixed estimate for images/documents (matches TS IMAGE_MAX_TOKEN_SIZE * 4)
            8000 // 2000 tokens * 4 chars/token
        }
        _ => {
            // Fallback: JSON stringify length
            serde_json::to_string(block).unwrap_or_default().len() as u64
        }
    }
}

/// Constants matching the TypeScript implementation
const AUTOCOMPACT_BUFFER_TOKENS: u64 = 13_000;
const MAX_OUTPUT_TOKENS_FOR_SUMMARY: u64 = 20_000;

/// Check if compaction is needed based on estimated token count.
pub fn should_compact(messages: &[Value], context_window: u64) -> bool {
    let estimated = estimate_tokens(messages);
    let threshold =
        context_window.saturating_sub(AUTOCOMPACT_BUFFER_TOKENS + MAX_OUTPUT_TOKENS_FOR_SUMMARY);
    estimated >= threshold
}

/// Default context window size (200K for most Claude models)
pub fn default_context_window() -> u64 {
    200_000
}

/// Compact the conversation by summarizing it via a separate API call.
/// Returns new messages array with just the summary.
pub async fn compact_conversation(
    api_client: &ApiClient,
    messages: &[Value],
    system_prompt: &[ContentBlock],
) -> Result<Vec<Value>> {
    let prompt = super::prompt::compact_prompt();

    // Build the compaction request: send the conversation + ask for summary
    // Use a separate, smaller request with no tools
    let mut compact_messages: Vec<Value> = messages.to_vec();
    compact_messages.push(json!({
        "role": "user",
        "content": [{"type": "text", "text": prompt}]
    }));

    // Make API call for summarization (streaming, no tools)
    let response = api_client
        .stream_request(&compact_messages, system_prompt, &[])
        .await?;

    // Stream SSE events from the response to extract the summary text
    let mut byte_stream = response.bytes_stream();
    let mut line_buffer = String::new();
    let mut current_event_type: Option<String> = None;
    let mut current_data: Option<String> = None;

    let mut summary = String::new();
    let mut accumulator = ContentBlockAccumulator::new();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk?;
        line_buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Process all complete lines accumulated so far
        while let Some(newline_pos) = line_buffer.find('\n') {
            let line = line_buffer[..newline_pos]
                .trim_end_matches('\r')
                .to_string();
            line_buffer = line_buffer[newline_pos + 1..].to_string();

            if let Some(rest) = line.strip_prefix("event:") {
                current_event_type = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                current_data = Some(rest.trim().to_string());
            } else if line.is_empty() {
                // Empty line marks the end of an SSE event
                if let (Some(event_type), Some(data)) =
                    (current_event_type.take(), current_data.take())
                {
                    if let Ok(event) = sse::parse_sse_event(&event_type, &data) {
                        match event {
                            SseEvent::ContentBlockStart { index, block } => {
                                accumulator.on_start(index, block);
                            }
                            SseEvent::ContentBlockDelta { index, delta } => {
                                accumulator.on_delta(index, delta);
                            }
                            SseEvent::ContentBlockStop { index } => {
                                if let Ok(ContentBlock::Text { text }) = accumulator.on_stop(index)
                                {
                                    summary.push_str(&text);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    if summary.is_empty() {
        anyhow::bail!("Compaction failed: no summary generated");
    }

    // Format summary: strip <analysis> scratchpad and extract <summary> content
    // (matches TS formatCompactSummary behavior)
    let summary = format_compact_summary(&summary);

    // Build compacted messages: just the summary as a user message
    let compact_user_msg = super::prompt::format_compact_user_message_simple(&summary);

    Ok(vec![json!({
        "role": "user",
        "content": [{"type": "text", "text": compact_user_msg}]
    })])
}

/// Format the compact summary by stripping the `<analysis>` drafting scratchpad
/// and replacing `<summary>` XML tags with readable section headers.
/// Matches the TS `formatCompactSummary()` in prompt.ts.
fn format_compact_summary(text: &str) -> String {
    let mut result = text.to_string();

    // Strip analysis section -- it's a drafting scratchpad that improves summary
    // quality but has no informational value once the summary is written.
    if let Some(start) = result.find("<analysis>") {
        if let Some(end) = result.find("</analysis>") {
            let before = &result[..start];
            let after = &result[end + "</analysis>".len()..];
            result = format!("{}{}", before, after);
        }
    }

    // Extract and format summary section
    if let Some(start) = result.find("<summary>") {
        if let Some(end) = result.find("</summary>") {
            let content = &result[start + "<summary>".len()..end];
            let before = &result[..start];
            let after = &result[end + "</summary>".len()..];
            result = format!("{}Summary:\n{}{}", before, content.trim(), after);
        }
    }

    // Clean up extra whitespace between sections
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_compact_summary_strips_analysis() {
        let input =
            "<analysis>scratchpad stuff</analysis>\n\n<summary>\nThe actual summary\n</summary>";
        let result = format_compact_summary(input);
        assert_eq!(result, "Summary:\nThe actual summary");
    }

    #[test]
    fn test_format_compact_summary_no_tags() {
        let input = "no analysis or summary tags here";
        let result = format_compact_summary(input);
        assert_eq!(result, "no analysis or summary tags here");
    }

    #[test]
    fn test_format_compact_summary_unclosed_analysis() {
        let input = "before <analysis>no closing tag";
        let result = format_compact_summary(input);
        assert_eq!(result, "before <analysis>no closing tag");
    }

    #[test]
    fn test_format_compact_summary_summary_only() {
        let input = "<summary>\n1. Primary Request\n2. Key Concepts\n</summary>";
        let result = format_compact_summary(input);
        assert_eq!(result, "Summary:\n1. Primary Request\n2. Key Concepts");
    }

    #[test]
    fn test_estimate_tokens_string_content() {
        let messages = vec![json!({"role": "user", "content": "hello world"})];
        let tokens = estimate_tokens(&messages);
        // "hello world" = 11 chars, tokens = 11 / 4 = 2
        assert_eq!(tokens, 2);
    }

    #[test]
    fn test_estimate_tokens_text_block() {
        let messages = vec![json!({
            "role": "user",
            "content": [{"type": "text", "text": "hello world, this is a longer message"}]
        })];
        let tokens = estimate_tokens(&messages);
        // 37 chars / 4 = 9
        assert_eq!(tokens, 9);
    }

    #[test]
    fn test_estimate_tokens_tool_use_block() {
        let messages = vec![json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "tu_1",
                "name": "Read",
                "input": {"file_path": "/some/file.rs"}
            }]
        })];
        let tokens = estimate_tokens(&messages);
        // "Read" = 4 chars + JSON({"file_path":"/some/file.rs"}) chars
        assert!(tokens > 0);
        // Should be much less than full JSON serialization of the whole message
        let json_estimation = serde_json::to_string(&messages).unwrap().len() as u64 / 4;
        assert!(tokens < json_estimation);
    }

    #[test]
    fn test_estimate_tokens_tool_result_block() {
        let messages = vec![json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "tu_1",
                "content": [{"type": "text", "text": "file contents here"}]
            }]
        })];
        let tokens = estimate_tokens(&messages);
        // "file contents here" = 18 chars / 4 = 4
        assert_eq!(tokens, 4);
    }

    #[test]
    fn test_estimate_tokens_image_block() {
        let messages = vec![json!({
            "role": "user",
            "content": [{"type": "image", "source": {"type": "base64", "data": "huge_base64_data"}}]
        })];
        let tokens = estimate_tokens(&messages);
        // Image blocks get fixed estimate of 8000 chars / 4 = 2000 tokens
        assert_eq!(tokens, 2000);
    }

    #[test]
    fn test_estimate_tokens_excludes_json_overhead() {
        // This test verifies that our improved estimation uses text content length,
        // not JSON serialization length (which would be ~2x larger due to key names,
        // braces, colons, etc.)
        let text = "a".repeat(400); // 400 chars = 100 tokens
        let messages = vec![json!({
            "role": "user",
            "content": [{"type": "text", "text": text}]
        })];
        let tokens = estimate_tokens(&messages);
        assert_eq!(tokens, 100);
    }
}
