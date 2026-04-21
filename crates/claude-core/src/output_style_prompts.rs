//! Built-in output-style prompts (Explanatory + Learning).
//!
//! Port of TS `constants/outputStyles.ts:30-133` — the two
//! built-in named output styles that ship with Claude Code
//! alongside the default style.
//!
//! `OutputStyle` itself (user-loadable styles) is already ported
//! in `output_styles.rs` as a loader for
//! `~/.claude/output-styles/*.md`. This module complements that
//! with the hardcoded `Explanatory` + `Learning` styles and
//! their shared `EXPLANATORY_FEATURE_PROMPT` suffix.
//!
//! TS uses `${figures.star}` / `${figures.bullet}` symbols from
//! the `figures` package; the Rust port inlines the Unicode
//! equivalents directly (`★` and `•`) — no runtime substitution.

/// Shared "Insights" footer appended to both Explanatory and
/// Learning styles. Port of TS
/// `constants/outputStyles.ts:30-37`.
pub const EXPLANATORY_FEATURE_PROMPT: &str =
    include_str!("prompts/output_style_explanatory_feature.md");

/// Built-in `Explanatory` style prelude (without the
/// `EXPLANATORY_FEATURE_PROMPT` suffix). Concatenate with
/// `EXPLANATORY_FEATURE_PROMPT` via
/// [`explanatory_style_prompt`] to get the full TS-equivalent
/// string. Port of TS `constants/outputStyles.ts:43-54`.
pub const EXPLANATORY_STYLE_PROMPT_PREFIX: &str =
    include_str!("prompts/output_style_explanatory.md");

/// Built-in `Learning` style prelude (without the shared
/// `EXPLANATORY_FEATURE_PROMPT` suffix). Concatenate with
/// `EXPLANATORY_FEATURE_PROMPT` via [`learning_style_prompt`]
/// to get the full TS-equivalent string. Port of TS
/// `constants/outputStyles.ts:56-133`.
pub const LEARNING_STYLE_PROMPT_PREFIX: &str =
    include_str!("prompts/output_style_learning.md");

/// Build the full Explanatory style prompt by concatenating
/// the prefix with the shared insights footer.
pub fn explanatory_style_prompt() -> String {
    format!("{EXPLANATORY_STYLE_PROMPT_PREFIX}{EXPLANATORY_FEATURE_PROMPT}")
}

/// Build the full Learning style prompt.
pub fn learning_style_prompt() -> String {
    format!("{LEARNING_STYLE_PROMPT_PREFIX}\n## Insights\n{EXPLANATORY_FEATURE_PROMPT}")
}

/// Built-in style metadata, matching the TS `OUTPUT_STYLE_CONFIG`
/// shape for `Explanatory` and `Learning`. `default` is omitted
/// (null in TS).
#[derive(Debug, Clone, Copy)]
pub struct BuiltinStyle {
    pub name: &'static str,
    pub description: &'static str,
    pub keep_coding_instructions: bool,
}

/// `Explanatory` style metadata. TS `outputStyles.ts:44-49`.
pub const EXPLANATORY_STYLE: BuiltinStyle = BuiltinStyle {
    name: "Explanatory",
    description: "Claude explains its implementation choices and codebase patterns",
    keep_coding_instructions: true,
};

/// `Learning` style metadata. TS `outputStyles.ts:57-62`.
pub const LEARNING_STYLE: BuiltinStyle = BuiltinStyle {
    name: "Learning",
    description: "Claude pauses and asks you to write small pieces of code for hands-on practice",
    keep_coding_instructions: true,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explanatory_prompt_contains_style_marker() {
        let p = explanatory_style_prompt();
        assert!(p.contains("# Explanatory Style Active"));
        assert!(p.contains("## Insights"));
    }

    #[test]
    fn learning_prompt_contains_learn_by_doing() {
        let p = learning_style_prompt();
        assert!(p.contains("# Learning Style Active"));
        assert!(p.contains("**Learn by Doing**"));
        assert!(p.contains("TODO(human)"));
    }

    #[test]
    fn both_styles_include_shared_insights_block() {
        let e = explanatory_style_prompt();
        let l = learning_style_prompt();
        // Both end with the shared insights footer.
        assert!(e.contains("In order to encourage learning, before and after writing code"));
        assert!(l.contains("In order to encourage learning, before and after writing code"));
    }

    #[test]
    fn metadata_matches_ts() {
        assert_eq!(EXPLANATORY_STYLE.name, "Explanatory");
        assert!(EXPLANATORY_STYLE.keep_coding_instructions);
        assert_eq!(LEARNING_STYLE.name, "Learning");
        assert!(LEARNING_STYLE.keep_coding_instructions);
    }
}
