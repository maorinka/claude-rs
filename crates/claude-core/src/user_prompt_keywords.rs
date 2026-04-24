//! Detect sentiment / continuation keywords in user prompts.
//!
//! Port of TS `utils/userPromptKeywords.ts:1-27`.
//!
//! Used by analytics (`tengu_input_prompt` event carries
//! `is_negative` / `is_keep_going` booleans). The classification is
//! lossy-on-purpose — these patterns exist to cluster prompts for
//! trend analysis, not to gate behaviour, so occasional false positives
//! are acceptable.

use once_cell::sync::Lazy;
use regex::Regex;

/// Frustration / profanity patterns. Compiled once; `(?i)` makes the
/// match case-insensitive so we avoid the TS `toLowerCase()` before-call.
///
/// Pattern matches TS verbatim. Rust's `regex` crate has the same `\b`
/// semantics as JS, so word-boundary behaviour carries over cleanly.
static NEGATIVE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(wtf|wth|ffs|omfg|shit(ty|tiest)?|dumbass|horrible|awful|piss(ed|ing)? off|piece of (shit|crap|junk)|what the (fuck|hell)|fucking? (broken|useless|terrible|awful|horrible)|fuck you|screw (this|you)|so frustrating|this sucks|damn it)\b",
    )
    .unwrap()
});

/// `\b(keep going|go on)\b`, case-insensitive.
static KEEP_GOING_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(keep going|go on)\b").unwrap());

/// Returns `true` when the input contains a profanity / frustration
/// marker. TS `matchesNegativeKeyword`.
pub fn matches_negative_keyword(input: &str) -> bool {
    NEGATIVE_PATTERN.is_match(input)
}

/// Returns `true` when the input reads as a "keep going" continuation.
/// TS `matchesKeepGoingKeyword`:
/// - Bare `"continue"` (trimmed, case-insensitive) matches — but ONLY
///   when it's the entire prompt, so a sentence like "please continue
///   the refactor" doesn't trigger.
/// - `keep going` / `go on` match anywhere in the input.
pub fn matches_keep_going_keyword(input: &str) -> bool {
    let trimmed = input.trim();
    // TS: `if (lowerInput === 'continue') return true`. Rust does the
    // same equality check with `eq_ignore_ascii_case` — input is ASCII
    // for this branch (the literal `continue` is ASCII and case-folding
    // is only defined for ASCII here).
    if trimmed.eq_ignore_ascii_case("continue") {
        return true;
    }
    KEEP_GOING_PATTERN.is_match(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_matches_common_profanity() {
        assert!(matches_negative_keyword("wtf is this"));
        assert!(matches_negative_keyword("This is fucking broken"));
        assert!(matches_negative_keyword("what the hell"));
        assert!(matches_negative_keyword("piece of shit"));
        assert!(matches_negative_keyword("so frustrating"));
        assert!(matches_negative_keyword("this sucks"));
        assert!(matches_negative_keyword("damn it"));
    }

    #[test]
    fn negative_case_insensitive() {
        assert!(matches_negative_keyword("WTF"));
        assert!(matches_negative_keyword("This Sucks"));
        assert!(matches_negative_keyword("Fuck You"));
    }

    #[test]
    fn negative_word_boundary() {
        // `\b` must prevent partial-word matches — a file named `wtfoo.ts`
        // shouldn't register as profanity.
        assert!(!matches_negative_keyword("wtfoo.ts"));
        assert!(!matches_negative_keyword("shittiestrategy"));
        // But a real word-boundary hit still matches.
        assert!(matches_negative_keyword("shittiest"));
    }

    #[test]
    fn negative_ignores_clean_input() {
        assert!(!matches_negative_keyword("please fix this bug"));
        assert!(!matches_negative_keyword(""));
        assert!(!matches_negative_keyword("hello world"));
    }

    #[test]
    fn keep_going_bare_continue() {
        assert!(matches_keep_going_keyword("continue"));
        assert!(matches_keep_going_keyword("  continue  "));
        assert!(matches_keep_going_keyword("Continue"));
        assert!(matches_keep_going_keyword("CONTINUE"));
    }

    #[test]
    fn keep_going_continue_in_sentence_does_not_match_via_equality() {
        // TS: `continue` only matches if it IS the whole prompt. A
        // sentence containing `continue` should NOT trigger via the
        // equality branch. The regex (`\b(keep going|go on)\b`) also
        // has no `continue` alternative, so full sentences don't match.
        assert!(!matches_keep_going_keyword("please continue the refactor"));
        assert!(!matches_keep_going_keyword("don't continue"));
    }

    #[test]
    fn keep_going_phrase_anywhere() {
        assert!(matches_keep_going_keyword("keep going"));
        assert!(matches_keep_going_keyword("just keep going!"));
        assert!(matches_keep_going_keyword("go on"));
        assert!(matches_keep_going_keyword("please go on with the task"));
    }

    #[test]
    fn keep_going_case_insensitive() {
        assert!(matches_keep_going_keyword("Keep Going"));
        assert!(matches_keep_going_keyword("Go On"));
    }

    #[test]
    fn keep_going_rejects_unrelated() {
        assert!(!matches_keep_going_keyword(""));
        assert!(!matches_keep_going_keyword("stop"));
        assert!(!matches_keep_going_keyword("halt execution"));
        // Partial word — `\b` must prevent matching mid-word.
        assert!(!matches_keep_going_keyword("ongoing"));
    }

    #[test]
    fn user_typo_kkep_going_does_not_match() {
        // Documenting behaviour: the `kkep going` typo (which the real
        // user in this port session kept sending) does NOT match the
        // `keep going` pattern, because the regex requires the exact
        // word-boundary-delimited phrase. Preserved from TS verbatim.
        assert!(!matches_keep_going_keyword("kkep going"));
        assert!(!matches_keep_going_keyword("kkep going please"));
    }
}
