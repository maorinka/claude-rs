//! User keybindings loader. Port of `src/keybindings/loadUserBindings.ts`.
//!
//! Reads `~/.claude/keybindings.json` if present; merges user entries on top
//! of `defaults::default_bindings()`. Returns a flat Vec<ParsedBinding> plus
//! any validation warnings. Callers apply last-binding-wins semantics (user
//! entries come after defaults in the returned vec).
//!
//! The TS file-watcher (chokidar) is NOT ported — Rust callers that want
//! hot-reload should wire `notify-rs` or re-invoke this function on
//! user-initiated reload.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use super::defaults::default_bindings;
use super::matching::ParsedBinding;
use super::parser::{parse_bindings, KeybindingBlock};
use super::reserved::Severity;
use super::validate::{validate_bindings, KeybindingWarning, WarningType};

/// Result of loading keybindings: merged bindings + any warnings.
#[derive(Debug, Default)]
pub struct LoadResult {
    pub bindings: Vec<ParsedBinding>,
    pub warnings: Vec<KeybindingWarning>,
}

/// Path to the user keybindings file (matches TS `getKeybindingsPath`).
/// Honours `CLAUDE_CONFIG_DIR`, falls back to `~/.claude/keybindings.json`.
pub fn get_keybindings_path() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(dir).join("keybindings.json");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude")
        .join("keybindings.json")
}

fn parse_error(message: impl Into<String>, suggestion: Option<&str>) -> KeybindingWarning {
    KeybindingWarning {
        warning_type: WarningType::ParseError,
        severity: Severity::Error,
        message: message.into(),
        suggestion: suggestion.map(str::to_string),
    }
}

/// Convert a raw JSON `{"bindings": [{"context": ..., "bindings": {...}}, ...]}`
/// payload into typed `KeybindingBlock`s. Returns `Err(warning)` on invalid
/// shape so the caller can surface it.
fn blocks_from_json(raw: &serde_json::Value) -> Result<Vec<KeybindingBlock>, KeybindingWarning> {
    let bindings_field = match raw.get("bindings") {
        Some(v) => v,
        None => {
            return Err(parse_error(
                "keybindings.json must have a \"bindings\" array",
                Some("Use format: { \"bindings\": [ ... ] }"),
            ));
        }
    };
    let arr = match bindings_field.as_array() {
        Some(a) => a,
        None => {
            return Err(parse_error(
                "\"bindings\" must be an array",
                Some("Set \"bindings\" to an array of keybinding blocks"),
            ));
        }
    };

    let mut out = Vec::with_capacity(arr.len());
    for (i, block_json) in arr.iter().enumerate() {
        let obj = match block_json.as_object() {
            Some(o) => o,
            None => {
                return Err(parse_error(
                    format!("bindings[{i}] is not an object"),
                    Some("Each block must have \"context\" (string) and \"bindings\" (object)"),
                ));
            }
        };
        let context = obj
            .get("context")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                parse_error(
                    format!("bindings[{i}].context is missing or not a string"),
                    Some("Add a \"context\" string field"),
                )
            })?
            .to_string();
        let raw_map = obj
            .get("bindings")
            .and_then(|v| v.as_object())
            .ok_or_else(|| {
                parse_error(
                    format!("bindings[{i}].bindings is missing or not an object"),
                    Some("Add a \"bindings\" object mapping key -> action"),
                )
            })?;

        let mut bmap = BTreeMap::new();
        for (k, v) in raw_map {
            let action = v.as_str().ok_or_else(|| {
                parse_error(
                    format!("bindings[{i}].bindings[{k}] must be a string action"),
                    None,
                )
            })?;
            bmap.insert(k.clone(), action.to_string());
        }

        out.push(KeybindingBlock {
            context,
            bindings: bmap,
        });
    }
    Ok(out)
}

/// Load user keybindings from the given raw JSON string. Returns merged
/// (defaults + user) bindings plus warnings. Callers typically use
/// [`load_keybindings`] for the common case of reading from disk.
pub fn load_keybindings_from_str(contents: &str) -> LoadResult {
    let default_parsed = parse_bindings(&default_bindings());

    let parsed: serde_json::Value = match serde_json::from_str(contents) {
        Ok(v) => v,
        Err(e) => {
            return LoadResult {
                bindings: default_parsed,
                warnings: vec![parse_error(
                    format!("Failed to parse keybindings.json: {}", e),
                    None,
                )],
            };
        }
    };

    let blocks = match blocks_from_json(&parsed) {
        Ok(b) => b,
        Err(w) => {
            return LoadResult {
                bindings: default_parsed,
                warnings: vec![w],
            };
        }
    };

    let user_parsed = parse_bindings(&blocks);
    let validation = validate_bindings(&blocks);

    // Merge: defaults first, user entries after (last-wins at match time).
    let mut merged = default_parsed;
    merged.extend(user_parsed);

    LoadResult {
        bindings: merged,
        warnings: validation,
    }
}

/// Load user keybindings from disk. Missing file → defaults with no
/// warnings. Unreadable file → defaults with a parse_error warning.
pub fn load_keybindings() -> LoadResult {
    let path = get_keybindings_path();
    match fs::read_to_string(&path) {
        Ok(s) => load_keybindings_from_str(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => LoadResult {
            bindings: parse_bindings(&default_bindings()),
            warnings: Vec::new(),
        },
        Err(e) => LoadResult {
            bindings: parse_bindings(&default_bindings()),
            warnings: vec![parse_error(
                format!("Failed to read {}: {}", path.display(), e),
                None,
            )],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_yields_defaults() {
        let r = load_keybindings_from_str("{}");
        // `{}` has no "bindings" key → parse error warning, defaults returned.
        assert!(!r.bindings.is_empty());
        assert!(r
            .warnings
            .iter()
            .any(|w| matches!(w.warning_type, WarningType::ParseError)));
    }

    #[test]
    fn valid_user_block_merges() {
        let json = r#"{
            "bindings": [
                {
                    "context": "Chat",
                    "bindings": {
                        "ctrl+j": "chat:foo"
                    }
                }
            ]
        }"#;
        let r = load_keybindings_from_str(json);
        assert!(r.warnings.is_empty());
        assert!(r.bindings.iter().any(|b| b.action == "chat:foo"));
    }

    #[test]
    fn reserved_rebind_surfaces_warning() {
        let json = r#"{
            "bindings": [
                {
                    "context": "Global",
                    "bindings": {
                        "ctrl+c": "app:does-not-work"
                    }
                }
            ]
        }"#;
        let r = load_keybindings_from_str(json);
        assert!(r
            .warnings
            .iter()
            .any(|w| matches!(w.warning_type, WarningType::NonRebindable)));
    }

    #[test]
    fn bad_json_returns_parse_error() {
        let r = load_keybindings_from_str("not json at all");
        assert!(r
            .warnings
            .iter()
            .any(|w| w.message.contains("Failed to parse")));
    }

    #[test]
    fn bindings_not_array_yields_error() {
        let json = r#"{ "bindings": "not an array" }"#;
        let r = load_keybindings_from_str(json);
        assert!(r
            .warnings
            .iter()
            .any(|w| w.message.contains("must be an array")));
    }
}
