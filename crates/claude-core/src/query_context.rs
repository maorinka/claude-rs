//! Pure-logic helpers for assembling query-engine parameters.
//!
//! Port of TS `utils/queryContext.ts:1-182`.
//!
//! **Reconstructed types disclaimer.** The TS file imports 12+
//! cross-cutting types that aren't ported in the Rust tree
//! (`Command`, `Tools`, `ToolUseContext`, `AppState`,
//! `MCPServerConnection`, `AgentDefinition`, `Message`,
//! `FileStateCache`, `CacheSafeParams`, `ThinkingConfig`, etc.) plus
//! three async "fetch" functions (`getSystemPrompt`,
//! `getUserContext`, `getSystemContext`). Rather than force-port
//! that entire upstream graph, this port isolates the **pure
//! logic** of the two TS functions into three helpers that a
//! caller can compose once the upstream async fetches return:
//!
//! 1. [`select_system_prompt_parts`] — the conditional-skip
//!    for `customSystemPrompt` in `fetchSystemPromptParts`.
//! 2. [`compose_system_prompt`] — the `asSystemPrompt([...])`
//!    concatenation.
//! 3. [`strip_in_progress_assistant`] — the last-message guard
//!    before forking context.
//! 4. [`default_thinking_config`] — the
//!    `thinkingConfig ?? (shouldEnableThinkingByDefault() ? …)`
//!    fallback.
//!
//! Scope note
//! ==========
//! The `ToolUseContext` / `CacheSafeParams` struct-assembly at
//! `queryContext.ts:142-178` is pure data shuffling once the
//! upstream types land. It's not ported here because every field is
//! one of those missing types — the logic portion is captured in
//! the four helpers above, and the receiving Rust caller can
//! assemble its own `ToolUseContext` Rust equivalent when that type
//! graph is ported.
//!
//! Fields touched
//! ==============
//! - `messages[-1].type` — `"assistant"` discriminator.
//! - `messages[-1].message.stop_reason` — `null` indicates in-flight.

use serde_json::{json, Value};

/// Choose which system-prompt parts to include based on whether a
/// custom prompt is provided.
///
/// Matches TS `fetchSystemPromptParts` at
/// `queryContext.ts:61-72`: when `custom_system_prompt.is_some()`,
/// the default prompt AND system context are skipped (empty).
/// `user_context` is always returned as-is.
///
/// The async fetches themselves (`getSystemPrompt`,
/// `getSystemContext`, `getUserContext`) are the caller's
/// responsibility — pass the results in and this helper applies the
/// conditional.
pub fn select_system_prompt_parts(
    default_system_prompt: Vec<String>,
    user_context: std::collections::HashMap<String, String>,
    system_context: std::collections::HashMap<String, String>,
    custom_system_prompt: Option<&str>,
) -> SystemPromptParts {
    let has_custom = custom_system_prompt.is_some();
    SystemPromptParts {
        default_system_prompt: if has_custom {
            Vec::new()
        } else {
            default_system_prompt
        },
        user_context,
        system_context: if has_custom {
            std::collections::HashMap::new()
        } else {
            system_context
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPromptParts {
    pub default_system_prompt: Vec<String>,
    pub user_context: std::collections::HashMap<String, String>,
    pub system_context: std::collections::HashMap<String, String>,
}

/// Concatenate the system prompt parts into one string array ready
/// for the API `system` field.
///
/// TS `queryContext.ts:127-132`:
/// ```
/// asSystemPrompt([
///   ...(customSystemPrompt !== undefined ? [customSystemPrompt] : defaultSystemPrompt),
///   ...(appendSystemPrompt ? [appendSystemPrompt] : []),
/// ])
/// ```
///
/// `asSystemPrompt` itself (from `systemPromptType.ts`) is a
/// brand-type cast in TS — no runtime cost. The Rust return is a
/// plain `Vec<String>`; callers that need a typed wrapper can add
/// one locally.
pub fn compose_system_prompt(
    default_system_prompt: &[String],
    custom_system_prompt: Option<&str>,
    append_system_prompt: Option<&str>,
) -> Vec<String> {
    let mut out: Vec<String> = match custom_system_prompt {
        Some(custom) => vec![custom.to_owned()],
        None => default_system_prompt.to_vec(),
    };
    if let Some(append) = append_system_prompt {
        if !append.is_empty() {
            out.push(append.to_owned());
        }
    }
    out
}

/// Strip an in-progress assistant message from the end of a message
/// stream. TS `queryContext.ts:136-140`.
///
/// TS detects `last.type === 'assistant' && last.message.stop_reason === null`
/// and slices it off. Comment explains: "same guard as btw.tsx. The
/// SDK can fire side_question mid-turn."
///
/// Returns a borrowed slice — caller clones only if they need
/// ownership. Matches the TS `.slice(0, -1)` behaviour (returns the
/// full slice when the last message isn't an in-progress assistant).
pub fn strip_in_progress_assistant(messages: &[Value]) -> &[Value] {
    let Some(last) = messages.last() else {
        return messages;
    };
    let Some(obj) = last.as_object() else {
        return messages;
    };
    if obj.get("type").and_then(Value::as_str) != Some("assistant") {
        return messages;
    }
    // TS: `.message.stop_reason === null`. A missing key is NOT the
    // same as null in JS (undefined vs null) — the TS code requires
    // an explicit null. Mirror that: only when `stop_reason` is
    // present AND `Value::Null` do we slice.
    let stop_reason = obj.get("message").and_then(|m| m.get("stop_reason"));
    if stop_reason == Some(&Value::Null) {
        &messages[..messages.len() - 1]
    } else {
        messages
    }
}

/// Return the caller's explicit thinking config, else default based
/// on `should_enable_thinking_by_default`.
///
/// TS `queryContext.ts:149-153`:
/// ```
/// thinkingConfig ?? (
///   shouldEnableThinkingByDefault() !== false
///     ? { type: 'adaptive' }
///     : { type: 'disabled' }
/// )
/// ```
///
/// Note the `!== false`: `shouldEnableThinkingByDefault` returns
/// `false | undefined | 'adaptive'` or similar, and TS treats
/// "anything not `false`" as enabling thinking. The Rust port takes
/// a clean `bool` — `true` = enable by default, `false` = disable.
/// Callers that need the three-state TS semantic pre-convert with
/// `should_enable_thinking_by_default() != Some(false)`.
pub fn default_thinking_config(
    explicit: Option<Value>,
    should_enable_by_default: bool,
) -> Value {
    if let Some(cfg) = explicit {
        return cfg;
    }
    if should_enable_by_default {
        json!({ "type": "adaptive" })
    } else {
        json!({ "type": "disabled" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn user_ctx() -> HashMap<String, String> {
        HashMap::from([("cwd".into(), "/home/u".into())])
    }

    fn sys_ctx() -> HashMap<String, String> {
        HashMap::from([("platform".into(), "linux".into())])
    }

    #[test]
    fn select_skips_default_and_system_with_custom() {
        let parts = select_system_prompt_parts(
            vec!["default part".into()],
            user_ctx(),
            sys_ctx(),
            Some("custom prompt"),
        );
        assert!(parts.default_system_prompt.is_empty());
        assert!(parts.system_context.is_empty());
        // user_context always survives.
        assert_eq!(parts.user_context.get("cwd").map(String::as_str), Some("/home/u"));
    }

    #[test]
    fn select_keeps_all_without_custom() {
        let parts = select_system_prompt_parts(
            vec!["default part".into()],
            user_ctx(),
            sys_ctx(),
            None,
        );
        assert_eq!(parts.default_system_prompt, vec!["default part".to_string()]);
        assert_eq!(parts.system_context.get("platform").map(String::as_str), Some("linux"));
        assert_eq!(parts.user_context.get("cwd").map(String::as_str), Some("/home/u"));
    }

    #[test]
    fn compose_with_custom_uses_custom_only() {
        let defaults = vec!["default a".into(), "default b".into()];
        let out = compose_system_prompt(&defaults, Some("custom"), None);
        assert_eq!(out, vec!["custom".to_string()]);
    }

    #[test]
    fn compose_without_custom_uses_defaults() {
        let defaults = vec!["a".into(), "b".into()];
        let out = compose_system_prompt(&defaults, None, None);
        assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn compose_appends_when_append_provided() {
        let defaults = vec!["main".into()];
        let out = compose_system_prompt(&defaults, None, Some("extra note"));
        assert_eq!(out, vec!["main".to_string(), "extra note".to_string()]);
    }

    #[test]
    fn compose_empty_append_not_pushed() {
        // TS `appendSystemPrompt ? [appendSystemPrompt] : []` treats
        // empty string as falsy. Rust mirrors.
        let defaults = vec!["main".into()];
        let out = compose_system_prompt(&defaults, None, Some(""));
        assert_eq!(out, vec!["main".to_string()]);
    }

    #[test]
    fn compose_custom_plus_append() {
        let defaults = vec!["default".into()];
        let out = compose_system_prompt(&defaults, Some("custom"), Some("append"));
        assert_eq!(out, vec!["custom".to_string(), "append".to_string()]);
    }

    #[test]
    fn strip_removes_in_progress_assistant() {
        let msgs = vec![
            json!({ "type": "user", "uuid": "u1" }),
            json!({
                "type": "assistant",
                "uuid": "a1",
                "message": { "stop_reason": null },
            }),
        ];
        let out = strip_in_progress_assistant(&msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["uuid"].as_str(), Some("u1"));
    }

    #[test]
    fn strip_keeps_completed_assistant() {
        let msgs = vec![json!({
            "type": "assistant",
            "uuid": "a1",
            "message": { "stop_reason": "end_turn" },
        })];
        let out = strip_in_progress_assistant(&msgs);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn strip_keeps_user_last() {
        let msgs = vec![json!({
            "type": "user",
            "uuid": "u1",
            "message": { "stop_reason": null },
        })];
        let out = strip_in_progress_assistant(&msgs);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn strip_missing_stop_reason_is_not_null() {
        // TS `stop_reason === null` is strict — undefined (missing)
        // does NOT satisfy. The Rust check requires
        // `Value::Null` presence.
        let msgs = vec![json!({
            "type": "assistant",
            "uuid": "a1",
            "message": {},
        })];
        let out = strip_in_progress_assistant(&msgs);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn strip_empty_input_is_empty() {
        let out = strip_in_progress_assistant(&[]);
        assert!(out.is_empty());
    }

    #[test]
    fn strip_doesnt_touch_earlier_assistant_messages() {
        let msgs = vec![
            json!({
                "type": "assistant",
                "uuid": "earlier",
                "message": { "stop_reason": null },
            }),
            json!({ "type": "user", "uuid": "mid" }),
            json!({
                "type": "assistant",
                "uuid": "last",
                "message": { "stop_reason": null },
            }),
        ];
        let out = strip_in_progress_assistant(&msgs);
        // Only the final in-progress assistant is stripped.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["uuid"].as_str(), Some("earlier"));
        assert_eq!(out[1]["uuid"].as_str(), Some("mid"));
    }

    #[test]
    fn thinking_returns_explicit_when_provided() {
        let explicit = json!({ "type": "budget", "budget_tokens": 5000 });
        let out = default_thinking_config(Some(explicit.clone()), true);
        assert_eq!(out, explicit);
    }

    #[test]
    fn thinking_default_adaptive_when_enabled() {
        let out = default_thinking_config(None, true);
        assert_eq!(out, json!({ "type": "adaptive" }));
    }

    #[test]
    fn thinking_default_disabled_when_not_enabled() {
        let out = default_thinking_config(None, false);
        assert_eq!(out, json!({ "type": "disabled" }));
    }

    #[test]
    fn thinking_explicit_disabled_survives_even_if_default_enabled() {
        // Pins the `??` semantic: explicit value wins regardless of
        // the default.
        let explicit = json!({ "type": "disabled" });
        let out = default_thinking_config(Some(explicit.clone()), true);
        assert_eq!(out, explicit);
    }
}
