//! Collapse consecutive completed-background-bash task notifications.
//!
//! Port of TS `utils/collapseBackgroundBashNotifications.ts:1-84`.
//!
//! **Reconstructed types disclaimer.** The TS file imports
//! `NormalizedUserMessage` and `RenderableMessage` from
//! `src/types/message.ts`, **which is missing from the leaked
//! source snapshot**. This port works directly on
//! `serde_json::Value` — the on-the-wire JSON shape the TS
//! messages cross — and documents the exact fields read/written.
//!
//! Fields touched
//! ==============
//! Read from each message:
//! - `msg.type` (string) — must be `"user"` to be eligible
//! - `msg.message.content` (array) — first element only
//! - `msg.message.content[0].type` — must be `"text"`
//! - `msg.message.content[0].text` — the outer tag payload
//!
//! Read from the inner text via [`extract_tag`]:
//! - `<status>` — must equal `"completed"` (failed/killed left alone)
//! - `<summary>` — must start with [`BACKGROUND_BASH_SUMMARY_PREFIX`]
//!   so monitor/agent/workflow notifications with the same tag don't
//!   collapse (TS comment at
//!   `collapseBackgroundBashNotifications.ts:21-26`)
//!
//! Written (when synthesising a merged notification):
//! - `msg.message.content` replaced with a single text block whose
//!   content is a re-emitted
//!   `<task-notification><status>completed</status><summary>N
//!   background commands completed</summary></task-notification>`.
//!   TS comment calls this "no new renderer needed" — the TUI
//!   already knows how to render the outer tag.
//!
//! Gate
//! ====
//! TS gates on `isFullscreenEnvEnabled() && !verbose`. Porting that
//! gate would require the full tmux/terminal detection graph (not
//! in this port's scope). The Rust function takes a caller-supplied
//! `fullscreen_enabled` parameter so the decision stays at the UI
//! layer where it belongs.

use serde_json::{json, Value};

/// TS `tasks/LocalShellTask/LocalShellTask.tsx:23`. Used to
/// distinguish bash-kind completions from monitor/agent/workflow.
pub const BACKGROUND_BASH_SUMMARY_PREFIX: &str = "Background command ";

/// TS `constants/xml.ts:28,33,34` (see [`xml_escape_for_extract`]).
pub const TASK_NOTIFICATION_TAG: &str = "task-notification";
pub const STATUS_TAG: &str = "status";
pub const SUMMARY_TAG: &str = "summary";

/// Extract the content between `<tag>…</tag>` at nesting depth 0,
/// matching TS `utils/messages.ts:633-687` `extractTag`.
///
/// Handles:
/// - self-closing tags with attributes
/// - nested tags of the same name (only the outermost matches)
/// - multiline content
///
/// Returns `None` when `html` or `tag_name` is empty, when the tag
/// is not found, or when every match occurs inside another
/// unmatched opening of the same tag.
pub fn extract_tag(html: &str, tag_name: &str) -> Option<String> {
    if html.trim().is_empty() || tag_name.trim().is_empty() {
        return None;
    }
    let escaped = xml_escape_for_extract(tag_name);
    // Opening tag with optional attributes, non-greedy content, closing tag.
    let pattern = format!(
        r"(?is)<{tag}(?:\s+[^>]*)?>([\s\S]*?)</{tag}>",
        tag = escaped
    );
    let re = regex::Regex::new(&pattern).ok()?;
    let open_pat = format!(r"(?is)<{tag}(?:\s+[^>]*?)?>", tag = escaped);
    let close_pat = format!(r"(?is)</{tag}>", tag = escaped);
    let open_re = regex::Regex::new(&open_pat).ok()?;
    let close_re = regex::Regex::new(&close_pat).ok()?;

    let mut last_index = 0usize;
    for m in re.captures_iter(html) {
        let full = m.get(0)?;
        let content = m.get(1)?;
        let before = &html[last_index..full.start()];
        // Depth counter in TS: opens - closes seen before this match.
        let opens = open_re.find_iter(before).count();
        let closes = close_re.find_iter(before).count();
        let depth = opens as i64 - closes as i64;
        if depth == 0 && !content.as_str().is_empty() {
            return Some(content.as_str().to_owned());
        }
        last_index = full.end();
    }
    None
}

/// Escape regex metacharacters in a tag name. Same regex-quoting TS
/// `escapeRegExp` applies to tag names like `task-notification` (the
/// `-` is a metacharacter inside `[...]` but safe outside, so
/// escaping is conservative but harmless).
fn xml_escape_for_extract(tag: &str) -> String {
    let mut out = String::with_capacity(tag.len() * 2);
    for c in tag.chars() {
        if matches!(
            c,
            '.' | '*'
                | '+'
                | '?'
                | '^'
                | '$'
                | '{'
                | '}'
                | '('
                | ')'
                | '|'
                | '['
                | ']'
                | '\\'
                | '/'
                | '-'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn is_completed_background_bash(msg: &Value) -> bool {
    let Some(obj) = msg.as_object() else {
        return false;
    };
    if obj.get("type").and_then(Value::as_str) != Some("user") {
        return false;
    }
    let Some(content) = obj
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    let Some(first) = content.first() else {
        return false;
    };
    let Some(first_obj) = first.as_object() else {
        return false;
    };
    if first_obj.get("type").and_then(Value::as_str) != Some("text") {
        return false;
    }
    let Some(text) = first_obj.get("text").and_then(Value::as_str) else {
        return false;
    };
    // Must contain the task-notification tag — TS uses `.includes` on
    // the opening tag name rather than full tag (allows attributes).
    if !text.contains(&format!("<{TASK_NOTIFICATION_TAG}")) {
        return false;
    }
    // Status must be "completed" — failed/killed stay visible.
    if extract_tag(text, STATUS_TAG).as_deref() != Some("completed") {
        return false;
    }
    // Summary must start with the bash prefix — distinguishes from
    // monitor/agent/workflow notifications with the same outer tag.
    extract_tag(text, SUMMARY_TAG)
        .map(|s| s.starts_with(BACKGROUND_BASH_SUMMARY_PREFIX))
        .unwrap_or(false)
}

/// Fold consecutive completed-background-bash task notifications
/// into a single "N background commands completed" synthesised
/// message. TS `collapseBackgroundBashNotifications`.
///
/// Pass-through (returns input clone) when `fullscreen_enabled` is
/// `false` or `verbose` is `true` — ctrl+O / non-TUI mode should
/// show each completion individually.
pub fn collapse_background_bash_notifications(
    messages: &[Value],
    verbose: bool,
    fullscreen_enabled: bool,
) -> Vec<Value> {
    if !fullscreen_enabled || verbose {
        return messages.to_vec();
    }

    let mut result: Vec<Value> = Vec::with_capacity(messages.len());
    let mut i = 0usize;

    while i < messages.len() {
        let msg = &messages[i];
        if is_completed_background_bash(msg) {
            let mut count = 0usize;
            while i < messages.len() && is_completed_background_bash(&messages[i]) {
                count += 1;
                i += 1;
            }
            if count == 1 {
                // `msg` was consumed by the inner loop — its index is
                // i-1 now, so re-read from the original slice.
                result.push(messages[i - 1].clone());
            } else {
                result.push(synthesise_merged(msg, count));
            }
        } else {
            result.push(msg.clone());
            i += 1;
        }
    }
    result
}

fn synthesise_merged(first: &Value, count: usize) -> Value {
    let mut out = first.clone();
    let summary = format!("{count} background commands completed");
    let text = format!(
        "<{TASK_NOTIFICATION_TAG}>\
         <{STATUS_TAG}>completed</{STATUS_TAG}>\
         <{SUMMARY_TAG}>{summary}</{SUMMARY_TAG}>\
         </{TASK_NOTIFICATION_TAG}>"
    );
    if let Some(obj) = out.as_object_mut() {
        obj.insert(
            "message".into(),
            json!({
                "role": "user",
                "content": [
                    { "type": "text", "text": text }
                ],
            }),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn completed_bg(i: usize) -> Value {
        let text = format!(
            "<{TASK_NOTIFICATION_TAG}>\
             <{STATUS_TAG}>completed</{STATUS_TAG}>\
             <{SUMMARY_TAG}>{BACKGROUND_BASH_SUMMARY_PREFIX}\"cmd {i}\" completed (exit 0)</{SUMMARY_TAG}>\
             </{TASK_NOTIFICATION_TAG}>"
        );
        json!({
            "type": "user",
            "uuid": format!("u{i}"),
            "message": {
                "role": "user",
                "content": [
                    { "type": "text", "text": text }
                ],
            }
        })
    }

    #[test]
    fn extract_tag_basic() {
        assert_eq!(
            extract_tag("<status>completed</status>", "status").as_deref(),
            Some("completed")
        );
    }

    #[test]
    fn extract_tag_with_attributes() {
        assert_eq!(
            extract_tag(r#"<status level="info">ok</status>"#, "status").as_deref(),
            Some("ok"),
        );
    }

    #[test]
    fn extract_tag_multiline() {
        let html = "<summary>line one\nline two</summary>";
        assert_eq!(extract_tag(html, "summary").as_deref(), Some("line one\nline two"));
    }

    #[test]
    fn extract_tag_returns_outermost_content_when_nested() {
        // `<s>outer<s>inner</s>rest</s>` → "outer<s>inner</s>rest".
        // The outermost match's content is the full inner text
        // including the nested tag. TS's depth tracking ensures this.
        let out = extract_tag("<s>outer<s>inner</s>rest</s>", "s");
        assert!(out.is_some());
        let v = out.unwrap();
        assert!(v.contains("inner"));
        assert!(v.contains("outer"));
    }

    #[test]
    fn extract_tag_missing_returns_none() {
        assert_eq!(extract_tag("<other>x</other>", "status"), None);
    }

    #[test]
    fn extract_tag_empty_inputs_return_none() {
        assert_eq!(extract_tag("", "status"), None);
        assert_eq!(extract_tag("<x>y</x>", ""), None);
    }

    #[test]
    fn disabled_fullscreen_passes_through() {
        let input = vec![completed_bg(1), completed_bg(2)];
        let out = collapse_background_bash_notifications(&input, false, false);
        assert_eq!(out, input);
    }

    #[test]
    fn verbose_passes_through() {
        let input = vec![completed_bg(1), completed_bg(2)];
        let out = collapse_background_bash_notifications(&input, true, true);
        assert_eq!(out, input);
    }

    #[test]
    fn single_completion_passes_through() {
        let input = vec![completed_bg(1)];
        let out = collapse_background_bash_notifications(&input, false, true);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn two_consecutive_completions_collapse() {
        let input = vec![completed_bg(1), completed_bg(2)];
        let out = collapse_background_bash_notifications(&input, false, true);
        assert_eq!(out.len(), 1);
        let text = out[0]["message"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("2 background commands completed"));
    }

    #[test]
    fn non_bash_notification_left_alone() {
        // Has task-notification + status=completed but summary lacks
        // the background prefix → untouched (monitor-kind).
        let other = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": format!("<{TASK_NOTIFICATION_TAG}>\
                                     <{STATUS_TAG}>completed</{STATUS_TAG}>\
                                     <{SUMMARY_TAG}>monitor stream event</{SUMMARY_TAG}>\
                                     </{TASK_NOTIFICATION_TAG}>"),
                }]
            }
        });
        let input = vec![other.clone(), other.clone()];
        let out = collapse_background_bash_notifications(&input, false, true);
        // Unchanged — neither matched the bash predicate.
        assert_eq!(out, input);
    }

    #[test]
    fn failed_status_left_alone() {
        let failed = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": format!("<{TASK_NOTIFICATION_TAG}>\
                                     <{STATUS_TAG}>failed</{STATUS_TAG}>\
                                     <{SUMMARY_TAG}>{BACKGROUND_BASH_SUMMARY_PREFIX}failed</{SUMMARY_TAG}>\
                                     </{TASK_NOTIFICATION_TAG}>"),
                }]
            }
        });
        let input = vec![failed.clone(), failed.clone()];
        let out = collapse_background_bash_notifications(&input, false, true);
        assert_eq!(out, input);
    }

    #[test]
    fn non_consecutive_groups_collapse_separately() {
        let other = json!({ "type": "assistant", "uuid": "a1" });
        let input = vec![
            completed_bg(1),
            completed_bg(2),
            other.clone(),
            completed_bg(3),
            completed_bg(4),
            completed_bg(5),
        ];
        let out = collapse_background_bash_notifications(&input, false, true);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1], other);
        let first = out[0]["message"]["content"][0]["text"].as_str().unwrap();
        let second = out[2]["message"]["content"][0]["text"].as_str().unwrap();
        assert!(first.contains("2 background commands"));
        assert!(second.contains("3 background commands"));
    }

    #[test]
    fn prefix_constant_pin() {
        // Pin the prefix — TS `tasks/LocalShellTask/LocalShellTask.tsx:23`.
        assert_eq!(BACKGROUND_BASH_SUMMARY_PREFIX, "Background command ");
    }
}
