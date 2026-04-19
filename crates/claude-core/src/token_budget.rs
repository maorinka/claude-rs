//! Parse inline token-budget directives from user prompts.
//!
//! Port of TS `utils/tokenBudget.ts:1-74`.
//!
//! Recognises two grammars:
//! - **Shorthand** — `+500k`, `+2.5M`, `+1B`. Anchored to start-of-string
//!   or end-of-string (preceded by whitespace) to avoid false positives
//!   against natural language like "something +stuff".
//! - **Verbose** — `use 1M tokens`, `spend 500k tokens`. Matches anywhere.
//!
//! All variants are case-insensitive. The `k`/`m`/`b` suffix multiplies
//! the numeric prefix by 1e3 / 1e6 / 1e9.

use once_cell::sync::Lazy;
use regex::Regex;

/// `^\s*\+(<num>)\s*(k|m|b)\b`, case-insensitive.
///
/// TS anchor: `^\s*\+…` — accepts leading whitespace then a `+` sigil.
static SHORTHAND_START_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^\s*\+(\d+(?:\.\d+)?)\s*(k|m|b)\b").unwrap());

/// `\s\+(<num>)\s*(k|m|b)\s*[.!?]?\s*$`, case-insensitive.
///
/// TS comment: "Lookbehind (?<=\s) is avoided — it defeats YARR JIT in JSC.
/// Capture the whitespace instead; callers offset match.index by 1 where
/// position matters." Preserved verbatim — Rust's regex engine has the
/// same constraint (no lookbehind at all), so the capture-then-offset
/// trick is load-bearing here too.
static SHORTHAND_END_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\s\+(\d+(?:\.\d+)?)\s*(k|m|b)\s*[.!?]?\s*$").unwrap());

/// `\b(use|spend)\s+(<num>)\s*(k|m|b)\s*tokens?\b`, case-insensitive.
static VERBOSE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:use|spend)\s+(\d+(?:\.\d+)?)\s*(k|m|b)\s*tokens?\b").unwrap());

fn apply_multiplier(value: &str, suffix: &str) -> Option<u64> {
    let v: f64 = value.parse().ok()?;
    let m: f64 = match suffix.to_ascii_lowercase().as_str() {
        "k" => 1_000.0,
        "m" => 1_000_000.0,
        "b" => 1_000_000_000.0,
        _ => return None,
    };
    Some((v * m) as u64)
}

/// Parse the first budget directive in `text`, returning the resolved
/// token count. Preference order matches TS: shorthand-start →
/// shorthand-end → verbose. Returns `None` when no directive matches.
pub fn parse_token_budget(text: &str) -> Option<u64> {
    if let Some(c) = SHORTHAND_START_RE.captures(text) {
        return apply_multiplier(c.get(1)?.as_str(), c.get(2)?.as_str());
    }
    if let Some(c) = SHORTHAND_END_RE.captures(text) {
        return apply_multiplier(c.get(1)?.as_str(), c.get(2)?.as_str());
    }
    if let Some(c) = VERBOSE_RE.captures(text) {
        return apply_multiplier(c.get(1)?.as_str(), c.get(2)?.as_str());
    }
    None
}

/// Byte range of a matched budget directive within the input.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct BudgetSpan {
    pub start: usize,
    pub end: usize,
}

/// Locate every budget directive in `text`, returned in source order.
///
/// Callers use these spans to strip or highlight the directive without
/// re-parsing. Overlap suppression is intentional — a bare `+500k` input
/// matches both `SHORTHAND_START_RE` and `SHORTHAND_END_RE`, and TS
/// dedupes by checking whether the end-match's start byte is already
/// covered (tokenBudget.ts:50-52). Rust mirrors exactly.
pub fn find_token_budget_positions(text: &str) -> Vec<BudgetSpan> {
    let mut positions: Vec<BudgetSpan> = Vec::new();

    if let Some(m) = SHORTHAND_START_RE.find(text) {
        // TS trims leading whitespace off the reported span so the
        // highlight doesn't underline irrelevant leading spaces. Rust
        // recreates that by measuring the trimmed prefix length.
        let whole = m.as_str();
        let trimmed_prefix_len = whole.len() - whole.trim_start().len();
        positions.push(BudgetSpan {
            start: m.start() + trimmed_prefix_len,
            end: m.end(),
        });
    }

    if let Some(m) = SHORTHAND_END_RE.find(text) {
        // Regex captures the leading `\s`; the actual directive starts one
        // byte later. All whitespace matched here is ASCII (the regex
        // class `\s` never matches a multi-byte char mid-pattern because
        // the surrounding tokens constrain it to a single char), so +1
        // is safe as a byte offset.
        let end_start = m.start() + 1;
        let already_covered = positions
            .iter()
            .any(|p| end_start >= p.start && end_start < p.end);
        if !already_covered {
            positions.push(BudgetSpan {
                start: end_start,
                end: m.end(),
            });
        }
    }

    for m in VERBOSE_RE.find_iter(text) {
        positions.push(BudgetSpan {
            start: m.start(),
            end: m.end(),
        });
    }

    positions
}

/// Format an en-US integer with comma thousands separators. TS uses
/// `Intl.NumberFormat('en-US').format(n)`; Rust has no locale-aware
/// formatter in std, so we inline the en-US-specific logic. Only
/// non-negative integers are expected (token counts).
fn format_en_us(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

/// Post-turn continuation message when a turn hit the budget.
/// Byte-identical to TS's `getBudgetContinuationMessage`.
pub fn get_budget_continuation_message(pct: u32, turn_tokens: u64, budget: u64) -> String {
    format!(
        "Stopped at {}% of token target ({} / {}). Keep working \u{2014} do not summarize.",
        pct,
        format_en_us(turn_tokens),
        format_en_us(budget),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_start_parses() {
        assert_eq!(parse_token_budget("+500k write a poem"), Some(500_000));
        assert_eq!(parse_token_budget("+2.5M do a thing"), Some(2_500_000));
        assert_eq!(parse_token_budget("+1B"), Some(1_000_000_000));
    }

    #[test]
    fn shorthand_start_case_insensitive() {
        assert_eq!(parse_token_budget("+500K"), Some(500_000));
        assert_eq!(parse_token_budget("+2.5m"), Some(2_500_000));
    }

    #[test]
    fn shorthand_end_parses() {
        assert_eq!(
            parse_token_budget("write a poem +500k"),
            Some(500_000)
        );
        assert_eq!(parse_token_budget("take your time +1M."), Some(1_000_000));
        assert_eq!(parse_token_budget("go big +2B!"), Some(2_000_000_000));
    }

    #[test]
    fn verbose_parses_anywhere() {
        assert_eq!(
            parse_token_budget("please use 1M tokens on this"),
            Some(1_000_000)
        );
        assert_eq!(
            parse_token_budget("we should spend 500k tokens"),
            Some(500_000)
        );
        assert_eq!(
            parse_token_budget("Spend 2.5B tokens"),
            Some(2_500_000_000)
        );
    }

    #[test]
    fn token_singular_accepted() {
        // `tokens?` regex allows both forms.
        assert_eq!(parse_token_budget("use 1k token"), Some(1_000));
    }

    #[test]
    fn rejects_natural_language_false_positives() {
        // Natural `+500k` embedded in a sentence must NOT match — shorthand
        // patterns are anchored. TS comment calls this out explicitly.
        assert_eq!(parse_token_budget("a+500k thing"), None);
        assert_eq!(parse_token_budget("hello world"), None);
    }

    #[test]
    fn rejects_invalid_suffix() {
        assert_eq!(parse_token_budget("+500q"), None);
        assert_eq!(parse_token_budget("+500"), None);
    }

    #[test]
    fn find_positions_bare_shorthand_dedupes() {
        // "+500k" matches both start and end patterns. Expect exactly one span.
        let spans = find_token_budget_positions("+500k");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0], BudgetSpan { start: 0, end: 5 });
    }

    #[test]
    fn find_positions_strips_leading_whitespace() {
        let spans = find_token_budget_positions("   +500k");
        assert_eq!(spans.len(), 1);
        // Highlight span starts at the `+`, not inside the leading spaces.
        assert_eq!(spans[0].start, 3);
        assert_eq!(spans[0].end, 8);
    }

    #[test]
    fn find_positions_end_shorthand_offset_by_one() {
        // End pattern captures the preceding whitespace; the reported span
        // starts one byte later so the highlight lines up on the `+`.
        let spans = find_token_budget_positions("do it +500k");
        assert_eq!(spans.len(), 1);
        // "do it " = 6 bytes; `+` at byte 6. Span starts at 6, ends at len 11.
        assert_eq!(spans[0].start, 6);
        assert_eq!(spans[0].end, 11);
        assert_eq!(&"do it +500k"[spans[0].start..spans[0].end], "+500k");
    }

    #[test]
    fn find_positions_verbose_global_matches_all() {
        // Two verbose matches in one string.
        let text = "use 1M tokens and also spend 2k tokens please";
        let spans = find_token_budget_positions(text);
        let hits: Vec<&str> = spans
            .iter()
            .map(|s| &text[s.start..s.end])
            .collect();
        assert_eq!(hits, vec!["use 1M tokens", "spend 2k tokens"]);
    }

    #[test]
    fn find_positions_empty_for_no_match() {
        assert!(find_token_budget_positions("just a normal prompt").is_empty());
    }

    #[test]
    fn format_en_us_thousands() {
        assert_eq!(format_en_us(0), "0");
        assert_eq!(format_en_us(999), "999");
        assert_eq!(format_en_us(1_000), "1,000");
        assert_eq!(format_en_us(1_234_567), "1,234,567");
        assert_eq!(format_en_us(1_000_000_000), "1,000,000,000");
    }

    #[test]
    fn continuation_message_matches_ts() {
        // Em-dash U+2014, singular percent form, comma thousands.
        let msg = get_budget_continuation_message(80, 4_000_000, 5_000_000);
        assert_eq!(
            msg,
            "Stopped at 80% of token target (4,000,000 / 5,000,000). Keep working — do not summarize."
        );
    }
}
