//! Build the outbound message list from a text prompt + optional
//! pasted images + preceding attachment messages.
//!
//! Port of TS `utils/processUserInput/processTextPrompt.ts:1-100`.
//!
//! **Reconstructed types disclaimer.** The TS file imports
//! `AttachmentMessage`, `SystemMessage`, and `UserMessage` from
//! `src/types/message.ts`, **which is missing from the leaked
//! source snapshot**. This port works on `serde_json::Value` and
//! `ContentBlockParam`-shaped maps, documenting the fields it
//! writes.
//!
//! Scope reductions from the TS source
//! ===================================
//! - Drops the `logEvent` / `logOTelEvent` / `startInteractionSpan`
//!   / `setPromptId` side-effect chain. Callers who want analytics
//!   can dispatch them around the call; this function is pure.
//! - `matches_negative_keyword` / `matches_keep_going_keyword` are
//!   EXPOSED as outputs on [`ProcessResult`] so the caller can
//!   continue to emit the `tengu_input_prompt` analytics event with
//!   the same `is_negative` / `is_keep_going` fields.
//!
//! Fields written on the new user message
//! ======================================
//! - `type`: `"user"`.
//! - `uuid`: supplied UUID or a fresh v4 (matches TS `randomUUID()`).
//! - `timestamp`: supplied ISO-8601 string or `now()`.
//! - `message.role`: `"user"`.
//! - `message.content`: either the raw `input` (string) OR an array
//!   that is `[text, ...imageContentBlocks]` when images are
//!   present.
//! - `imagePasteIds` (optional): present only when non-empty.
//! - `permissionMode` (optional).
//! - `isMeta` (optional).

use crate::user_prompt_keywords::{matches_keep_going_keyword, matches_negative_keyword};
use serde_json::{json, Value};

/// Outcome of `process_text_prompt`. Matches TS `{ messages, shouldQuery }`
/// plus two bonus fields surfacing the analytics classifiers so
/// callers can emit the `tengu_input_prompt` event inline.
#[derive(Debug, Clone)]
pub struct ProcessResult {
    pub messages: Vec<Value>,
    pub should_query: bool,
    pub is_negative: bool,
    pub is_keep_going: bool,
    /// The prompt UUID assigned to this invocation — TS stores via
    /// `setPromptId(state)`. Caller can do the same.
    pub prompt_id: String,
}

/// Build the user message(s) for a text prompt. TS
/// `processTextPrompt`.
///
/// Parameters mirror TS argument order:
/// - `input` is accepted as `&Value` to match TS's
///   `string | Array<ContentBlockParam>` union. String → string;
///   array → array-of-blocks; anything else → empty-array.
/// - `image_content_blocks` — already-constructed image blocks,
///   appended after the text.
/// - `image_paste_ids` — echoed onto the new user message so the
///   UI can re-expand references.
/// - `attachment_messages` — emitted verbatim AFTER the new user
///   message (TS appends them to the output list).
/// - `uuid` / `timestamp` / `permission_mode` / `is_meta` — optional
///   factory overrides that match TS `createUserMessage`.
pub fn process_text_prompt(
    input: &Value,
    image_content_blocks: &[Value],
    image_paste_ids: &[u64],
    attachment_messages: &[Value],
    uuid: Option<&str>,
    permission_mode: Option<&str>,
    is_meta: Option<bool>,
) -> ProcessResult {
    // Fresh prompt_id — TS `randomUUID()`.
    let prompt_id = uuid::Uuid::new_v4().to_string();

    // Extract plain text for keyword classification. TS:
    //   typeof input === 'string' ? input : input.find(b => b.type === 'text')?.text || ''
    let user_prompt_text: String = match input {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .find_map(|b| {
                let obj = b.as_object()?;
                (obj.get("type")?.as_str()? == "text")
                    .then(|| obj.get("text").and_then(Value::as_str).unwrap_or(""))
            })
            .unwrap_or("")
            .to_owned(),
        _ => String::new(),
    };

    let is_negative = matches_negative_keyword(&user_prompt_text);
    let is_keep_going = matches_keep_going_keyword(&user_prompt_text);

    let final_uuid = uuid.map(str::to_owned).unwrap_or_else(|| {
        // TS factory defaults `uuid || randomUUID()`, and the prompt_id
        // is a SEPARATE UUID used as an analytics key. They must not
        // collide — the TS code allocates two `randomUUID()` values.
        uuid::Uuid::new_v4().to_string()
    });
    let timestamp = chrono::Utc::now().to_rfc3339();

    // Build content. TS branches on whether there are pasted images.
    let content: Value = if !image_content_blocks.is_empty() {
        // TS builds `[text_block?, ...image_blocks]`. Empty/whitespace
        // input → no leading text block.
        let mut blocks: Vec<Value> = Vec::new();
        match input {
            Value::String(s) if !s.trim().is_empty() => {
                blocks.push(json!({ "type": "text", "text": s }));
            }
            Value::Array(arr) => {
                // TS passes the array through unchanged when no string.
                blocks.extend(arr.iter().cloned());
            }
            _ => {}
        }
        blocks.extend(image_content_blocks.iter().cloned());
        Value::Array(blocks)
    } else {
        input.clone()
    };

    let mut user_msg = json!({
        "type": "user",
        "uuid": final_uuid,
        "timestamp": timestamp,
        "message": {
            "role": "user",
            "content": content,
        }
    });

    // Optional fields — only attached when non-default / non-empty,
    // matching TS's spread-with-undefineds behaviour (undefined keys
    // don't appear in the serialised JSON).
    if let Some(obj) = user_msg.as_object_mut() {
        if !image_paste_ids.is_empty() {
            obj.insert("imagePasteIds".into(), json!(image_paste_ids));
        }
        if let Some(mode) = permission_mode {
            obj.insert("permissionMode".into(), Value::from(mode));
        }
        if is_meta.unwrap_or(false) {
            obj.insert("isMeta".into(), Value::from(true));
        }
    }

    let mut messages: Vec<Value> = Vec::with_capacity(1 + attachment_messages.len());
    messages.push(user_msg);
    messages.extend(attachment_messages.iter().cloned());

    ProcessResult {
        messages,
        should_query: true,
        is_negative,
        is_keep_going,
        prompt_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_string_input_no_images() {
        let r = process_text_prompt(&json!("hello"), &[], &[], &[], None, None, None);
        assert_eq!(r.messages.len(), 1);
        assert!(r.should_query);
        let msg = &r.messages[0];
        assert_eq!(msg["type"].as_str(), Some("user"));
        assert_eq!(msg["message"]["content"].as_str(), Some("hello"));
        assert!(!r.is_negative);
        assert!(!r.is_keep_going);
    }

    #[test]
    fn with_image_blocks_produces_array_content() {
        let images = vec![json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": "abc" }
        })];
        let r = process_text_prompt(
            &json!("here you go"),
            &images,
            &[1, 2],
            &[],
            None,
            None,
            None,
        );
        let content = r.messages[0]["message"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"].as_str(), Some("text"));
        assert_eq!(content[0]["text"].as_str(), Some("here you go"));
        assert_eq!(content[1]["type"].as_str(), Some("image"));
        assert_eq!(r.messages[0]["imagePasteIds"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn empty_text_with_images_skips_text_block() {
        let images = vec![json!({ "type": "image", "source": {} })];
        let r = process_text_prompt(&json!(""), &images, &[], &[], None, None, None);
        let content = r.messages[0]["message"]["content"].as_array().unwrap();
        // Only the image block — TS doesn't push an empty text block.
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"].as_str(), Some("image"));
    }

    #[test]
    fn whitespace_only_text_with_images_skips_text_block() {
        let images = vec![json!({ "type": "image", "source": {} })];
        let r = process_text_prompt(&json!("   \n  "), &images, &[], &[], None, None, None);
        let content = r.messages[0]["message"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
    }

    #[test]
    fn array_input_passes_through_without_images() {
        let input = json!([
            { "type": "text", "text": "primary" },
            { "type": "text", "text": "secondary" },
        ]);
        let r = process_text_prompt(&input, &[], &[], &[], None, None, None);
        let content = r.messages[0]["message"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
    }

    #[test]
    fn attachment_messages_appended_after_user() {
        let attachments = vec![
            json!({ "type": "attachment", "uuid": "a1", "attachment": { "type": "file" } }),
            json!({ "type": "attachment", "uuid": "a2", "attachment": { "type": "hook" } }),
        ];
        let r = process_text_prompt(&json!("hi"), &[], &[], &attachments, None, None, None);
        assert_eq!(r.messages.len(), 3);
        assert_eq!(r.messages[0]["type"].as_str(), Some("user"));
        assert_eq!(r.messages[1]["type"].as_str(), Some("attachment"));
        assert_eq!(r.messages[2]["type"].as_str(), Some("attachment"));
    }

    #[test]
    fn uuid_override_respected() {
        let r = process_text_prompt(
            &json!("hi"),
            &[],
            &[],
            &[],
            Some("00000000-0000-0000-0000-000000000000"),
            None,
            None,
        );
        assert_eq!(
            r.messages[0]["uuid"].as_str(),
            Some("00000000-0000-0000-0000-000000000000")
        );
    }

    #[test]
    fn permission_mode_and_is_meta_applied() {
        let r = process_text_prompt(
            &json!("x"),
            &[],
            &[],
            &[],
            None,
            Some("planning"),
            Some(true),
        );
        assert_eq!(r.messages[0]["permissionMode"].as_str(), Some("planning"));
        assert_eq!(r.messages[0]["isMeta"].as_bool(), Some(true));
    }

    #[test]
    fn is_meta_false_is_omitted() {
        // TS uses `isMeta || undefined` — false is dropped.
        let r = process_text_prompt(&json!("x"), &[], &[], &[], None, None, Some(false));
        assert!(r.messages[0].as_object().unwrap().get("isMeta").is_none());
    }

    #[test]
    fn empty_image_paste_ids_not_included() {
        let r = process_text_prompt(&json!("x"), &[], &[], &[], None, None, None);
        assert!(r.messages[0]
            .as_object()
            .unwrap()
            .get("imagePasteIds")
            .is_none());
    }

    #[test]
    fn negative_keyword_surfaced() {
        let r = process_text_prompt(
            &json!("this is fucking broken"),
            &[],
            &[],
            &[],
            None,
            None,
            None,
        );
        assert!(r.is_negative);
    }

    #[test]
    fn keep_going_keyword_surfaced() {
        let r = process_text_prompt(&json!("continue"), &[], &[], &[], None, None, None);
        assert!(r.is_keep_going);
    }

    #[test]
    fn array_input_classifies_first_text_block_for_negative() {
        let input = json!([
            { "type": "text", "text": "this sucks" },
            { "type": "text", "text": "clean second block" },
        ]);
        let r = process_text_prompt(&input, &[], &[], &[], None, None, None);
        assert!(r.is_negative);
    }

    #[test]
    fn fresh_prompt_id_on_every_call() {
        let a = process_text_prompt(&json!("a"), &[], &[], &[], None, None, None);
        let b = process_text_prompt(&json!("a"), &[], &[], &[], None, None, None);
        assert_ne!(a.prompt_id, b.prompt_id);
    }

    #[test]
    fn uuid_and_prompt_id_are_distinct() {
        // TS allocates two separate randomUUID() values — one for the
        // prompt_id (analytics key), one for the message uuid.
        let r = process_text_prompt(&json!("a"), &[], &[], &[], None, None, None);
        let msg_uuid = r.messages[0]["uuid"].as_str().unwrap();
        assert_ne!(msg_uuid, r.prompt_id);
    }

    #[test]
    fn non_string_non_array_input_yields_empty_classification() {
        // TS typing says string | ContentBlockParam[], so objects
        // shouldn't occur — but defensive handling prevents panics.
        let r = process_text_prompt(&json!({}), &[], &[], &[], None, None, None);
        assert!(!r.is_negative);
        assert!(!r.is_keep_going);
    }
}
