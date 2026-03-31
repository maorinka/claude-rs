use anyhow::Result;
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::api::accumulator::ContentBlockAccumulator;
use crate::api::sse::{self, SseEvent};
use crate::api::client::ApiClient;
use crate::types::content::ContentBlock;

/// Token estimation: ~4 chars per token (rough but matches TS)
fn estimate_tokens(messages: &[Value]) -> u64 {
    let json_str = serde_json::to_string(messages).unwrap_or_default();
    (json_str.len() as u64) / 4
}

/// Constants matching the TypeScript implementation
const AUTOCOMPACT_BUFFER_TOKENS: u64 = 13_000;
const MAX_OUTPUT_TOKENS_FOR_SUMMARY: u64 = 20_000;

/// Check if compaction is needed based on estimated token count.
pub fn should_compact(messages: &[Value], context_window: u64) -> bool {
    let estimated = estimate_tokens(messages);
    let threshold = context_window.saturating_sub(AUTOCOMPACT_BUFFER_TOKENS + MAX_OUTPUT_TOKENS_FOR_SUMMARY);
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
                                if let Ok(block) = accumulator.on_stop(index) {
                                    if let ContentBlock::Text { text } = block {
                                        summary.push_str(&text);
                                    }
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
    let compact_user_msg = super::prompt::format_compact_user_message(&summary);

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
        let input = "<analysis>scratchpad stuff</analysis>\n\n<summary>\nThe actual summary\n</summary>";
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
    fn test_estimate_tokens() {
        let messages = vec![json!({"role": "user", "content": "hello world"})];
        let tokens = estimate_tokens(&messages);
        // Should be roughly json_length / 4
        assert!(tokens > 0);
    }
}
