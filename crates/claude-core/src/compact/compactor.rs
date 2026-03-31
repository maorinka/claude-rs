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

    // Strip <analysis> block if present (matches TS behavior)
    let summary = strip_analysis_block(&summary);

    // Build compacted messages: just the summary as a user message
    let compact_user_msg = super::prompt::format_compact_user_message(&summary);

    Ok(vec![json!({
        "role": "user",
        "content": [{"type": "text", "text": compact_user_msg}]
    })])
}

/// Strip the `<analysis>...</analysis>` block from the summary.
/// The TS code uses this as a scratchpad that improves summary quality
/// but adds no value to the saved context.
fn strip_analysis_block(text: &str) -> String {
    if let Some(start) = text.find("<analysis>") {
        if let Some(end) = text.find("</analysis>") {
            let before = &text[..start];
            let after = &text[end + "</analysis>".len()..];
            return format!("{}{}", before.trim(), after.trim());
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_analysis_block_present() {
        let input = "before <analysis>scratchpad stuff</analysis> after";
        let result = strip_analysis_block(input);
        assert_eq!(result, "beforeafter");
    }

    #[test]
    fn test_strip_analysis_block_absent() {
        let input = "no analysis block here";
        let result = strip_analysis_block(input);
        assert_eq!(result, "no analysis block here");
    }

    #[test]
    fn test_strip_analysis_block_unclosed() {
        let input = "before <analysis>no closing tag";
        let result = strip_analysis_block(input);
        assert_eq!(result, "before <analysis>no closing tag");
    }

    #[test]
    fn test_estimate_tokens() {
        let messages = vec![json!({"role": "user", "content": "hello world"})];
        let tokens = estimate_tokens(&messages);
        // Should be roughly json_length / 4
        assert!(tokens > 0);
    }
}
