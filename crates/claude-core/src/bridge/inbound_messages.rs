//! Inbound bridge user-message helpers.
//!
//! Mirrors TS `src/bridge/inboundMessages.ts`: accept only SDK `user`
//! messages, extract string or content-block-array content, preserve an
//! optional UUID, and normalize malformed base64 image blocks that use
//! `mediaType` or omit `media_type`.

use base64::Engine as _;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct InboundMessageFields {
    pub content: Value,
    pub uuid: Option<String>,
}

pub fn extract_inbound_message_fields(msg: &Value) -> Option<InboundMessageFields> {
    if msg.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let content = msg.get("message")?.get("content")?;
    if content.is_null() {
        return None;
    }
    if content.as_array().is_some_and(Vec::is_empty) {
        return None;
    }
    if !content.is_string() && !content.is_array() {
        return None;
    }

    let uuid = msg
        .get("uuid")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let content = if content.is_array() {
        normalize_image_blocks(content)
    } else {
        content.clone()
    };

    Some(InboundMessageFields { content, uuid })
}

pub fn normalize_image_blocks(blocks: &Value) -> Value {
    let Some(array) = blocks.as_array() else {
        return blocks.clone();
    };
    if !array.iter().any(is_malformed_base64_image) {
        return blocks.clone();
    }

    Value::Array(
        array
            .iter()
            .map(|block| {
                if !is_malformed_base64_image(block) {
                    return block.clone();
                }
                let data = block
                    .pointer("/source/data")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let media_type = block
                    .pointer("/source/mediaType")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| detect_image_format_from_base64(data).to_string());
                let mut normalized = block.clone();
                normalized["source"] = json!({
                    "type": "base64",
                    "media_type": media_type,
                    "data": data,
                });
                normalized
            })
            .collect(),
    )
}

fn is_malformed_base64_image(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some("image")
        && block.pointer("/source/type").and_then(Value::as_str) == Some("base64")
        && block
            .pointer("/source/media_type")
            .and_then(Value::as_str)
            .is_none()
}

pub fn detect_image_format_from_base64(base64_data: &str) -> &'static str {
    let Ok(buffer) = base64::engine::general_purpose::STANDARD.decode(base64_data) else {
        return "image/png";
    };
    detect_image_format_from_buffer(&buffer)
}

pub fn detect_image_format_from_buffer(buffer: &[u8]) -> &'static str {
    if buffer.len() < 4 {
        return "image/png";
    }
    if buffer[0..4] == [0x89, 0x50, 0x4e, 0x47] {
        return "image/png";
    }
    if buffer.len() >= 3 && buffer[0..3] == [0xff, 0xd8, 0xff] {
        return "image/jpeg";
    }
    if buffer[0..3] == [0x47, 0x49, 0x46] {
        return "image/gif";
    }
    if buffer.len() >= 12
        && buffer[0..4] == [0x52, 0x49, 0x46, 0x46]
        && buffer[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        return "image/webp";
    }
    "image/png"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_user_string_content_and_uuid() {
        let msg = json!({
            "type": "user",
            "uuid": "u-1",
            "message": {"content": "hello"}
        });
        let fields = extract_inbound_message_fields(&msg).unwrap();
        assert_eq!(fields.content, json!("hello"));
        assert_eq!(fields.uuid.as_deref(), Some("u-1"));
    }

    #[test]
    fn skips_non_user_missing_and_empty_array_content() {
        assert!(extract_inbound_message_fields(&json!({
            "type": "assistant",
            "message": {"content": "hello"}
        }))
        .is_none());
        assert!(extract_inbound_message_fields(&json!({
            "type": "user",
            "message": {}
        }))
        .is_none());
        assert!(extract_inbound_message_fields(&json!({
            "type": "user",
            "message": {"content": []}
        }))
        .is_none());
    }

    #[test]
    fn normalizes_camel_case_media_type() {
        let blocks = json!([{
            "type": "image",
            "source": {
                "type": "base64",
                "mediaType": "image/jpeg",
                "data": "/9j/AA=="
            }
        }]);
        let normalized = normalize_image_blocks(&blocks);
        assert_eq!(normalized[0]["source"]["media_type"], "image/jpeg");
        assert!(normalized[0]["source"].get("mediaType").is_none());
    }

    #[test]
    fn detects_missing_media_type_from_magic_bytes() {
        let blocks = json!([{
            "type": "image",
            "source": {
                "type": "base64",
                "data": "R0lGODlhAQABAAAAACw="
            }
        }]);
        let normalized = normalize_image_blocks(&blocks);
        assert_eq!(normalized[0]["source"]["media_type"], "image/gif");
    }
}
