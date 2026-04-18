//! Parser for the Claude Code hints protocol.
//!
//! Port of the parser half of TS `src/utils/claudeCodeHints.ts`.
//! The TS file also owns a single-slot store backed by an
//! useSyncExternalStore signal — that reactive primitive isn't on
//! the Rust side yet, so the store half is deferred.
//!
//! Protocol: CLIs/SDKs running under Claude Code can emit a
//! self-closing `<claude-code-hint v="1" type="plugin"
//! value="..." />` tag to stderr/stdout. The harness scans tool
//! output for these tags, strips them before the output reaches
//! the model (they're a harness-only side channel), and surfaces
//! the hint to the user.
//!
//! Parser rules (match TS exactly):
//! - Outer match is whole-line anchored (leading/trailing blanks
//!   tolerated). A tag buried inside a larger line — e.g. a log
//!   line that quotes the tag — is ignored.
//! - Attributes accept `key="value"` and `key=value` (terminated
//!   by whitespace or the `/>` closing sequence). No escape
//!   sequences inside quoted values.
//! - Unknown / unsupported `v` or `type` → drop the hint (but
//!   still strip the line).
//! - Missing / empty `value` → drop the hint (still strip).
//! - After dropping lines, runs of ≥3 newlines collapse to 2 so
//!   the model doesn't see spurious vertical whitespace.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCodeHint {
    pub v: u32,
    pub hint_type: ClaudeCodeHintType,
    pub value: String,
    pub source_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeCodeHintType {
    Plugin,
}

impl ClaudeCodeHintType {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "plugin" => Some(Self::Plugin),
            _ => None,
        }
    }
}

const SUPPORTED_VERSIONS: &[u32] = &[1];

pub struct HintExtractionResult {
    pub hints: Vec<ClaudeCodeHint>,
    pub stripped: String,
}

/// Scan `output` for hint tags. Returns the parsed hints plus the
/// output with hint lines removed. `command`'s first whitespace-
/// separated token is recorded as each hint's `source_command`.
pub fn extract_claude_code_hints(output: &str, command: &str) -> HintExtractionResult {
    if !output.contains("<claude-code-hint") {
        return HintExtractionResult {
            hints: Vec::new(),
            stripped: output.to_string(),
        };
    }

    let source_command = first_command_token(command);
    let mut hints: Vec<ClaudeCodeHint> = Vec::new();
    let mut stripped_lines: Vec<String> = Vec::new();
    let mut any_stripped = false;

    for line in output.split_inclusive('\n') {
        let (body, newline) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        if let Some(hint) = parse_whole_line_hint(body, &source_command) {
            // Hint recognised — drop the line from output.
            any_stripped = true;
            if let Some(h) = hint {
                hints.push(h);
            }
            // Still drop the line; never re-emit.
            continue;
        }
        if let Some(()) = match_hint_tag_shape(body) {
            // Tag shape matched but parsing rejected (unsupported
            // v/type or empty value). Still strip the line — the
            // model shouldn't see the raw tag.
            any_stripped = true;
            let _ = ();
            continue;
        }
        stripped_lines.push(format!("{body}{newline}"));
    }

    let stripped: String = stripped_lines.concat();
    let final_output = if any_stripped {
        collapse_blank_runs(&stripped)
    } else {
        output.to_string()
    };

    HintExtractionResult {
        hints,
        stripped: final_output,
    }
}

/// Return `Some(Some(hint))` when the line is a valid hint,
/// `Some(None)` when the tag shape matched but the hint was
/// rejected (still causes the line to be stripped), `None` when
/// the line doesn't look like a hint at all.
fn parse_whole_line_hint(
    line: &str,
    source_command: &str,
) -> Option<Option<ClaudeCodeHint>> {
    let body = line.trim_matches([' ', '\t']);
    let attr_body = body.strip_prefix("<claude-code-hint")?;
    let attr_body = attr_body.strip_suffix("/>")?;
    // Between the open and `/>`, we require at least one whitespace
    // before attrs (matching TS `<claude-code-hint\s+`).
    let attr_body = attr_body.trim_matches([' ', '\t']);
    if attr_body.is_empty() {
        return Some(None);
    }
    // Must have started with whitespace before attrs, not a letter.
    // Since `trim_matches` consumed it, re-check the original shape:
    // re-split on the first whitespace after the element name.
    if !line.trim_start().starts_with("<claude-code-hint ")
        && !line.trim_start().starts_with("<claude-code-hint\t")
    {
        return None;
    }

    let attrs = parse_attrs(attr_body);
    let v = attrs
        .iter()
        .find_map(|(k, v)| (k == "v").then_some(v.as_str()))
        .and_then(|s| s.parse::<u32>().ok());
    let hint_type = attrs
        .iter()
        .find_map(|(k, v)| (k == "type").then_some(v.as_str()));
    let value = attrs
        .iter()
        .find_map(|(k, v)| (k == "value").then_some(v.as_str()))
        .unwrap_or("");

    let Some(v) = v else {
        return Some(None);
    };
    if !SUPPORTED_VERSIONS.contains(&v) {
        return Some(None);
    }
    let Some(hint_type) = hint_type.and_then(ClaudeCodeHintType::from_str) else {
        return Some(None);
    };
    if value.is_empty() {
        return Some(None);
    }

    Some(Some(ClaudeCodeHint {
        v,
        hint_type,
        value: value.to_string(),
        source_command: source_command.to_string(),
    }))
}

/// Does this line have the exact `<claude-code-hint ... />` shape
/// (anchored, leading/trailing whitespace tolerated)? Used to strip
/// lines whose tag was well-formed but whose content we rejected.
fn match_hint_tag_shape(line: &str) -> Option<()> {
    let body = line.trim_matches([' ', '\t']);
    if !body.starts_with("<claude-code-hint") {
        return None;
    }
    if !body.ends_with("/>") {
        return None;
    }
    let after_tag = &body["<claude-code-hint".len()..];
    let first_char = after_tag.chars().next()?;
    if !first_char.is_whitespace() && first_char != '/' {
        return None;
    }
    Some(())
}

fn parse_attrs(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let bytes = body.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        // Skip whitespace.
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        // Key: `\w+` (alphanumeric + underscore).
        let key_start = i;
        while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if i == key_start {
            i += 1;
            continue;
        }
        let key = std::str::from_utf8(&bytes[key_start..i]).unwrap_or("");
        if i >= n || bytes[i] != b'=' {
            continue;
        }
        i += 1; // consume '='
        if i >= n {
            out.push((key.to_string(), String::new()));
            break;
        }
        let value = if bytes[i] == b'"' {
            i += 1;
            let start = i;
            while i < n && bytes[i] != b'"' {
                i += 1;
            }
            let v = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
            if i < n {
                i += 1;
            }
            v.to_string()
        } else {
            let start = i;
            while i < n
                && !bytes[i].is_ascii_whitespace()
                && !(bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'>')
            {
                i += 1;
            }
            let v = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
            v.to_string()
        };
        out.push((key.to_string(), value));
    }
    out
}

fn first_command_token(command: &str) -> String {
    let trimmed = command.trim();
    if let Some(idx) = trimmed.find(char::is_whitespace) {
        trimmed[..idx].to_string()
    } else {
        trimmed.to_string()
    }
}

fn collapse_blank_runs(s: &str) -> String {
    // Replace runs of ≥3 newlines with exactly 2.
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        if bytes[i] == b'\n' {
            let mut j = i;
            while j < n && bytes[j] == b'\n' {
                j += 1;
            }
            let run = j - i;
            if run >= 3 {
                out.push_str("\n\n");
            } else {
                for _ in 0..run {
                    out.push('\n');
                }
            }
            i = j;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(out: &str, cmd: &str) -> HintExtractionResult {
        extract_claude_code_hints(out, cmd)
    }

    #[test]
    fn no_tag_fast_path() {
        let r = extract("no hints here", "pkg install foo");
        assert!(r.hints.is_empty());
        assert_eq!(r.stripped, "no hints here");
    }

    #[test]
    fn parses_valid_plugin_hint() {
        let out =
            "plugin installed\n<claude-code-hint v=\"1\" type=\"plugin\" value=\"foo@marketplace\" />\ndone\n";
        let r = extract(out, "pkg install foo --flag");
        assert_eq!(r.hints.len(), 1);
        let h = &r.hints[0];
        assert_eq!(h.v, 1);
        assert_eq!(h.hint_type, ClaudeCodeHintType::Plugin);
        assert_eq!(h.value, "foo@marketplace");
        assert_eq!(h.source_command, "pkg");
        assert_eq!(r.stripped, "plugin installed\ndone\n");
    }

    #[test]
    fn strips_unknown_version() {
        let out = "<claude-code-hint v=\"99\" type=\"plugin\" value=\"x\" />\n";
        let r = extract(out, "pkg");
        assert!(r.hints.is_empty());
        assert!(!r.stripped.contains("claude-code-hint"));
    }

    #[test]
    fn strips_unknown_type() {
        let out = "<claude-code-hint v=\"1\" type=\"bogus\" value=\"x\" />\n";
        let r = extract(out, "pkg");
        assert!(r.hints.is_empty());
        assert!(!r.stripped.contains("claude-code-hint"));
    }

    #[test]
    fn strips_empty_value() {
        let out = "<claude-code-hint v=\"1\" type=\"plugin\" value=\"\" />\n";
        let r = extract(out, "pkg");
        assert!(r.hints.is_empty());
        assert!(!r.stripped.contains("claude-code-hint"));
    }

    #[test]
    fn tolerates_leading_whitespace() {
        let out = "  <claude-code-hint v=\"1\" type=\"plugin\" value=\"z\" />  \n";
        let r = extract(out, "pkg");
        assert_eq!(r.hints.len(), 1);
        assert_eq!(r.hints[0].value, "z");
    }

    #[test]
    fn buried_tag_in_log_is_ignored() {
        // Tag not whole-line → stays in output, not parsed.
        let out = "debug: saw <claude-code-hint v=\"1\" type=\"plugin\" value=\"x\" /> on stderr\n";
        let r = extract(out, "pkg");
        assert!(r.hints.is_empty());
        assert!(r.stripped.contains("<claude-code-hint"));
    }

    #[test]
    fn accepts_unquoted_attribute_value() {
        let out = "<claude-code-hint v=1 type=plugin value=simple />\n";
        let r = extract(out, "pkg");
        assert_eq!(r.hints.len(), 1);
        assert_eq!(r.hints[0].value, "simple");
    }

    #[test]
    fn collapses_blank_runs_after_strip() {
        let out = "a\n\n<claude-code-hint v=\"1\" type=\"plugin\" value=\"v\" />\n\nb\n";
        let r = extract(out, "pkg");
        assert_eq!(r.hints.len(), 1);
        // Stripped line leaves a run that gets collapsed to "\n\n".
        assert_eq!(r.stripped, "a\n\nb\n");
    }

    #[test]
    fn source_command_is_first_token() {
        let out = "<claude-code-hint v=\"1\" type=\"plugin\" value=\"v\" />\n";
        let r = extract(out, "  sudo   npm install foo  ");
        assert_eq!(r.hints[0].source_command, "sudo");
    }

    #[test]
    fn missing_v_is_rejected() {
        let out = "<claude-code-hint type=\"plugin\" value=\"x\" />\n";
        let r = extract(out, "pkg");
        assert!(r.hints.is_empty());
    }

    #[test]
    fn missing_type_is_rejected() {
        let out = "<claude-code-hint v=\"1\" value=\"x\" />\n";
        let r = extract(out, "pkg");
        assert!(r.hints.is_empty());
    }
}
