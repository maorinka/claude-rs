//! `## Searching past context` prompt block.
//!
//! Port of TS `memdir/memdir.ts:375`
//! `buildSearchingPastContextSection`. Rendered once per
//! session into the system prompt when the `tengu_coral_fern`
//! GrowthBook flag is on. Tells the model how to search memory
//! files first, session transcripts as a last resort.
//!
//! # Tool-surface choice
//!
//! TS picks between the `Grep` tool invocation syntax and a raw
//! `grep` shell line based on whether "embedded search tools"
//! are compiled in or REPL mode is active. The Rust port accepts
//! that boolean via [`SearchingPastContextInputs::embedded`] so
//! the caller decides — the flag isn't surfaced to this crate
//! today.

use crate::tool_names::GREP_TOOL_NAME;

/// Inputs for [`build_searching_past_context_section`].
pub struct SearchingPastContextInputs<'a> {
    /// Absolute path of the auto-memory directory. Used for the
    /// per-session memory search invocation.
    pub auto_mem_dir: &'a str,
    /// Absolute path of the project directory. Used for the
    /// transcript-log fallback search.
    pub project_dir: &'a str,
    /// `true` when embedded `ugrep`/REPL-mode shell is the
    /// model's preferred search surface; `false` when the
    /// `Grep` tool is available.
    pub embedded: bool,
}

/// Build the `## Searching past context` lines. Returns a
/// `Vec<String>` so callers can `lines.extend(...)` it into a
/// larger block — matches the TS array-of-strings return type.
///
/// Caller applies the `tengu_coral_fern` feature-flag gate —
/// this function always produces the section since the flag
/// lives outside claude-core.
pub fn build_searching_past_context_section(
    inputs: &SearchingPastContextInputs<'_>,
) -> Vec<String> {
    let (mem_search, transcript_search) = if inputs.embedded {
        (
            format!(
                r#"grep -rn "<search term>" {dir} --include="*.md""#,
                dir = inputs.auto_mem_dir
            ),
            format!(
                r#"grep -rn "<search term>" {dir}/ --include="*.jsonl""#,
                dir = inputs.project_dir
            ),
        )
    } else {
        (
            format!(
                r#"{tool} with pattern="<search term>" path="{dir}" glob="*.md""#,
                tool = GREP_TOOL_NAME,
                dir = inputs.auto_mem_dir
            ),
            format!(
                r#"{tool} with pattern="<search term>" path="{dir}/" glob="*.jsonl""#,
                tool = GREP_TOOL_NAME,
                dir = inputs.project_dir
            ),
        )
    };

    vec![
        "## Searching past context".to_string(),
        String::new(),
        "When looking for past context:".to_string(),
        "1. Search topic files in your memory directory:".to_string(),
        "```".to_string(),
        mem_search,
        "```".to_string(),
        "2. Session transcript logs (last resort — large files, slow):".to_string(),
        "```".to_string(),
        transcript_search,
        "```".to_string(),
        "Use narrow search terms (error messages, file paths, function names) rather than broad keywords.".to_string(),
        String::new(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(lines: &[String]) -> String {
        lines.join("\n")
    }

    #[test]
    fn grep_tool_variant_references_tool_name() {
        let lines = build_searching_past_context_section(&SearchingPastContextInputs {
            auto_mem_dir: "/u/.claude/memory",
            project_dir: "/proj",
            embedded: false,
        });
        let s = joined(&lines);
        assert!(s.contains("## Searching past context"));
        assert!(
            s.contains(r#"Grep with pattern="<search term>" path="/u/.claude/memory" glob="*.md""#)
        );
        assert!(s.contains(r#"path="/proj/" glob="*.jsonl""#));
    }

    #[test]
    fn embedded_variant_uses_grep_shell_form() {
        let lines = build_searching_past_context_section(&SearchingPastContextInputs {
            auto_mem_dir: "/u/.claude/memory",
            project_dir: "/proj",
            embedded: true,
        });
        let s = joined(&lines);
        assert!(s.contains(r#"grep -rn "<search term>" /u/.claude/memory --include="*.md""#));
        assert!(s.contains(r#"grep -rn "<search term>" /proj/ --include="*.jsonl""#));
        // Embedded form drops the tool-name syntax.
        assert!(!s.contains("Grep with pattern"));
    }

    #[test]
    fn section_keeps_narrow_search_rule() {
        let lines = build_searching_past_context_section(&SearchingPastContextInputs {
            auto_mem_dir: "/mem",
            project_dir: "/proj",
            embedded: false,
        });
        let s = joined(&lines);
        assert!(s.contains("Use narrow search terms"));
        assert!(s.contains("error messages, file paths, function names"));
    }

    #[test]
    fn section_has_exactly_two_numbered_items() {
        let lines = build_searching_past_context_section(&SearchingPastContextInputs {
            auto_mem_dir: "/mem",
            project_dir: "/proj",
            embedded: false,
        });
        let step_1s = lines.iter().filter(|l| l.starts_with("1. ")).count();
        let step_2s = lines.iter().filter(|l| l.starts_with("2. ")).count();
        assert_eq!(step_1s, 1);
        assert_eq!(step_2s, 1);
    }
}
