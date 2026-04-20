//! `CacheSafeParams` — the input struct for a forked agent call
//! that shares a prompt-cache prefix with its parent.
//!
//! Port of TS `utils/forkedAgent.ts:57-68`.
//!
//! Why "cache-safe"
//! ================
//! The prompt-cache prefix (system prompt + user context + system
//! context) must match the parent's byte-for-byte to hit the cache.
//! This struct groups those fields so fork callers pass them intact
//! rather than reassembling (which risks drift — a missing newline
//! or re-ordered context entry invalidates the whole prefix).
//!
//! Type representation
//! ===================
//! Three fields carry Rust-native types that map cleanly:
//! - `system_prompt: Vec<String>` — TS `SystemPrompt` (branded
//!   `string[]`).
//! - `user_context` / `system_context`: `HashMap<String, String>`.
//!
//! Two fields carry types that aren't ported:
//! - `tool_use_context: serde_json::Value` — TS `ToolUseContext`
//!   has 10+ function-typed fields (`setAppState`,
//!   `setInProgressToolUseIDs`, `updateFileHistoryState`, etc.) that
//!   are runtime behaviour, not data. Callers that need the struct
//!   shape can serialise / deserialise the opaque Value.
//! - `fork_context_messages: Vec<serde_json::Value>` — TS
//!   `Message[]` — the `src/types/message.ts` file is missing from
//!   the leak (see `messages_fold` module docs). Stream-shape is
//!   JSON-compatible regardless.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheSafeParams {
    /// TS `systemPrompt: SystemPrompt` — the base prompt array the
    /// API `system` field is assembled from. Must exactly match the
    /// parent's prompt for cache hits.
    #[serde(rename = "systemPrompt")]
    pub system_prompt: Vec<String>,

    /// User-provided context entries. TS `{ [k]: string }`.
    #[serde(rename = "userContext")]
    pub user_context: HashMap<String, String>,

    /// Host-injected context (platform, git state, etc.). TS
    /// `{ [k]: string }`.
    #[serde(rename = "systemContext")]
    pub system_context: HashMap<String, String>,

    /// TS `ToolUseContext`. Opaque `Value` because the TS type has
    /// function-typed fields (`setAppState`, `abortController`,
    /// etc.) that can't be represented as plain data. Callers
    /// construct this from their already-built runtime struct via
    /// `serde_json::to_value`.
    #[serde(rename = "toolUseContext")]
    pub tool_use_context: Value,

    /// Parent message stream to fork from. `Vec<Value>` because
    /// `types/message.ts` is missing from the leak; shape is the
    /// on-the-wire JSON.
    #[serde(rename = "forkContextMessages")]
    pub fork_context_messages: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrips_preserving_camel_case() {
        let params = CacheSafeParams {
            system_prompt: vec!["system".into(), "rules".into()],
            user_context: HashMap::from([("cwd".into(), "/home".into())]),
            system_context: HashMap::from([("platform".into(), "linux".into())]),
            tool_use_context: json!({ "options": { "debug": false } }),
            fork_context_messages: vec![json!({ "type": "user", "uuid": "u1" })],
        };
        let v = serde_json::to_value(&params).unwrap();
        // TS camelCase on every field.
        assert!(v.get("systemPrompt").is_some());
        assert!(v.get("userContext").is_some());
        assert!(v.get("systemContext").is_some());
        assert!(v.get("toolUseContext").is_some());
        assert!(v.get("forkContextMessages").is_some());

        let back: CacheSafeParams = serde_json::from_value(v).unwrap();
        assert_eq!(back.system_prompt, params.system_prompt);
        assert_eq!(back.user_context, params.user_context);
        assert_eq!(back.fork_context_messages, params.fork_context_messages);
    }

    #[test]
    fn deserialises_from_ts_shape() {
        let wire = json!({
            "systemPrompt": ["S1", "S2"],
            "userContext": { "cwd": "/x" },
            "systemContext": { "platform": "mac" },
            "toolUseContext": { "options": {} },
            "forkContextMessages": []
        });
        let p: CacheSafeParams = serde_json::from_value(wire).unwrap();
        assert_eq!(p.system_prompt.len(), 2);
        assert_eq!(p.user_context.get("cwd").map(String::as_str), Some("/x"));
        assert!(p.fork_context_messages.is_empty());
    }

    #[test]
    fn empty_fields_roundtrip() {
        let params = CacheSafeParams {
            system_prompt: Vec::new(),
            user_context: HashMap::new(),
            system_context: HashMap::new(),
            tool_use_context: Value::Null,
            fork_context_messages: Vec::new(),
        };
        let v = serde_json::to_value(&params).unwrap();
        let back: CacheSafeParams = serde_json::from_value(v).unwrap();
        assert!(back.system_prompt.is_empty());
        assert!(back.user_context.is_empty());
    }
}
