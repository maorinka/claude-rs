//! Assistant daily-log system-prompt builder (KAIROS auto-memory).
//!
//! Port of TS `src/memdir/memdir.ts:327-370` (`buildAssistantDailyLogPrompt`).
//! When auto-memory is on, this string is spliced into the system prompt so
//! the model appends observations to the day's log file rather than writing
//! them as chat output. A separate nightly process distills those logs into
//! `MEMORY.md` and topic files.
//!
//! The `auto_mem_dir` argument is the resolved auto-memory directory path
//! (TS `getAutoMemPath()`). Wiring for that lookup lives outside this
//! module — callers supply the path.
//!
//! The TS version chains with `buildSearchingPastContextSection` at the
//! tail; that section is gated by the `tengu_coral_fern` feature flag, so
//! this builder takes an explicit `searching_past_context` input that the
//! caller constructs when the flag is on and passes as an empty slice when
//! off. Keeps the feature-flag lookup out of claude-core.

use super::entrypoint::ENTRYPOINT_NAME;
use super::prompt::WHAT_NOT_TO_SAVE_SECTION;

/// Inputs to [`build_assistant_daily_log_prompt`].
pub struct DailyLogPromptInputs<'a> {
    /// Absolute path of the auto-memory directory — what
    /// `getAutoMemPath()` returns in TS.
    pub auto_mem_dir: &'a str,
    /// When `true`, omit the `## MEMORY.md` index section — matches the
    /// TS `skipIndex = true` branch used by the team-mem variant that
    /// emits the index elsewhere.
    pub skip_index: bool,
    /// The `buildSearchingPastContextSection(...)` output (pre-built by
    /// the caller, gated on the `tengu_coral_fern` feature flag). Pass
    /// an empty slice when the flag is off.
    pub searching_past_context: &'a [String],
}

/// Build the full daily-log system-prompt section. Joins lines with `\n`
/// exactly as TS `lines.join('\n')` does.
pub fn build_assistant_daily_log_prompt(inputs: &DailyLogPromptInputs<'_>) -> String {
    let log_path_pattern = format!("{}/logs/YYYY/MM/YYYY-MM-DD.md", inputs.auto_mem_dir);

    let mut lines: Vec<String> = Vec::new();
    lines.push("# auto memory".to_string());
    lines.push(String::new());
    lines.push(format!(
        "You have a persistent, file-based memory system found at: `{}`",
        inputs.auto_mem_dir
    ));
    lines.push(String::new());
    lines.push(
        "This session is long-lived. As you work, record anything worth remembering by **appending** to today's daily log file:".to_string(),
    );
    lines.push(String::new());
    lines.push(format!("`{log_path_pattern}`"));
    lines.push(String::new());
    lines.push(
        "Substitute today's date (from `currentDate` in your context) for `YYYY-MM-DD`. When the date rolls over mid-session, start appending to the new day's file.".to_string(),
    );
    lines.push(String::new());
    lines.push(
        "Write each entry as a short timestamped bullet. Create the file (and parent directories) on first write if it does not exist. Do not rewrite or reorganize the log — it is append-only. A separate nightly process distills these logs into `MEMORY.md` and topic files.".to_string(),
    );
    lines.push(String::new());
    lines.push("## What to log".to_string());
    lines.push(
        r#"- User corrections and preferences ("use bun, not npm"; "stop summarizing diffs")"#
            .to_string(),
    );
    lines.push("- Facts about the user, their role, or their goals".to_string());
    lines.push(
        "- Project context that is not derivable from the code (deadlines, incidents, decisions and their rationale)".to_string(),
    );
    lines.push(
        "- Pointers to external systems (dashboards, Linear projects, Slack channels)".to_string(),
    );
    lines.push("- Anything the user explicitly asks you to remember".to_string());
    lines.push(String::new());
    for s in WHAT_NOT_TO_SAVE_SECTION {
        lines.push((*s).to_string());
    }
    lines.push(String::new());

    if !inputs.skip_index {
        lines.push(format!("## {ENTRYPOINT_NAME}"));
        lines.push(format!(
            "`{ENTRYPOINT_NAME}` is the distilled index (maintained nightly from your logs) and is loaded into your context automatically. Read it for orientation, but do not edit it directly — record new information in today's log instead."
        ));
        lines.push(String::new());
    }

    for s in inputs.searching_past_context {
        lines.push(s.clone());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> DailyLogPromptInputs<'static> {
        DailyLogPromptInputs {
            auto_mem_dir: "/home/u/.claude/memory",
            skip_index: false,
            searching_past_context: &[],
        }
    }

    #[test]
    fn emits_auto_memory_header_and_log_path_pattern() {
        let p = build_assistant_daily_log_prompt(&base());
        assert!(p.starts_with("# auto memory\n"));
        assert!(
            p.contains("`/home/u/.claude/memory/logs/YYYY/MM/YYYY-MM-DD.md`"),
            "log path pattern missing from prompt: {p}"
        );
    }

    #[test]
    fn includes_what_to_log_and_what_not_to_save() {
        let p = build_assistant_daily_log_prompt(&base());
        assert!(p.contains("## What to log"));
        // WHAT_NOT_TO_SAVE_SECTION first bullet starts "- Code patterns…"
        assert!(p.contains("## What NOT to save in memory"));
    }

    #[test]
    fn default_includes_index_section() {
        let p = build_assistant_daily_log_prompt(&base());
        assert!(p.contains(&format!("## {ENTRYPOINT_NAME}")));
        assert!(p.contains("distilled index"));
    }

    #[test]
    fn skip_index_omits_memory_md_section() {
        let mut i = base();
        i.skip_index = true;
        let p = build_assistant_daily_log_prompt(&i);
        assert!(!p.contains(&format!("## {ENTRYPOINT_NAME}\n")));
    }

    #[test]
    fn appends_searching_past_context_lines_verbatim() {
        let extra = vec![
            "## Searching past context".to_string(),
            String::new(),
            "test line".to_string(),
        ];
        let mut i = base();
        i.searching_past_context = &extra;
        let p = build_assistant_daily_log_prompt(&i);
        assert!(p.contains("## Searching past context"));
        assert!(p.contains("\ntest line"));
    }
}
