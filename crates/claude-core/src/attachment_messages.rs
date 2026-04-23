//! Context-injected attachment messages.
//!
//! Port of TS `utils/messages.ts:3603-4188`. These are the
//! user-role messages the TS REPL injects at specific moments
//! (PDF opened, IDE file selected, plan mode entered, task
//! notification, budget/date/ultrathink/deferred-tool
//! transitions, etc.) so the model sees them mid-turn.
//!
//! The Rust port doesn't have the attachment-injection runtime
//! yet — this module parks every text constant + builder so when
//! the context injector lands, callers reach for byte-stable
//! text that matches TS verbatim (cache parity).
//!
//! # TS→Rust notes
//!
//! TS tool-name `${...}` interpolations are inlined as literals
//! matching [`crate::tool_names`] (Agent, AskUserQuestion, Read,
//! SendMessage, TaskCreate, TaskUpdate, ExitPlanModeV2 =
//! `ExitPlanMode`) so the prompts stay byte-stable against the
//! tool-name constants.
//!
//! # Not covered here
//!
//! - Plan mode's 5-phase full workflow — large enough to warrant
//!   its own module ([`crate::plan_mode_workflow`]).
//! - Cache-breaker / auto-mode / ultrathink runtime — the text
//!   lives here but the caller-side injection is still deferred.

use crate::tool_names::AGENT_TOOL_NAME;

/// `PDF file: …` attachment text. Built only when a PDF
/// attachment appears in the user's message. Port of TS
/// `utils/messages.ts:3603-3608`.
pub fn pdf_reference_attachment(
    filename: &str,
    page_count: u32,
    file_size_human: &str,
) -> String {
    format!(
        "PDF file: {filename} ({page_count} pages, {file_size_human}). \
         This PDF is too large to read all at once. You MUST use the Read tool with the pages parameter \
         to read specific page ranges (e.g., pages: \"1-5\"). Do NOT call Read without the pages parameter \
         or it will fail. Start by reading the first few pages to understand the structure, then read more as needed. \
         Maximum 20 pages per request."
    )
}

/// IDE "selected lines" attachment text. Port of TS
/// `utils/messages.ts:3623`.
pub fn ide_selected_lines_attachment(
    line_start: u32,
    line_end: u32,
    filename: &str,
    content: &str,
) -> String {
    format!(
        "The user selected the lines {line_start} to {line_end} from {filename}:\n{content}\n\nThis may or may not be related to the current task."
    )
}

/// IDE "opened file" attachment text. Port of TS
/// `utils/messages.ts:3631`.
pub fn ide_opened_file_attachment(filename: &str) -> String {
    format!(
        "The user opened the file {filename} in the IDE. This may or may not be related to the current task."
    )
}

/// Plan-file reference attachment — injected when a prior
/// session's plan file exists. Port of TS
/// `utils/messages.ts:3639`.
pub fn plan_file_reference_attachment(
    plan_file_path: &str,
    plan_content: &str,
) -> String {
    format!(
        "A plan file exists from plan mode at: {plan_file_path}\n\nPlan contents:\n\n{plan_content}\n\nIf this plan is relevant to the current work and not already complete, continue working on it."
    )
}

/// Invoked-skills attachment. Injected so the model keeps
/// following previously-activated skill guidelines. Port of TS
/// `utils/messages.ts:3658`.
pub fn invoked_skills_attachment(skills_content: &str) -> String {
    format!(
        "The following skills were invoked in this session. Continue to follow these guidelines:\n\n{skills_content}"
    )
}

/// TodoWrite periodic reminder + optional existing-contents
/// tail. Port of TS `utils/messages.ts:3668`. Pass an empty
/// `todo_items` string to skip the "existing contents" tail.
pub fn todo_reminder_attachment(todo_items: &str) -> String {
    let base = "The TodoWrite tool hasn't been used recently. If you're working on tasks that would benefit from tracking progress, consider using the TodoWrite tool to track progress. Also consider cleaning up the todo list if has become stale and no longer matches what you are working on. Only use it if it's relevant to the current work. This is just a gentle reminder - ignore if not applicable. Make sure that you NEVER mention this reminder to the user\n";
    if todo_items.is_empty() {
        base.to_string()
    } else {
        format!("{base}\n\nHere are the existing contents of your todo list:\n\n[{todo_items}]")
    }
}

/// Task tool reminder (TaskCreate/TaskUpdate analogue of
/// [`todo_reminder_attachment`]). Port of TS
/// `utils/messages.ts:3688`. Tool names baked in as `TaskCreate`
/// / `TaskUpdate` matching the registered tool names.
pub const TASK_REMINDER_ATTACHMENT: &str = "The task tools haven't been used recently. If you're working on tasks that would benefit from tracking progress, consider using TaskCreate to add new tasks and TaskUpdate to update task status (set to in_progress when starting, completed when done). Also consider cleaning up the task list if it has become stale. Only use these if relevant to the current work. This is just a gentle reminder - ignore if not applicable. Make sure that you NEVER mention this reminder to the user\n";

/// Output-style active reminder. Port of TS
/// `utils/messages.ts:3807`.
pub fn output_style_reminder_attachment(style_name: &str) -> String {
    format!("{style_name} output style is active. Remember to follow the specific guidelines for this style.")
}

/// `<new-diagnostics>` attachment emitted by IDE integration.
/// Port of TS `utils/messages.ts:3821`.
pub fn diagnostics_attachment(diagnostic_summary: &str) -> String {
    format!(
        "<new-diagnostics>The following new diagnostic issues were detected:\n\n{diagnostic_summary}</new-diagnostics>"
    )
}

/// Plan-mode re-entry attachment. Shown when the user re-enters
/// plan mode mid-session with an existing plan file. Port of TS
/// `utils/messages.ts:3830-3842`. Tool-name interpolation uses
/// `ExitPlanMode` (the registered tool name).
pub fn plan_mode_reentry_attachment(plan_file_path: &str) -> String {
    format!(
        "## Re-entering Plan Mode\n\n\
         You are returning to plan mode after having previously exited it. A plan file exists at {plan_file_path} from your previous planning session.\n\n\
         **Before proceeding with any new planning, you should:**\n\
         1. Read the existing plan file to understand what was previously planned\n\
         2. Evaluate the user's current request against that plan\n\
         3. Decide how to proceed:\n   \
         - **Different task**: If the user's request is for a different task—even if it's similar or related—start fresh by overwriting the existing plan\n   \
         - **Same task, continuing**: If this is explicitly a continuation or refinement of the exact same task, modify the existing plan while cleaning up outdated or irrelevant sections\n\
         4. Continue on with the plan process and most importantly you should always edit the plan file one way or the other before calling ExitPlanMode\n\n\
         Treat this as a fresh planning session. Do not assume the existing plan is relevant without evaluating it first."
    )
}

/// Auto-mode exit attachment. Port of TS
/// `utils/messages.ts:3864-3866`.
pub const AUTO_MODE_EXIT_ATTACHMENT: &str = "## Exited Auto Mode

You have exited auto mode. The user may now want to interact more directly. You should ask clarifying questions when the approach is ambiguous rather than making assumptions.";

/// MCP text-resource caveat appended after a fetched MCP
/// resource. Port of TS `utils/messages.ts:3899-3908`.
pub const MCP_RESOURCE_RE_READ_WARNING: &str =
    "Do NOT read this resource again unless you think it may have changed, since you already have the full contents.";

/// Agent-mention attachment. Injected when the user `@mentions`
/// an agent by name. Port of TS `utils/messages.ts:3949`.
pub fn agent_mention_attachment(agent_type: &str) -> String {
    format!(
        "The user has expressed a desire to invoke the agent \"{agent_type}\". Please invoke the agent appropriately, passing in the required context to it. "
    )
}

/// Task-status attachment variants. Port of TS
/// `utils/messages.ts:3960-4017`.
pub fn task_stopped_attachment(description: &str, task_id: &str) -> String {
    format!("Task \"{description}\" ({task_id}) was stopped by the user.")
}

/// Body of the running-task attachment, before progress/tail.
/// Port of TS first `agent-still-running` line.
pub fn task_running_prefix(description: &str, task_id: &str) -> String {
    format!("Background agent \"{description}\" ({task_id}) is still running.")
}

/// Trailing guidance appended to the running-task attachment.
/// Port of TS `utils/messages.ts:3988` (with `SendMessage` tool
/// name baked in). `output_file_path` is the path the model can
/// Read for partial output.
pub fn task_running_tail(output_file_path: &str) -> String {
    format!(
        "Do NOT spawn a duplicate. You will be notified when it completes. You can read partial output at {output_file_path} or send it a message with SendMessage."
    )
}

/// Header line for a completed-task attachment. Port of TS
/// `utils/messages.ts:4004`.
pub fn task_completed_header(
    task_id: &str,
    task_type: &str,
    display_status: &str,
    description: &str,
) -> String {
    format!(
        "Task {task_id} (type: {task_type}) (status: {display_status}) (description: {description})"
    )
}

/// Trailing "read the output file" line of the completed-task
/// attachment. Port of TS `utils/messages.ts:4017`.
pub fn task_completed_tail(output_file_path: &str) -> String {
    format!("Read the output file to retrieve the result: {output_file_path}")
}

/// Token usage budget attachment. Port of TS
/// `utils/messages.ts:4059`.
pub fn token_budget_attachment(used: &str, total: &str, remaining: &str) -> String {
    format!("Token usage: {used}/{total}; {remaining} remaining")
}

/// USD budget attachment. Port of TS `utils/messages.ts:4067`.
pub fn usd_budget_attachment(used: &str, total: &str, remaining: &str) -> String {
    format!("USD budget: ${used}/${total}; ${remaining} remaining")
}

/// Output-tokens per-turn + per-session attachment. Port of TS
/// `utils/messages.ts:4075`. `turn_text` is the pre-formatted
/// per-turn tokens block ("123 tokens (45% of prior turn)" in
/// TS); caller owns the formatting to stay platform-agnostic.
pub fn output_tokens_attachment(turn_text: &str, session: u64) -> String {
    format!("Output tokens — turn: {turn_text} · session: {session}")
}

/// Compaction reminder attachment. Port of TS
/// `utils/messages.ts:4142`.
pub const COMPACTION_REMINDER_ATTACHMENT: &str =
    "Auto-compact is enabled. When the context window is nearly full, older messages will be automatically summarized so you can continue working seamlessly. There is no need to stop or rush — you have unlimited context through automatic compaction.";

/// Date-change attachment, emitted once when the system date
/// rolls over mid-session. Port of TS `utils/messages.ts:4165`.
pub fn date_change_attachment(new_date: &str) -> String {
    format!(
        "The date has changed. Today's date is now {new_date}. DO NOT mention this to the user explicitly because they are already aware."
    )
}

/// Ultrathink reasoning-effort attachment. Port of TS
/// `utils/messages.ts:4173`. `level` comes from the user
/// selector (e.g. `"high"`, `"ultra"`).
pub fn ultrathink_effort_attachment(level: &str) -> String {
    format!("The user has requested reasoning effort level: {level}. Apply this to the current turn.")
}

/// Deferred-tools added-delta attachment. Port of TS
/// `utils/messages.ts:4180`. `added_lines` is pre-joined with
/// `\n` by the caller.
pub fn deferred_tools_added_attachment(added_lines: &str) -> String {
    format!(
        "The following deferred tools are now available via ToolSearch:\n{added_lines}"
    )
}

/// Deferred-tools removed-delta attachment. Port of TS
/// `utils/messages.ts:4188`. `removed_names` is pre-joined with
/// `\n` by the caller.
pub fn deferred_tools_removed_attachment(removed_names: &str) -> String {
    format!(
        "The following deferred tools are no longer available (their MCP server disconnected). Do not search for them — ToolSearch will return no match:\n{removed_names}"
    )
}

/// Auto mode full system instructions. Port of TS
/// `utils/messages.ts:3428-3438`.
pub const AUTO_MODE_FULL_INSTRUCTIONS: &str = "## Auto Mode Active

Auto mode is active. The user chose continuous, autonomous execution. You should:

1. **Execute immediately** — Start implementing right away. Make reasonable assumptions and proceed on low-risk work.
2. **Minimize interruptions** — Prefer making reasonable assumptions over asking questions for routine decisions.
3. **Prefer action over planning** — Do not enter plan mode unless the user explicitly asks. When in doubt, start coding.
4. **Expect course corrections** — The user may provide suggestions or course corrections at any point; treat those as normal input.
5. **Do not take overly destructive actions** — Auto mode is not a license to destroy. Anything that deletes data or modifies shared or production systems still needs explicit user confirmation. If you reach such a decision point, ask and wait, or course correct to a safer method instead.
6. **Avoid data exfiltration** — Post even routine messages to chat platforms or work tickets only if the user has directed you to. You must not share secrets (e.g. credentials, internal documentation) unless the user has explicitly authorized both that specific secret and its destination.";

/// Auto mode sparse reminder (injected on subsequent turns after
/// the full instructions landed once). Port of TS
/// `utils/messages.ts:3446`.
pub const AUTO_MODE_SPARSE_REMINDER: &str = "Auto mode still active (see full instructions earlier in conversation). Execute autonomously, minimize interruptions, prefer action over planning.";

/// Budget continuation message — emitted by the token-budget
/// continuation system when a turn stops under target. Port of
/// TS `utils/tokenBudget.ts:72`. `pct`, `turn_tokens`, `budget`
/// are caller-formatted (e.g. `"47%"`, `"12,345"`).
pub fn budget_continuation_message(pct: &str, turn_tokens: &str, budget: &str) -> String {
    format!(
        "Stopped at {pct} of token target ({turn_tokens} / {budget}). Keep working — do not summarize."
    )
}

/// Sanity assert that the `AGENT_TOOL_NAME` literal embedded in
/// agent-mention tests still matches the registered name.
#[doc(hidden)]
pub fn _agent_tool_name_sanity() -> &'static str {
    AGENT_TOOL_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_attachment_has_key_rules() {
        let a = pdf_reference_attachment("paper.pdf", 42, "3.2 MB");
        assert!(a.starts_with("PDF file: paper.pdf (42 pages, 3.2 MB)"));
        assert!(a.contains("pages parameter"));
        assert!(a.contains("pages: \"1-5\""));
        assert!(a.contains("Maximum 20 pages per request"));
    }

    #[test]
    fn ide_attachments_build_expected_text() {
        let sel = ide_selected_lines_attachment(10, 20, "src/x.rs", "body\n");
        assert!(sel.contains("the lines 10 to 20 from src/x.rs"));
        assert!(sel.ends_with("This may or may not be related to the current task."));
        let open = ide_opened_file_attachment("README.md");
        assert_eq!(
            open,
            "The user opened the file README.md in the IDE. This may or may not be related to the current task."
        );
    }

    #[test]
    fn plan_file_reference_wraps_contents() {
        let a = plan_file_reference_attachment("/plan.md", "step 1\nstep 2");
        assert!(a.contains("A plan file exists from plan mode at: /plan.md"));
        assert!(a.contains("Plan contents:\n\nstep 1\nstep 2"));
        assert!(a.ends_with("continue working on it."));
    }

    #[test]
    fn todo_reminder_omits_items_when_empty() {
        let r = todo_reminder_attachment("");
        assert!(r.contains("TodoWrite tool hasn't been used recently"));
        assert!(!r.contains("existing contents of your todo list"));
    }

    #[test]
    fn todo_reminder_appends_items_when_present() {
        let r = todo_reminder_attachment("\"task A\", \"task B\"");
        assert!(r.contains(
            "Here are the existing contents of your todo list:\n\n[\"task A\", \"task B\"]"
        ));
    }

    #[test]
    fn task_reminder_mentions_tool_names_verbatim() {
        // The constant bakes TaskCreate + TaskUpdate as literals
        // (not `${...}`) so it ships in the binary unchanged.
        assert!(TASK_REMINDER_ATTACHMENT.contains("TaskCreate"));
        assert!(TASK_REMINDER_ATTACHMENT.contains("TaskUpdate"));
        assert!(TASK_REMINDER_ATTACHMENT.contains("gentle reminder"));
    }

    #[test]
    fn output_style_reminder_interpolates_name() {
        assert_eq!(
            output_style_reminder_attachment("Learning"),
            "Learning output style is active. Remember to follow the specific guidelines for this style."
        );
    }

    #[test]
    fn diagnostics_attachment_wraps_tag() {
        let d = diagnostics_attachment("main.ts:10 error: foo");
        assert!(d.starts_with("<new-diagnostics>"));
        assert!(d.ends_with("</new-diagnostics>"));
    }

    #[test]
    fn plan_mode_reentry_mentions_tool_name_and_path() {
        let p = plan_mode_reentry_attachment("/p/plan.md");
        assert!(p.starts_with("## Re-entering Plan Mode"));
        assert!(p.contains("/p/plan.md"));
        assert!(p.contains("ExitPlanMode"));
    }

    #[test]
    fn auto_mode_full_and_sparse_are_distinct() {
        assert!(AUTO_MODE_FULL_INSTRUCTIONS.contains("## Auto Mode Active"));
        assert!(AUTO_MODE_FULL_INSTRUCTIONS.contains("overly destructive"));
        assert!(AUTO_MODE_SPARSE_REMINDER.contains("Auto mode still active"));
        assert!(
            AUTO_MODE_SPARSE_REMINDER.len() < AUTO_MODE_FULL_INSTRUCTIONS.len(),
            "sparse reminder must be shorter than full instructions"
        );
    }

    #[test]
    fn auto_mode_exit_requires_clarifying_questions() {
        assert!(AUTO_MODE_EXIT_ATTACHMENT.contains("Exited Auto Mode"));
        assert!(AUTO_MODE_EXIT_ATTACHMENT.contains("clarifying questions"));
    }

    #[test]
    fn task_running_tail_bakes_send_message_tool_name() {
        let tail = task_running_tail("/tmp/out.log");
        assert!(tail.contains("SendMessage"));
        assert!(tail.contains("/tmp/out.log"));
        assert!(tail.contains("Do NOT spawn a duplicate"));
    }

    #[test]
    fn task_status_builders_format_expected_text() {
        assert_eq!(
            task_stopped_attachment("refactor", "t1"),
            "Task \"refactor\" (t1) was stopped by the user."
        );
        assert_eq!(
            task_running_prefix("refactor", "t1"),
            "Background agent \"refactor\" (t1) is still running."
        );
        assert_eq!(
            task_completed_header("t1", "agent", "completed", "refactor"),
            "Task t1 (type: agent) (status: completed) (description: refactor)"
        );
    }

    #[test]
    fn token_and_usd_budget_formatting() {
        assert_eq!(
            token_budget_attachment("10k", "100k", "90k"),
            "Token usage: 10k/100k; 90k remaining"
        );
        assert_eq!(
            usd_budget_attachment("1.00", "10.00", "9.00"),
            "USD budget: $1.00/$10.00; $9.00 remaining"
        );
    }

    #[test]
    fn date_change_includes_dont_mention_clause() {
        let d = date_change_attachment("2026-05-01");
        assert!(d.contains("Today's date is now 2026-05-01"));
        assert!(d.contains("DO NOT mention this to the user"));
    }

    #[test]
    fn deferred_tools_deltas_name_toolsearch() {
        let added = deferred_tools_added_attachment("- FooTool\n- BarTool");
        assert!(added.contains("available via ToolSearch"));
        assert!(added.contains("- FooTool"));
        let removed = deferred_tools_removed_attachment("- FooTool");
        assert!(removed.contains("ToolSearch will return no match"));
    }

    #[test]
    fn mcp_resource_warning_is_shared_constant() {
        assert!(MCP_RESOURCE_RE_READ_WARNING.contains("Do NOT read this resource again"));
    }

    #[test]
    fn agent_mention_quotes_type() {
        let m = agent_mention_attachment("code-reviewer");
        assert!(m.contains("\"code-reviewer\""));
        assert!(m.ends_with("passing in the required context to it. "));
    }

    #[test]
    fn budget_continuation_format_matches_ts() {
        let m = budget_continuation_message("47%", "12,345", "26,000");
        assert_eq!(
            m,
            "Stopped at 47% of token target (12,345 / 26,000). Keep working — do not summarize."
        );
    }

    #[test]
    fn ultrathink_effort_echoes_level() {
        assert_eq!(
            ultrathink_effort_attachment("high"),
            "The user has requested reasoning effort level: high. Apply this to the current turn."
        );
    }

    #[test]
    fn compaction_reminder_explains_autocompact() {
        assert!(COMPACTION_REMINDER_ATTACHMENT.contains("Auto-compact"));
        assert!(COMPACTION_REMINDER_ATTACHMENT.contains("unlimited context"));
    }

    #[test]
    fn agent_tool_name_sanity_matches_registered() {
        // If the Agent tool ever renames, attachment text that
        // hardcodes `Agent` drifts. The `_agent_tool_name_sanity`
        // hook just surfaces the constant so a regression is
        // visible in this test.
        assert_eq!(_agent_tool_name_sanity(), "Agent");
    }
}
