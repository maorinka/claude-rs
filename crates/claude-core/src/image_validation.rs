//! API-boundary validation for base64 image payload size.
//!
//! Port of TS `utils/imageValidation.ts:1-104`.
//!
//! Last-line-of-defence check before images ship to the API. Upstream
//! paths (paste attach, file read) should already clamp or resize, but
//! this guard ensures a bypass still surfaces a typed error at the
//! boundary rather than a cryptic HTTP 400 from the server.
//!
//! **Important**: the 5 MB cap is on the **base64-encoded string
//! length**, not the decoded bytes. Raw 3.75 MB → base64 5 MB.
//! `IMAGE_TARGET_RAW_SIZE` in `constants::api_limits` captures that.

use crate::constants::api_limits::API_IMAGE_MAX_BASE64_SIZE;
use serde_json::Value;
use thiserror::Error;

/// Per-image record of what tripped the cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OversizedImage {
    /// 1-indexed position in the flattened image sequence across all
    /// user messages — matches TS `imageIndex++` at imageValidation.ts:67.
    pub index: usize,
    /// Size of the base64 payload in bytes (UTF-8 bytes, which equals
    /// ASCII chars here since base64 is ASCII-only).
    pub size: usize,
}

#[derive(Debug, Error)]
pub enum ImageValidationError {
    /// TS throws `ImageSizeError` with a formatted message. Rust
    /// surfaces the structured list so callers can format for their
    /// own context (CLI / TUI / JSON API) without string parsing, and
    /// the `Display` impl still produces the TS-equivalent string for
    /// log/stderr compatibility.
    #[error("{}", format_image_size_error(images, *max_size))]
    SizeLimitExceeded {
        images: Vec<OversizedImage>,
        max_size: usize,
    },
}

fn format_file_size(bytes: usize) -> String {
    // Minimal en-US-style formatter with binary units. Matches TS
    // `formatFileSize` shape (used inline at imageValidation.ts:22, 26).
    // Kept local because the existing `format_file_size` in
    // `mcp/output_storage.rs` is file-local; hoisting it would be a
    // larger refactor than this port deserves.
    const KB: usize = 1024;
    const MB: usize = 1024 * 1024;
    if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_image_size_error(images: &[OversizedImage], max_size: usize) -> String {
    match images {
        [single] => format!(
            "Image base64 size ({}) exceeds API limit ({}). Please resize the image before sending.",
            format_file_size(single.size),
            format_file_size(max_size),
        ),
        many => {
            let list: Vec<String> = many
                .iter()
                .map(|img| format!("Image {}: {}", img.index, format_file_size(img.size)))
                .collect();
            format!(
                "{} images exceed the API limit ({}): {}. Please resize these images before sending.",
                many.len(),
                format_file_size(max_size),
                list.join(", "),
            )
        }
    }
}

fn is_base64_image_block(block: &Value) -> Option<&str> {
    // Matches TS `isBase64ImageBlock` shape walker:
    // `{ type: 'image', source: { type: 'base64', data: <string> } }`
    let obj = block.as_object()?;
    if obj.get("type")?.as_str()? != "image" {
        return None;
    }
    let source = obj.get("source")?.as_object()?;
    if source.get("type")?.as_str()? != "base64" {
        return None;
    }
    source.get("data")?.as_str()
}

/// Validate that every base64 image block across the provided messages
/// fits under `API_IMAGE_MAX_BASE64_SIZE`. Returns `Err` with the full
/// list of oversized images (so the caller can show one message that
/// flags every offender, not just the first).
///
/// Accepts the TS "wrapped message" shape `{ type: "user", message:
/// { content: [...] } }`. Only `user` messages are inspected — TS does
/// the same at imageValidation.ts:76 because assistant messages come
/// from the API, not user input.
pub fn validate_images_for_api(messages: &[Value]) -> Result<(), ImageValidationError> {
    let mut oversized: Vec<OversizedImage> = Vec::new();
    let mut image_index: usize = 0;

    for msg in messages {
        let Some(m) = msg.as_object() else { continue };
        if m.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let Some(inner) = m.get("message").and_then(Value::as_object) else {
            continue;
        };
        let Some(content) = inner.get("content").and_then(Value::as_array) else {
            continue;
        };

        for block in content {
            if let Some(data) = is_base64_image_block(block) {
                image_index += 1;
                // TS checks `block.source.data.length` (UTF-16 code
                // units). Base64 payloads are ASCII-only, so byte
                // count, UTF-8 char count, and UTF-16 code-unit count
                // all agree — `.len()` is safe.
                let size = data.len();
                if size > API_IMAGE_MAX_BASE64_SIZE {
                    oversized.push(OversizedImage {
                        index: image_index,
                        size,
                    });
                }
            }
        }
    }

    if oversized.is_empty() {
        Ok(())
    } else {
        Err(ImageValidationError::SizeLimitExceeded {
            images: oversized,
            max_size: API_IMAGE_MAX_BASE64_SIZE,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_msg_with_content(content: Value) -> Value {
        json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": content,
            }
        })
    }

    fn image_block(size: usize) -> Value {
        json!({
            "type": "image",
            "source": {
                "type": "base64",
                // ASCII 'A' repeated — base64-valid, cheap to allocate.
                "data": "A".repeat(size),
            }
        })
    }

    #[test]
    fn accepts_empty_input() {
        assert!(validate_images_for_api(&[]).is_ok());
    }

    #[test]
    fn accepts_messages_with_no_images() {
        let msgs = vec![user_msg_with_content(json!([
            { "type": "text", "text": "hello" }
        ]))];
        assert!(validate_images_for_api(&msgs).is_ok());
    }

    #[test]
    fn accepts_image_within_limit() {
        let msgs = vec![user_msg_with_content(json!([image_block(1_000_000)]))];
        assert!(validate_images_for_api(&msgs).is_ok());
    }

    #[test]
    fn rejects_single_oversized_image() {
        let too_big = API_IMAGE_MAX_BASE64_SIZE + 1;
        let msgs = vec![user_msg_with_content(json!([image_block(too_big)]))];
        let err = validate_images_for_api(&msgs).unwrap_err();
        let ImageValidationError::SizeLimitExceeded {
            images,
            max_size,
        } = err;
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].index, 1);
        assert_eq!(images[0].size, too_big);
        assert_eq!(max_size, API_IMAGE_MAX_BASE64_SIZE);
    }

    #[test]
    fn single_image_error_message_matches_ts_shape() {
        // TS: "Image base64 size (X) exceeds API limit (Y). Please resize the image before sending."
        let msgs = vec![user_msg_with_content(json!([image_block(
            API_IMAGE_MAX_BASE64_SIZE + 1
        )]))];
        let err = validate_images_for_api(&msgs).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Image base64 size"), "unexpected: {msg}");
        assert!(msg.contains("exceeds API limit"));
        assert!(msg.contains("Please resize the image before sending"));
    }

    #[test]
    fn rejects_multiple_oversized_images() {
        let too_big = API_IMAGE_MAX_BASE64_SIZE + 1;
        let msgs = vec![
            user_msg_with_content(json!([image_block(too_big), image_block(100)])),
            user_msg_with_content(json!([image_block(too_big)])),
        ];
        let err = validate_images_for_api(&msgs).unwrap_err();
        let ImageValidationError::SizeLimitExceeded { images, .. } = err;
        assert_eq!(images.len(), 2);
        // TS increments `imageIndex` for EVERY image block (oversize or not),
        // so the two oversized images get indices 1 and 3 — the 2nd block's
        // valid image bumped the counter to 2 before the 3rd overflowed.
        assert_eq!(images[0].index, 1);
        assert_eq!(images[1].index, 3);
    }

    #[test]
    fn multi_image_error_message_lists_all() {
        let too_big = API_IMAGE_MAX_BASE64_SIZE + 1;
        let msgs = vec![user_msg_with_content(json!([
            image_block(too_big),
            image_block(too_big),
        ]))];
        let err = validate_images_for_api(&msgs).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.starts_with("2 images exceed the API limit"), "unexpected: {msg}");
        assert!(msg.contains("Image 1"));
        assert!(msg.contains("Image 2"));
    }

    #[test]
    fn ignores_assistant_messages() {
        // Assistant-origin images come from the API and aren't subject
        // to the outbound check. TS hard-codes `m.type !== 'user'` skip.
        let assistant = json!({
            "type": "assistant",
            "message": { "content": [image_block(API_IMAGE_MAX_BASE64_SIZE + 1)] }
        });
        assert!(validate_images_for_api(&[assistant]).is_ok());
    }

    #[test]
    fn ignores_string_content() {
        // `content` as a string (non-array) has no image blocks to check.
        let msg = json!({
            "type": "user",
            "message": { "role": "user", "content": "hello" }
        });
        assert!(validate_images_for_api(&[msg]).is_ok());
    }

    #[test]
    fn ignores_non_base64_image_source() {
        // URL-sourced images aren't validated; the API size cap is on
        // the base64 data field specifically.
        let url_image = json!({
            "type": "image",
            "source": {
                "type": "url",
                "url": "https://example.com/x.png"
            }
        });
        let msgs = vec![user_msg_with_content(json!([url_image]))];
        assert!(validate_images_for_api(&msgs).is_ok());
    }

    #[test]
    fn exact_limit_accepted() {
        // Boundary: `> max_size` is the reject condition, so exact
        // `max_size` must pass. (Matches TS `base64Size > API_IMAGE_MAX_BASE64_SIZE`.)
        let msgs = vec![user_msg_with_content(json!([image_block(
            API_IMAGE_MAX_BASE64_SIZE
        )]))];
        assert!(validate_images_for_api(&msgs).is_ok());
    }

    #[test]
    fn format_file_size_renders_units() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(1500), "1.46 KB");
        assert_eq!(format_file_size(2 * 1024 * 1024), "2.00 MB");
    }
}
