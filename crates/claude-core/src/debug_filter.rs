//! Debug message category filtering.
//!
//! Port of TS `src/utils/debugFilter.ts`. Used by the `--debug`
//! CLI flag so operators can narrow noisy traces to a specific
//!   subsystem:
//! - `--debug api,hooks` → only messages in those categories.
//! - `--debug !file,!1p` → everything except those.
//! - Mixed include + exclude is rejected (returns None → "show all")
//!   because the behaviour is ambiguous.
//!
//! The TS `memoize` cache is intentionally NOT ported — call sites
//! parse the filter once at startup and keep the result. Callers
//! that need memoisation can wrap their own.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugFilter {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub is_exclusive: bool,
}

/// Parse a comma-separated filter string into a `DebugFilter`.
/// Returns `None` when the filter is empty, blank, or mixes include
/// + exclude (that mode is treated as "show all" per TS).
pub fn parse_debug_filter(filter_string: Option<&str>) -> Option<DebugFilter> {
    let filter_string = filter_string?;
    if filter_string.trim().is_empty() {
        return None;
    }

    let filters: Vec<String> = filter_string
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if filters.is_empty() {
        return None;
    }

    let has_exclusive = filters.iter().any(|f| f.starts_with('!'));
    let has_inclusive = filters.iter().any(|f| !f.starts_with('!'));
    if has_exclusive && has_inclusive {
        return None;
    }

    let clean: Vec<String> = filters
        .iter()
        .map(|f| f.trim_start_matches('!').to_lowercase())
        .collect();

    Some(DebugFilter {
        include: if has_exclusive {
            Vec::new()
        } else {
            clean.clone()
        },
        exclude: if has_exclusive { clean } else { Vec::new() },
        is_exclusive: has_exclusive,
    })
}

/// Extract debug categories from `message`. Patterns (in order):
///  1. `MCP server "name" ...` → `["mcp", name]` (checked first so
///     a plain-colon pattern doesn't misfire).
///  2. `category: ...` (no `[` before the colon) → `[category]`.
///  3. `[CATEGORY] ...` → `[category]`.
///  4. `... 1p event: ...` → adds `"1p"`.
///  5. ` : secondary (type|mode|status|event):` → adds secondary
///     when it's a reasonable category name (<30 chars, no space).
///
/// Categories are lowercased and deduplicated (first-occurrence
/// order preserved).
pub fn extract_debug_categories(message: &str) -> Vec<String> {
    let mut categories: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let push = |c: String, cats: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        if !c.is_empty() && seen.insert(c.clone()) {
            cats.push(c);
        }
    };

    // Pattern 1: MCP server "name"
    let mcp = try_match_mcp(message);
    if let Some(name) = mcp {
        push("mcp".to_string(), &mut categories, &mut seen);
        push(name.to_lowercase(), &mut categories, &mut seen);
    } else if let Some(prefix) = try_match_leading_colon_prefix(message) {
        push(prefix.trim().to_lowercase(), &mut categories, &mut seen);
    }

    // Pattern 3: [CATEGORY] at start.
    if let Some(b) = try_match_leading_bracket(message) {
        push(b.trim().to_lowercase(), &mut categories, &mut seen);
    }

    // Pattern 4: "1p event:" anywhere (case-insensitive).
    if message.to_lowercase().contains("1p event:") {
        push("1p".to_string(), &mut categories, &mut seen);
    }

    // Pattern 5: secondary.
    if let Some(sec) = try_match_secondary(message) {
        let s = sec.trim().to_lowercase();
        if s.len() < 30 && !s.contains(' ') {
            push(s, &mut categories, &mut seen);
        }
    }

    categories
}

/// Decide whether a message with the given extracted `categories`
/// passes `filter`. `None` filter ⇒ always visible.
pub fn should_show_debug_categories(categories: &[String], filter: Option<&DebugFilter>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    if categories.is_empty() {
        // Uncategorised: excluded in both modes (exclusive for
        // safety; inclusive by definition).
        return false;
    }
    if filter.is_exclusive {
        !categories.iter().any(|c| filter.exclude.contains(c))
    } else {
        categories.iter().any(|c| filter.include.contains(c))
    }
}

/// Combined helper: extract categories then apply the filter.
pub fn should_show_debug_message(message: &str, filter: Option<&DebugFilter>) -> bool {
    if filter.is_none() {
        return true;
    }
    let cats = extract_debug_categories(message);
    should_show_debug_categories(&cats, filter)
}

fn try_match_mcp(message: &str) -> Option<&str> {
    // `^MCP server ["']([^"']+)["']`
    let rest = message.strip_prefix("MCP server ")?;
    let (quote_char, rest) = match rest.chars().next()? {
        c @ ('"' | '\'') => (c, &rest[1..]),
        _ => return None,
    };
    let end = rest.find(quote_char)?;
    if end == 0 {
        return None;
    }
    Some(&rest[..end])
}

fn try_match_leading_colon_prefix(message: &str) -> Option<&str> {
    // `^([^:[]+):`
    let mut end = None;
    for (i, c) in message.char_indices() {
        if c == ':' {
            end = Some(i);
            break;
        }
        if c == '[' {
            return None;
        }
    }
    let end = end?;
    if end == 0 {
        return None;
    }
    Some(&message[..end])
}

fn try_match_leading_bracket(message: &str) -> Option<&str> {
    // `^\[([^\]]+)]`
    let rest = message.strip_prefix('[')?;
    let end = rest.find(']')?;
    if end == 0 {
        return None;
    }
    Some(&rest[..end])
}

fn try_match_secondary(message: &str) -> Option<&str> {
    // TS: `:\s*([^:]+?)(?:\s+(?:type|mode|status|event))?:`
    // Find a `: X:` window, then strip an optional trailing
    // " <keyword>" off X so callers don't treat the keyword itself
    // as the category name.
    let first = message.find(':')?;
    let after = &message[first + 1..];
    let leading_ws = after.len() - after.trim_start_matches([' ', '\t']).len();
    let body_start = first + 1 + leading_ws;
    let body = &message[body_start..];
    let close_rel = body.find(':')?;
    let mut token = &body[..close_rel];
    for kw in [" type", " mode", " status", " event"] {
        if let Some(s) = token.strip_suffix(kw) {
            token = s;
            break;
        }
    }
    let token = token.trim_end();
    if token.is_empty() {
        return None;
    }
    Some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_none_for_empty() {
        assert!(parse_debug_filter(None).is_none());
        assert!(parse_debug_filter(Some("")).is_none());
        assert!(parse_debug_filter(Some("   ")).is_none());
    }

    #[test]
    fn parse_inclusive_list() {
        let f = parse_debug_filter(Some("api,hooks")).unwrap();
        assert!(!f.is_exclusive);
        assert_eq!(f.include, vec!["api", "hooks"]);
        assert!(f.exclude.is_empty());
    }

    #[test]
    fn parse_exclusive_list_strips_bang() {
        let f = parse_debug_filter(Some("!1p,!file")).unwrap();
        assert!(f.is_exclusive);
        assert_eq!(f.exclude, vec!["1p", "file"]);
        assert!(f.include.is_empty());
    }

    #[test]
    fn parse_mixed_returns_none() {
        assert!(parse_debug_filter(Some("api,!file")).is_none());
    }

    #[test]
    fn parse_lowercases_and_trims() {
        let f = parse_debug_filter(Some("API , Hooks")).unwrap();
        assert_eq!(f.include, vec!["api", "hooks"]);
    }

    #[test]
    fn extract_mcp_and_server_name() {
        let cats = extract_debug_categories("MCP server \"obs\": ping");
        assert!(cats.iter().any(|c| c == "mcp"));
        assert!(cats.iter().any(|c| c == "obs"));
    }

    #[test]
    fn extract_mcp_single_quoted_name() {
        let cats = extract_debug_categories("MCP server 'foo': tick");
        assert!(cats.contains(&"mcp".to_string()));
        assert!(cats.contains(&"foo".to_string()));
    }

    #[test]
    fn extract_leading_colon_prefix() {
        let cats = extract_debug_categories("API: request sent");
        assert_eq!(cats[0], "api");
    }

    #[test]
    fn extract_bracket_category() {
        let cats = extract_debug_categories("[hooks] fired");
        assert!(cats.contains(&"hooks".to_string()));
    }

    #[test]
    fn extract_ant_only_1p_event_combined() {
        let cats = extract_debug_categories("[ANT-ONLY] 1P event: tengu_timer");
        assert!(cats.contains(&"ant-only".to_string()));
        assert!(cats.contains(&"1p".to_string()));
    }

    #[test]
    fn extract_no_categories_for_plain_text() {
        let cats = extract_debug_categories("hello world");
        assert!(cats.is_empty());
    }

    #[test]
    fn should_show_passes_when_no_filter() {
        assert!(should_show_debug_message("anything", None));
    }

    #[test]
    fn inclusive_filter_matches() {
        let f = parse_debug_filter(Some("api"));
        assert!(should_show_debug_message("API: hi", f.as_ref()));
        assert!(!should_show_debug_message("[hooks] hi", f.as_ref()));
    }

    #[test]
    fn exclusive_filter_hides_named() {
        let f = parse_debug_filter(Some("!api"));
        assert!(!should_show_debug_message("API: hi", f.as_ref()));
        assert!(should_show_debug_message("[hooks] hi", f.as_ref()));
    }

    #[test]
    fn uncategorised_hidden_under_any_filter() {
        let inc = parse_debug_filter(Some("api"));
        assert!(!should_show_debug_message("plain", inc.as_ref()));
        let exc = parse_debug_filter(Some("!api"));
        assert!(!should_show_debug_message("plain", exc.as_ref()));
    }
}
