use crate::types::message::ApiMessage;
use anyhow::{anyhow, Result};
use serde::Deserialize;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SseEvent {
    MessageStart {
        message: ApiMessage,
    },
    ContentBlockStart {
        index: usize,
        block: ContentBlockStart,
    },
    ContentBlockDelta {
        index: usize,
        delta: ContentDelta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        stop_reason: Option<String>,
        usage: Option<DeltaUsage>,
    },
    MessageStop,
    Ping,
    Error {
        message: String,
    },
}

#[derive(Debug)]
pub enum ContentBlockStart {
    Text,
    ToolUse { id: String, name: String },
    Thinking,
    ServerToolUse { id: String, name: String },
}

#[derive(Debug)]
pub enum ContentDelta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
    ThinkingDelta { thinking: String },
    SignatureDelta { signature: String },
}

#[derive(Debug, Deserialize)]
pub struct DeltaUsage {
    pub output_tokens: u64,
}

// ── Internal deserialization helpers ─────────────────────────────────────────

#[derive(Deserialize)]
struct MessageStartPayload {
    message: ApiMessage,
}

#[derive(Deserialize)]
struct ContentBlockStartPayload {
    index: usize,
    content_block: RawContentBlock,
}

#[derive(Deserialize)]
struct RawContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    id: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize)]
struct ContentBlockDeltaPayload {
    index: usize,
    delta: RawDelta,
}

#[derive(Deserialize)]
struct RawDelta {
    #[serde(rename = "type")]
    delta_type: String,
    text: Option<String>,
    partial_json: Option<String>,
    thinking: Option<String>,
    signature: Option<String>,
}

#[derive(Deserialize)]
struct ContentBlockStopPayload {
    index: usize,
}

#[derive(Deserialize)]
struct MessageDeltaPayload {
    delta: MessageDeltaInner,
    #[serde(default)]
    usage: Option<DeltaUsage>,
}

#[derive(Deserialize)]
struct MessageDeltaInner {
    stop_reason: Option<String>,
}

// ── Public functions ──────────────────────────────────────────────────────────

/// Parse a single SSE event given its event-type string and JSON data payload.
pub fn parse_sse_event(event_type: &str, data: &str) -> Result<SseEvent> {
    match event_type {
        "message_start" => {
            let p: MessageStartPayload = serde_json::from_str(data)?;
            Ok(SseEvent::MessageStart { message: p.message })
        }

        "content_block_start" => {
            let p: ContentBlockStartPayload = serde_json::from_str(data)?;
            let block = match p.content_block.block_type.as_str() {
                "text" => ContentBlockStart::Text,
                "tool_use" => ContentBlockStart::ToolUse {
                    id: p.content_block.id.unwrap_or_default(),
                    name: p.content_block.name.unwrap_or_default(),
                },
                "thinking" => ContentBlockStart::Thinking,
                "server_tool_use" => ContentBlockStart::ServerToolUse {
                    id: p.content_block.id.unwrap_or_default(),
                    name: p.content_block.name.unwrap_or_default(),
                },
                other => return Err(anyhow!("Unknown content_block type: {}", other)),
            };
            Ok(SseEvent::ContentBlockStart {
                index: p.index,
                block,
            })
        }

        "content_block_delta" => {
            let p: ContentBlockDeltaPayload = serde_json::from_str(data)?;
            let delta = match p.delta.delta_type.as_str() {
                "text_delta" => ContentDelta::TextDelta {
                    text: p.delta.text.unwrap_or_default(),
                },
                "input_json_delta" => ContentDelta::InputJsonDelta {
                    partial_json: p.delta.partial_json.unwrap_or_default(),
                },
                "thinking_delta" => ContentDelta::ThinkingDelta {
                    thinking: p.delta.thinking.unwrap_or_default(),
                },
                "signature_delta" => ContentDelta::SignatureDelta {
                    signature: p.delta.signature.unwrap_or_default(),
                },
                other => return Err(anyhow!("Unknown delta type: {}", other)),
            };
            Ok(SseEvent::ContentBlockDelta {
                index: p.index,
                delta,
            })
        }

        "content_block_stop" => {
            let p: ContentBlockStopPayload = serde_json::from_str(data)?;
            Ok(SseEvent::ContentBlockStop { index: p.index })
        }

        "message_delta" => {
            let p: MessageDeltaPayload = serde_json::from_str(data)?;
            Ok(SseEvent::MessageDelta {
                stop_reason: p.delta.stop_reason,
                usage: p.usage,
            })
        }

        "message_stop" => Ok(SseEvent::MessageStop),

        "ping" => Ok(SseEvent::Ping),

        "error" => {
            #[derive(Deserialize)]
            struct ErrorPayload {
                #[serde(default)]
                error: Option<ErrorInner>,
            }
            #[derive(Deserialize)]
            struct ErrorInner {
                message: Option<String>,
            }
            let message = serde_json::from_str::<ErrorPayload>(data)
                .ok()
                .and_then(|p| p.error)
                .and_then(|e| e.message)
                .unwrap_or_else(|| data.to_string());
            Ok(SseEvent::Error { message })
        }

        other => Err(anyhow!("Unknown SSE event type: {}", other)),
    }
}

/// Parse a complete raw SSE text stream (multiple events separated by blank lines)
/// into a `Vec<SseEvent>`. Events that fail to parse are silently skipped.
pub fn parse_sse_stream(raw: &str) -> Vec<SseEvent> {
    let mut events = Vec::new();

    for block in raw.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let mut event_type: Option<&str> = None;
        let mut data: Option<&str> = None;

        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                event_type = Some(rest.trim());
            } else if let Some(rest) = line.strip_prefix("data:") {
                data = Some(rest.trim());
            }
        }

        if let (Some(et), Some(d)) = (event_type, data) {
            if let Ok(event) = parse_sse_event(et, d) {
                events.push(event);
            }
        }
    }

    events
}
