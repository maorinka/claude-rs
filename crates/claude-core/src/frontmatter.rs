//! Shared YAML frontmatter parser + related helpers.
//!
//! Consolidates the frontmatter handling used by skills, output styles,
//! agents, and memory files. Port of the subset of TS
//! `src/utils/frontmatterParser.ts` that doesn't require a full YAML
//! implementation:
//!
//!   - `parse_frontmatter(markdown)` — strip the `---\n...\n---\n`
//!     header, return (map, body). Supports scalars, inline arrays
//!     `[a, b]`, and block sequences.
//!   - `split_path_in_frontmatter(input)` — comma-split + brace-
//!     expand a glob pattern. `"src/*.{ts,tsx}"` → `["src/*.ts",
//!     "src/*.tsx"]`.
//!   - `parse_boolean_frontmatter`, `coerce_description_to_string`,
//!     `parse_positive_int_from_frontmatter` — small type coercers.
//!
//! Nested mappings are NOT supported — the TS FrontmatterData surface
//! is flat, so nothing in the tree actually needs them today. If a
//! future skill spec adds `permissions: { allow: [...] }`-style
//! structure we'll swap in a real YAML crate.

use std::collections::BTreeMap;

/// A single frontmatter value. Matches the narrow shape TS actually
/// uses: scalars, lists, and missing fields.
#[derive(Debug, Clone, PartialEq)]
pub enum FrontmatterValue {
    String(String),
    Bool(bool),
    Number(f64),
    List(Vec<FrontmatterValue>),
    Null,
}

impl FrontmatterValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FrontmatterValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            FrontmatterValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_number(&self) -> Option<f64> {
        match self {
            FrontmatterValue::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[FrontmatterValue]> {
        match self {
            FrontmatterValue::List(v) => Some(v.as_slice()),
            _ => None,
        }
    }
}

pub type Frontmatter = BTreeMap<String, FrontmatterValue>;

/// Parsed markdown: separated frontmatter map + body text.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMarkdown {
    pub frontmatter: Frontmatter,
    pub content: String,
}

/// Parse markdown content. If the first non-BOM line is `---`, read
/// until the next `---` line as the YAML frontmatter block; otherwise
/// return an empty map + the whole string as content. Matches TS
/// `parseFrontmatter` and the existing simpler parser in
/// `output_styles::parse_frontmatter` — callers can migrate to this
/// when they need typed values beyond bare strings.
pub fn parse_frontmatter(markdown: &str) -> ParsedMarkdown {
    let trimmed = markdown.strip_prefix('\u{FEFF}').unwrap_or(markdown);
    if !trimmed.starts_with("---\n") && !trimmed.starts_with("---\r\n") {
        return ParsedMarkdown {
            frontmatter: BTreeMap::new(),
            content: markdown.to_string(),
        };
    }

    let after_open = trimmed.splitn(2, '\n').nth(1).unwrap_or("");
    let close_pos = after_open
        .find("\n---\n")
        .or_else(|| after_open.find("\n---\r\n"))
        .or_else(|| {
            if after_open.ends_with("\n---") {
                Some(after_open.len() - 4)
            } else {
                None
            }
        });

    let Some(close) = close_pos else {
        return ParsedMarkdown {
            frontmatter: BTreeMap::new(),
            content: markdown.to_string(),
        };
    };

    let yaml = &after_open[..close];
    let body_start = close + "\n---\n".len();
    let body = if body_start <= after_open.len() {
        &after_open[body_start.min(after_open.len())..]
    } else {
        ""
    };

    ParsedMarkdown {
        frontmatter: parse_yaml_block(yaml),
        content: body.to_string(),
    }
}

/// Minimal YAML block parser — scalars + inline lists + block
/// sequences. See module doc for non-goals.
fn parse_yaml_block(src: &str) -> Frontmatter {
    let mut out = BTreeMap::new();
    let mut lines = src.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        // Skip blanks + full-line comments.
        if trimmed.is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }
        // Only accept top-level key lines (no leading whitespace).
        let indent_len = line.len() - line.trim_start().len();
        if indent_len != 0 {
            continue;
        }
        let Some((raw_key, raw_val)) = trimmed.split_once(':') else {
            continue;
        };
        let key = raw_key.trim().to_string();
        if key.is_empty() {
            continue;
        }
        let val = raw_val.trim();

        // Block sequence: `key:` with list items following.
        if val.is_empty() {
            let mut items = Vec::new();
            while let Some(peek) = lines.peek() {
                let p = *peek;
                if let Some(rest) = p.trim_start().strip_prefix("- ") {
                    items.push(parse_scalar(rest.trim()));
                    lines.next();
                } else if p.trim().is_empty() {
                    lines.next();
                } else {
                    break;
                }
            }
            out.insert(key, FrontmatterValue::List(items));
            continue;
        }

        // Inline list: `key: [a, b, c]`.
        if val.starts_with('[') && val.ends_with(']') {
            let inner = &val[1..val.len() - 1];
            let items: Vec<FrontmatterValue> = split_respecting_braces(inner)
                .into_iter()
                .map(|s| parse_scalar(s.trim()))
                .collect();
            out.insert(key, FrontmatterValue::List(items));
            continue;
        }

        out.insert(key, parse_scalar(val));
    }

    out
}

fn parse_scalar(raw: &str) -> FrontmatterValue {
    let s = raw;
    if s.is_empty() || s == "null" || s == "~" {
        return FrontmatterValue::Null;
    }
    match s.to_ascii_lowercase().as_str() {
        "true" | "yes" => return FrontmatterValue::Bool(true),
        "false" | "no" => return FrontmatterValue::Bool(false),
        _ => {}
    }
    // Strip surrounding quotes.
    let unquoted = if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        &s[1..s.len() - 1]
    } else {
        s
    };
    if let Ok(n) = unquoted.parse::<f64>() {
        return FrontmatterValue::Number(n);
    }
    FrontmatterValue::String(unquoted.to_string())
}

fn split_respecting_braces(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for c in input.chars() {
        match c {
            '{' | '[' => {
                depth += 1;
                cur.push(c);
            }
            '}' | ']' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Expand `{a,b}` brace patterns in a glob. Mirrors TS `expandBraces`.
fn expand_braces(pattern: &str) -> Vec<String> {
    // Find the first `{...}` group (no nesting support — matches TS).
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let Some(close) = pattern[open..].find('}').map(|i| i + open) else {
        return vec![pattern.to_string()];
    };
    let prefix = &pattern[..open];
    let group = &pattern[open + 1..close];
    let suffix = &pattern[close + 1..];

    let alternatives: Vec<&str> = group.split(',').map(|s| s.trim()).collect();
    let mut out = Vec::new();
    for alt in alternatives {
        let expanded_suffix = expand_braces(&format!("{}{}", alt, suffix));
        for s in expanded_suffix {
            out.push(format!("{}{}", prefix, s));
        }
    }
    out
}

/// Comma-split a paths frontmatter value and expand brace patterns.
/// Accepts either a string like `"src/*.ts, docs/**/*.md"` or a list.
pub fn split_path_in_frontmatter(value: &FrontmatterValue) -> Vec<String> {
    match value {
        FrontmatterValue::List(items) => items
            .iter()
            .flat_map(split_path_in_frontmatter)
            .collect(),
        FrontmatterValue::String(s) => {
            let parts = split_respecting_braces(s);
            parts
                .into_iter()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .flat_map(|p| expand_braces(&p))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Coerce a frontmatter value to a description string. Falls back to
/// `default_value` when the field is absent or not stringifiable.
/// Matches TS `coerceDescriptionToString`.
pub fn coerce_description_to_string(
    v: Option<&FrontmatterValue>,
    default_value: &str,
) -> String {
    match v {
        Some(FrontmatterValue::String(s)) if !s.is_empty() => s.clone(),
        Some(FrontmatterValue::Number(n)) => n.to_string(),
        Some(FrontmatterValue::Bool(b)) => b.to_string(),
        _ => default_value.to_string(),
    }
}

/// Parse a truthy/falsy frontmatter value. Matches TS
/// `parseBooleanFrontmatter` — strings `"true"`/`"false"` work, bools
/// pass through, numbers 1/0 coerce, everything else is false.
pub fn parse_boolean_frontmatter(v: &FrontmatterValue) -> bool {
    match v {
        FrontmatterValue::Bool(b) => *b,
        FrontmatterValue::String(s) => matches!(s.to_ascii_lowercase().as_str(), "true" | "yes" | "1" | "on"),
        FrontmatterValue::Number(n) => *n != 0.0,
        _ => false,
    }
}

/// Parse a positive integer from a frontmatter value. Matches TS
/// `parsePositiveIntFromFrontmatter` — returns `None` if missing, not
/// an integer, or <= 0.
pub fn parse_positive_int_from_frontmatter(v: Option<&FrontmatterValue>) -> Option<u64> {
    let n = match v? {
        FrontmatterValue::Number(n) => *n,
        FrontmatterValue::String(s) => s.parse::<f64>().ok()?,
        _ => return None,
    };
    if n.fract() != 0.0 || n <= 0.0 {
        return None;
    }
    Some(n as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_frontmatter() {
        let m = parse_frontmatter(
            "---\nname: Test\ndescription: A test\n---\n\nBody here.\n",
        );
        assert_eq!(m.frontmatter.get("name").unwrap().as_str(), Some("Test"));
        assert_eq!(
            m.frontmatter.get("description").unwrap().as_str(),
            Some("A test")
        );
        assert!(m.content.starts_with("\nBody"));
    }

    #[test]
    fn inline_list_parsed() {
        let m = parse_frontmatter("---\ntags: [a, b, c]\n---\n");
        let tags = m.frontmatter.get("tags").unwrap().as_list().unwrap();
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0].as_str(), Some("a"));
    }

    #[test]
    fn block_sequence_parsed() {
        let m = parse_frontmatter("---\ntags:\n  - a\n  - b\n---\n");
        let tags = m.frontmatter.get("tags").unwrap().as_list().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[1].as_str(), Some("b"));
    }

    #[test]
    fn no_frontmatter_returns_whole_body() {
        let m = parse_frontmatter("# Heading\n\nbody");
        assert!(m.frontmatter.is_empty());
        assert_eq!(m.content, "# Heading\n\nbody");
    }

    #[test]
    fn booleans_and_numbers_parsed() {
        let m = parse_frontmatter("---\nenabled: true\ncount: 42\n---\n");
        assert_eq!(m.frontmatter.get("enabled").unwrap().as_bool(), Some(true));
        assert_eq!(m.frontmatter.get("count").unwrap().as_number(), Some(42.0));
    }

    #[test]
    fn quoted_string_keeps_special_chars() {
        let m = parse_frontmatter(
            "---\npaths: \"src/*.{ts,tsx}\"\n---\n",
        );
        assert_eq!(
            m.frontmatter.get("paths").unwrap().as_str(),
            Some("src/*.{ts,tsx}")
        );
    }

    #[test]
    fn split_path_expands_brace() {
        let paths = split_path_in_frontmatter(&FrontmatterValue::String(
            "src/*.{ts,tsx}".into(),
        ));
        assert_eq!(paths, vec!["src/*.ts", "src/*.tsx"]);
    }

    #[test]
    fn split_path_expands_cross_product() {
        let paths = split_path_in_frontmatter(&FrontmatterValue::String(
            "{a,b}/{c,d}".into(),
        ));
        assert_eq!(paths.len(), 4);
        assert!(paths.contains(&"a/c".to_string()));
        assert!(paths.contains(&"b/d".to_string()));
    }

    #[test]
    fn split_path_handles_list_input() {
        let value = FrontmatterValue::List(vec![
            FrontmatterValue::String("a".into()),
            FrontmatterValue::String("src/*.{ts,tsx}".into()),
        ]);
        let paths = split_path_in_frontmatter(&value);
        assert_eq!(paths, vec!["a", "src/*.ts", "src/*.tsx"]);
    }

    #[test]
    fn coerce_description_fallback() {
        assert_eq!(coerce_description_to_string(None, "fallback"), "fallback");
        assert_eq!(
            coerce_description_to_string(Some(&FrontmatterValue::String("x".into())), "f"),
            "x"
        );
    }

    #[test]
    fn parse_boolean_variants() {
        assert!(parse_boolean_frontmatter(&FrontmatterValue::Bool(true)));
        assert!(parse_boolean_frontmatter(&FrontmatterValue::String("yes".into())));
        assert!(!parse_boolean_frontmatter(&FrontmatterValue::String("no".into())));
        assert!(parse_boolean_frontmatter(&FrontmatterValue::Number(1.0)));
        assert!(!parse_boolean_frontmatter(&FrontmatterValue::Null));
    }

    #[test]
    fn parse_positive_int_valid() {
        let v = FrontmatterValue::Number(5.0);
        assert_eq!(parse_positive_int_from_frontmatter(Some(&v)), Some(5));
    }

    #[test]
    fn parse_positive_int_rejects_negative_and_float() {
        assert!(parse_positive_int_from_frontmatter(Some(&FrontmatterValue::Number(-1.0)))
            .is_none());
        assert!(parse_positive_int_from_frontmatter(Some(&FrontmatterValue::Number(1.5)))
            .is_none());
        assert!(parse_positive_int_from_frontmatter(None).is_none());
    }
}
