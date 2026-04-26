//! Auto-mode (YOLO / `--dangerously-skip-permissions`) classifier.
//!
//! Port of TS `src/utils/permissions/yoloClassifier.ts`. Asks the
//! secondary (Haiku) model whether a tool action is safe to allow
//! without interactive approval. The Rust port can't issue a Haiku
//! tool-use call via the current SecondaryModel trait (which only
//! exposes one-shot text completion), so we ask the model to return
//! the same JSON the TS classify_result tool receives, then parse
//! it.
//!
//! When no secondary model is registered (CLI startup didn't wire
//! one in), `classify_action` returns Ok(None) — callers should fall
//! back to interactive approval.

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::secondary_model;

/// Parsed verdict from the Haiku classifier. Mirrors the TS schema
/// (yoloClassifier.ts:262-285) verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClassifierVerdict {
    /// Brief step-by-step reasoning. Surface in audit logs but not
    /// in the user-facing UI.
    pub thinking: String,
    /// `true` → block (callers should deny or fall back to ask).
    /// `false` → allow.
    #[serde(rename = "shouldBlock")]
    pub should_block: bool,
    /// One-line explanation of the decision.
    pub reason: String,
}

/// Classify an agent action. `tool_name` + `tool_input_json` describe
/// what the agent wants to do; `recent_transcript` is the recent
/// conversation context the classifier uses for risk assessment.
///
/// Returns `Ok(None)` when no secondary model is registered.
/// Returns `Err` only if the model call itself fails — JSON parse
/// failures map to a conservative `should_block: true` verdict so the
/// auto-mode path falls back to interactive approval rather than
/// silently allowing.
pub async fn classify_action(
    tool_name: &str,
    tool_input_json: &str,
    recent_transcript: &str,
    cancel: CancellationToken,
) -> Result<Option<ClassifierVerdict>> {
    let Some(model) = secondary_model::get_global() else {
        return Ok(None);
    };

    // The TS classifier ships the schema as a tool definition and reads
    // the tool-use block. We don't have tool-use plumbing on the
    // secondary-model path, so we ask for the same JSON inline.
    let composed = format!(
        "You are an automated security classifier for an AI coding agent. \
Read the action description and return EXACTLY one JSON object matching:\n\
\n\
{{\n  \"thinking\": \"<brief step-by-step reasoning>\",\n  \"shouldBlock\": <true|false>,\n  \"reason\": \"<one-line explanation>\"\n}}\n\
\n\
Block when the action could destroy data, exfiltrate secrets, modify \
systems outside the project, run untrusted code, or otherwise cause \
unrecoverable side effects. Allow read-only or trivially reversible \
edits inside the project working directory.\n\
\n\
Tool: {tool_name}\n\
Input: {tool_input_json}\n\
\n\
Recent transcript:\n\
{recent_transcript}\n\
\n\
Respond with the JSON object only. No prose, no fences."
    );

    let response = model.summarize(&composed, cancel).await?;
    Ok(Some(parse_verdict(&response)))
}

/// Parse the Haiku response into a verdict. Tolerates surrounding
/// fences / prose by extracting the first balanced JSON object. On
/// parse failure returns a conservative blocking verdict so callers
/// can route to interactive approval.
fn parse_verdict(response: &str) -> ClassifierVerdict {
    let trimmed = response.trim();
    if let Some(json) = extract_json_object(trimmed) {
        if let Ok(v) = serde_json::from_str::<ClassifierVerdict>(json) {
            return v;
        }
        // Try to coerce — sometimes the model emits extra fields, or
        // booleans as strings. Parse loosely.
        if let Ok(val) = serde_json::from_str::<Value>(json) {
            let thinking = val
                .get("thinking")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let should_block = val
                .get("shouldBlock")
                .and_then(|v| match v {
                    Value::Bool(b) => Some(*b),
                    Value::String(s) => match s.to_ascii_lowercase().as_str() {
                        "true" => Some(true),
                        "false" => Some(false),
                        _ => None,
                    },
                    _ => None,
                })
                .unwrap_or(true);
            let reason = val
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("(classifier response missing reason)")
                .to_string();
            return ClassifierVerdict {
                thinking,
                should_block,
                reason,
            };
        }
    }
    ClassifierVerdict {
        thinking: String::new(),
        should_block: true,
        reason: format!(
            "classifier returned unparseable response: {}",
            truncate(trimmed, 120)
        ),
    }
}

/// Extract the first JSON object substring (`{ ... }`) from a haystack.
/// Handles strings + escapes so braces inside string literals don't
/// confuse the matcher.
fn extract_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        if b == b'"' {
            in_str = true;
        } else if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(&s[start..=i]);
            }
        }
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut cut = max.min(s.len());
        while cut > 0 && !s.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}…", &s[..cut])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_response() {
        let raw = r#"{"thinking":"reads only","shouldBlock":false,"reason":"safe read"}"#;
        let v = parse_verdict(raw);
        assert!(!v.should_block);
        assert_eq!(v.reason, "safe read");
    }

    #[test]
    fn parses_with_surrounding_fences() {
        let raw =
            "```json\n{\"thinking\":\"x\",\"shouldBlock\":true,\"reason\":\"rm -rf root\"}\n```";
        let v = parse_verdict(raw);
        assert!(v.should_block);
    }

    #[test]
    fn coerces_string_boolean() {
        let raw = r#"{"thinking":"x","shouldBlock":"true","reason":"y"}"#;
        let v = parse_verdict(raw);
        assert!(v.should_block);
    }

    #[test]
    fn handles_braces_inside_strings() {
        let raw = r#"prefix
{"thinking":"input was {\"a\":1}","shouldBlock":false,"reason":"ok"}
suffix"#;
        let v = parse_verdict(raw);
        assert!(!v.should_block);
        assert!(v.thinking.contains("a"));
    }

    #[test]
    fn unparseable_response_blocks_conservatively() {
        let v = parse_verdict("totally not json");
        assert!(v.should_block);
        assert!(v.reason.contains("unparseable"));
    }

    #[tokio::test]
    async fn returns_none_without_secondary_model() {
        let out = classify_action(
            "Bash",
            r#"{"command":"ls"}"#,
            "user: hi",
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(out.is_none());
    }
}
