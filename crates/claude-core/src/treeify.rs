//! ASCII tree renderer for nested JSON-shaped data.
//!
//! Port of TS `utils/treeify.ts:1-170`.
//!
//! Scope reductions from the TS module
//! ===================================
//! - TS threads ink theme colours (`treeCharColors.treeChar` /
//!   `.key` / `.value`) through every line. The Rust port returns
//!   plain text — callers that want colour post-process with
//!   `crossterm::style` / `owo-colors`. No UI callers in the Rust
//!   tree currently render this, and plumbing a theme through would
//!   force a `claude-core` → UI-crate dep we don't want.
//! - TS handles `[Function]`. JSON has no functions, so that branch
//!   is elided.
//! - Circular references can't occur in `serde_json::Value` (strict
//!   tree), so the TS `visited` WeakSet is not needed.

use serde_json::{Map, Value};

const BRANCH: &str = "├";
const LAST_BRANCH: &str = "└";
const LINE: &str = "│";
const EMPTY: &str = " ";

#[derive(Debug, Clone, Default)]
pub struct TreeifyOptions {
    /// When `true`, leaf scalars render as `key: value`. When `false`,
    /// only the key is emitted. Matches TS `showValues`.
    pub show_values: bool,
}

impl TreeifyOptions {
    /// Default TS behaviour: render scalar values inline.
    pub fn with_values() -> Self {
        Self { show_values: true }
    }
}

/// Render a JSON-shaped object as an ASCII tree.
///
/// Only the top-level object is traversed as a tree — scalar top-level
/// values return their string representation. Arrays render as
/// `[Array(N)]` summaries (matches TS `utils/treeify.ts:127-131`), not
/// as expanded sub-trees, because TS uses tree output for settings /
/// config shapes where arrays are always small leaf payloads.
pub fn treeify(obj: &Value, options: &TreeifyOptions) -> String {
    match obj {
        Value::Object(map) => render_object(map, options),
        // Non-object root — just stringify. TS `treeify` is typed to
        // accept only `TreeNode` (object), but callers historically
        // pass strings through (empty message fall-back at
        // treeify.ts:62). Mirror that grace.
        other => scalar_to_string(other),
    }
}

fn render_object(map: &Map<String, Value>, options: &TreeifyOptions) -> String {
    if map.is_empty() {
        return "(empty)".to_owned();
    }

    // TS single-empty-string-key special case (treeify.ts:153-166): a
    // tree with a single `{"": "some string"}` entry renders as
    // `└ some string` (used for status-line messages).
    if map.len() == 1 {
        let (k, v) = map.iter().next().unwrap();
        if k.trim().is_empty() {
            if let Value::String(s) = v {
                return format!("{LAST_BRANCH} {s}");
            }
        }
    }

    let mut lines: Vec<String> = Vec::new();
    grow_branch(map, "", 0, options, &mut lines);
    lines.join("\n")
}

fn grow_branch(
    map: &Map<String, Value>,
    prefix: &str,
    depth: usize,
    options: &TreeifyOptions,
    lines: &mut Vec<String>,
) {
    let keys: Vec<&String> = map.keys().collect();
    let last = keys.len().saturating_sub(1);

    for (i, key) in keys.iter().enumerate() {
        let value = &map[*key];
        let is_last = i == last;
        let node_prefix = if depth == 0 && i == 0 { "" } else { prefix };
        let tree_char = if is_last { LAST_BRANCH } else { BRANCH };

        let key_part = if key.trim().is_empty() {
            String::new()
        } else {
            (*key).clone()
        };
        let should_add_colon = !key_part.is_empty();

        let mut line = String::new();
        line.push_str(node_prefix);
        line.push_str(tree_char);
        if !key_part.is_empty() {
            line.push(' ');
            line.push_str(&key_part);
        }

        match value {
            Value::Object(inner) => {
                lines.push(line);
                let continuation = if is_last { EMPTY } else { LINE };
                let next_prefix = format!("{node_prefix}{continuation} ");
                grow_branch(inner, &next_prefix, depth + 1, options, lines);
            }
            Value::Array(arr) => {
                push_scalar_line(&mut line, should_add_colon, &format!("[Array({})]", arr.len()));
                lines.push(line);
            }
            _ if options.show_values => {
                let val = scalar_to_string(value);
                push_scalar_line(&mut line, should_add_colon, &val);
                lines.push(line);
            }
            _ => {
                lines.push(line);
            }
        }
    }
}

fn push_scalar_line(line: &mut String, should_add_colon: bool, value: &str) {
    if should_add_colon {
        line.push_str(": ");
    } else if !line.is_empty() {
        line.push(' ');
    }
    line.push_str(value);
}

fn scalar_to_string(v: &Value) -> String {
    match v {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        // Shouldn't reach here from `grow_branch` (arrays / objects
        // handled up-stream), but defensively stringify.
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_object_returns_placeholder() {
        assert_eq!(treeify(&json!({}), &TreeifyOptions::with_values()), "(empty)");
    }

    #[test]
    fn scalar_root_passes_through() {
        assert_eq!(treeify(&json!("hi"), &TreeifyOptions::with_values()), "hi");
        assert_eq!(treeify(&json!(42), &TreeifyOptions::with_values()), "42");
        assert_eq!(treeify(&json!(null), &TreeifyOptions::with_values()), "null");
    }

    #[test]
    fn single_empty_key_renders_just_value() {
        let out = treeify(&json!({ "": "status text" }), &TreeifyOptions::with_values());
        assert_eq!(out, "└ status text");
    }

    #[test]
    fn flat_object_with_values() {
        let out = treeify(
            &json!({ "a": 1, "b": true }),
            &TreeifyOptions::with_values(),
        );
        // serde_json preserves insertion order for `!json` macro → stable.
        assert!(out.contains("├ a: 1"), "output: {out}");
        assert!(out.contains("└ b: true"), "output: {out}");
    }

    #[test]
    fn flat_object_without_values() {
        let out = treeify(&json!({ "a": 1, "b": 2 }), &TreeifyOptions::default());
        // show_values=false → no `:` suffix.
        assert!(!out.contains(':'), "output: {out}");
        assert!(out.contains("├ a"));
        assert!(out.contains("└ b"));
    }

    #[test]
    fn nested_object_uses_line_and_empty_continuations() {
        let out = treeify(
            &json!({
                "outer": {
                    "a": 1,
                    "b": 2,
                },
                "trailing": "x",
            }),
            &TreeifyOptions::with_values(),
        );
        // `outer` is NOT the last key → continuation uses `│`.
        assert!(out.contains("│"), "output: {out}");
        // `trailing` IS last → would use empty continuation if nested.
        assert!(out.contains("└ trailing: x"), "output: {out}");
    }

    #[test]
    fn arrays_render_as_count_summary() {
        let out = treeify(
            &json!({ "items": [1, 2, 3] }),
            &TreeifyOptions::with_values(),
        );
        assert!(out.contains("[Array(3)]"), "output: {out}");
    }

    #[test]
    fn empty_array_renders_count_zero() {
        let out = treeify(&json!({ "items": [] }), &TreeifyOptions::with_values());
        assert!(out.contains("[Array(0)]"));
    }

    #[test]
    fn null_scalar_renders_as_null() {
        let out = treeify(&json!({ "maybe": null }), &TreeifyOptions::with_values());
        assert!(out.contains("└ maybe: null"));
    }

    #[test]
    fn deeply_nested_shows_proper_indent() {
        let out = treeify(
            &json!({
                "a": {
                    "b": {
                        "c": 1
                    }
                }
            }),
            &TreeifyOptions::with_values(),
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        // Leaf line must start with two levels of continuation space.
        assert!(
            lines[2].starts_with("  ") || lines[2].starts_with(LAST_BRANCH),
            "unexpected depth-3 line: {}",
            lines[2]
        );
    }

    #[test]
    fn key_without_trim_treated_as_scalar_positional() {
        // Pure whitespace key: value appears inline without a `:`.
        // (TS doesn't add `:` when the key trims to empty.)
        let out = treeify(&json!({ "   ": "x" }), &TreeifyOptions::with_values());
        assert!(!out.contains(':'), "output: {out}");
        assert!(out.contains("x"));
    }
}
