//! Background-task notification XML messages + tag constants.
//!
//! Port of TS `constants/xml.ts` (tag names) +
//! `tasks/LocalAgentTask/LocalAgentTask.tsx:252` +
//! `tasks/LocalMainSessionTask.ts:255` +
//! `tasks/LocalShellTask/LocalShellTask.tsx:75-136`. These are
//! the XML blobs TS emits as user-role messages when a
//! background agent/session/shell task changes state; the main
//! loop sees them, the model reacts. The Rust port's task
//! runtime doesn't dispatch these yet — this module parks the
//! tag constants + summary/builder helpers so when background
//! tasks ship the prompt cache stays byte-stable with TS.
//!
//! # What's here
//! - XML tag name constants matching TS `constants/xml.ts:27-38`
//! - [`BACKGROUND_BASH_SUMMARY_PREFIX`] from
//!   `LocalShellTask.tsx:23`
//! - [`AgentTaskStatus`] / [`ShellTaskKind`] enums
//! - Summary builders for agent, session, shell-monitor,
//!   shell-bash, and stalled-shell notifications
//! - [`build_task_notification_xml`] — assembles the XML block

/// `<task-notification>` wrapper tag. Port of TS
/// `constants/xml.ts:28` `TASK_NOTIFICATION_TAG`.
pub const TASK_NOTIFICATION_TAG: &str = "task-notification";

/// `<task-id>` child tag.
pub const TASK_ID_TAG: &str = "task-id";

/// `<tool-use-id>` child tag (emitted only when the task traces
/// back to a live tool call).
pub const TOOL_USE_ID_TAG: &str = "tool-use-id";

/// `<task-type>` child tag.
pub const TASK_TYPE_TAG: &str = "task-type";

/// `<output-file>` child tag.
pub const OUTPUT_FILE_TAG: &str = "output-file";

/// `<status>` child tag.
pub const STATUS_TAG: &str = "status";

/// `<summary>` child tag.
pub const SUMMARY_TAG: &str = "summary";

/// `<reason>` child tag (used in some stop/fail paths).
pub const REASON_TAG: &str = "reason";

/// `<worktree>` wrapper. Emitted only when the agent ran with
/// `isolation: "worktree"` and produced commits.
pub const WORKTREE_TAG: &str = "worktree";

/// `<worktreePath>` child tag inside `<worktree>`.
pub const WORKTREE_PATH_TAG: &str = "worktreePath";

/// `<worktreeBranch>` child tag inside `<worktree>`.
pub const WORKTREE_BRANCH_TAG: &str = "worktreeBranch";

/// Prefix used by bash-kind shell-task summaries so the main
/// loop can tell "Background command X" notifications apart
/// from other status lines. Port of TS
/// `tasks/LocalShellTask/LocalShellTask.tsx:23`.
pub const BACKGROUND_BASH_SUMMARY_PREFIX: &str = "Background command ";

/// Terminal status of a background agent task. Port of the three
/// TS branches at LocalAgentTask.tsx:252.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTaskStatus {
    Completed,
    Failed,
    Stopped,
}

impl AgentTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// Shell task kind — `monitor` for streaming scripts,
/// `bash` for one-shot commands. Port of the TS branch split at
/// LocalShellTask.tsx:136.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellTaskKind {
    Monitor,
    Bash,
}

/// Build the `Agent "<description>" <verb>` summary line. Port
/// of TS `LocalAgentTask.tsx:252-255`.
pub fn agent_task_summary(
    status: AgentTaskStatus,
    description: &str,
    error: Option<&str>,
) -> String {
    match status {
        AgentTaskStatus::Completed => format!("Agent \"{description}\" completed"),
        AgentTaskStatus::Failed => {
            let err = error.unwrap_or("Unknown error");
            format!("Agent \"{description}\" failed: {err}")
        }
        AgentTaskStatus::Stopped => format!("Agent \"{description}\" was stopped"),
    }
}

/// Build the `Background session "<description>" <verb>` line.
/// Port of TS `LocalMainSessionTask.ts:255-259`.
pub fn background_session_summary(completed: bool, description: &str) -> String {
    if completed {
        format!("Background session \"{description}\" completed")
    } else {
        format!("Background session \"{description}\" failed")
    }
}

/// Build the monitor-kind completion summary.
/// Port of TS `LocalShellTask.tsx:136` monitor branch.
pub fn shell_monitor_summary(
    status: AgentTaskStatus,
    description: &str,
    exit_code: Option<i32>,
) -> String {
    match status {
        AgentTaskStatus::Completed => format!("Monitor \"{description}\" stream ended"),
        AgentTaskStatus::Failed => {
            if let Some(code) = exit_code {
                format!("Monitor \"{description}\" script failed (exit {code})")
            } else {
                format!("Monitor \"{description}\" script failed")
            }
        }
        AgentTaskStatus::Stopped => format!("Monitor \"{description}\" stopped"),
    }
}

/// Build the bash-kind shell-task completion summary with the
/// [`BACKGROUND_BASH_SUMMARY_PREFIX`]. Port of TS
/// `LocalShellTask.tsx:136` bash branch.
pub fn shell_bash_summary(
    status: AgentTaskStatus,
    description: &str,
    exit_code: Option<i32>,
) -> String {
    match status {
        AgentTaskStatus::Completed => {
            let tail = exit_code
                .map(|c| format!(" (exit code {c})"))
                .unwrap_or_default();
            format!("{BACKGROUND_BASH_SUMMARY_PREFIX}\"{description}\" completed{tail}")
        }
        AgentTaskStatus::Failed => {
            let tail = exit_code
                .map(|c| format!(" with exit code {c}"))
                .unwrap_or_default();
            format!("{BACKGROUND_BASH_SUMMARY_PREFIX}\"{description}\" failed{tail}")
        }
        AgentTaskStatus::Stopped => {
            format!("{BACKGROUND_BASH_SUMMARY_PREFIX}\"{description}\" was stopped")
        }
    }
}

/// Build the `<description> appears to be waiting for interactive
/// input` summary used by the stalled-shell notification. Port
/// of TS `LocalShellTask.tsx:75`.
pub fn stalled_shell_summary(description: &str) -> String {
    format!(
        "{BACKGROUND_BASH_SUMMARY_PREFIX}\"{description}\" appears to be waiting for interactive input"
    )
}

/// Trailing guidance added to the stalled-shell notification.
/// Tells the model to kill + retry with piped input. Port of
/// TS `LocalShellTask.tsx:84-86`.
pub const STALLED_SHELL_TAIL: &str =
    "The command is likely blocked on an interactive prompt. Kill this task and re-run with piped input (e.g., `echo y | command`) or a non-interactive flag if one exists.";

/// Lowercase XML escape — escapes the five XML metacharacters.
/// Matches TS `utils/escapeXml` (content-only, not attribute
/// escaping).
pub fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Optional worktree block for the agent-task notification.
pub struct WorktreeBlock<'a> {
    pub path: &'a str,
    pub branch: &'a str,
}

/// Fields for [`build_task_notification_xml`].
pub struct TaskNotificationInputs<'a> {
    pub task_id: &'a str,
    /// Live tool-use id, if the task is tied to a specific tool
    /// call. Omitting matches TS's `toolUseIdLine = ''` branch.
    pub tool_use_id: Option<&'a str>,
    /// Absolute path of the file the task writes its output to.
    pub output_file: &'a str,
    /// Stringified status. Callers can pass
    /// `AgentTaskStatus::as_str()` or their own wire string
    /// (shell tasks don't always emit a status line).
    pub status: Option<&'a str>,
    /// Pre-built human-readable summary (output of one of the
    /// `*_summary` builders above). Already XML-escaped on
    /// stalled-shell path; other builders emit plain text and
    /// this function escapes unconditionally.
    pub summary: &'a str,
    /// Optional `<result>…</result>` block. TS wraps the agent's
    /// final assistant text in a dedicated element on the
    /// agent-task path.
    pub result_section: &'a str,
    /// Optional `<usage>…</usage>` block (token counts). TS
    /// emits this on the agent-task path when usage is
    /// available.
    pub usage_section: &'a str,
    pub worktree: Option<WorktreeBlock<'a>>,
}

/// Assemble the `<task-notification>` XML block. Port of the
/// three TS builders (LocalAgentTask/LocalMainSessionTask/
/// LocalShellTask) collapsed into one — each caller supplies the
/// optional bits it cares about.
pub fn build_task_notification_xml(inputs: &TaskNotificationInputs<'_>) -> String {
    let mut out = format!("<{TASK_NOTIFICATION_TAG}>\n");
    out.push_str(&format!(
        "<{TASK_ID_TAG}>{id}</{TASK_ID_TAG}>",
        id = escape_xml(inputs.task_id)
    ));
    if let Some(tool_use_id) = inputs.tool_use_id {
        out.push_str(&format!(
            "\n<{TOOL_USE_ID_TAG}>{id}</{TOOL_USE_ID_TAG}>",
            id = escape_xml(tool_use_id)
        ));
    }
    out.push('\n');
    out.push_str(&format!(
        "<{OUTPUT_FILE_TAG}>{p}</{OUTPUT_FILE_TAG}>\n",
        p = escape_xml(inputs.output_file)
    ));
    if let Some(status) = inputs.status {
        out.push_str(&format!(
            "<{STATUS_TAG}>{s}</{STATUS_TAG}>\n",
            s = escape_xml(status)
        ));
    }
    out.push_str(&format!(
        "<{SUMMARY_TAG}>{s}</{SUMMARY_TAG}>",
        s = escape_xml(inputs.summary)
    ));
    if !inputs.result_section.is_empty() {
        out.push_str(inputs.result_section);
    }
    if !inputs.usage_section.is_empty() {
        out.push_str(inputs.usage_section);
    }
    if let Some(wt) = &inputs.worktree {
        out.push_str(&format!(
            "\n<{WORKTREE_TAG}>\n<{WORKTREE_PATH_TAG}>{p}</{WORKTREE_PATH_TAG}>\n<{WORKTREE_BRANCH_TAG}>{b}</{WORKTREE_BRANCH_TAG}>\n</{WORKTREE_TAG}>",
            p = escape_xml(wt.path),
            b = escape_xml(wt.branch),
        ));
    }
    out.push_str(&format!("\n</{TASK_NOTIFICATION_TAG}>"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_constants_match_ts_wire_names() {
        assert_eq!(TASK_NOTIFICATION_TAG, "task-notification");
        assert_eq!(TASK_ID_TAG, "task-id");
        assert_eq!(TOOL_USE_ID_TAG, "tool-use-id");
        assert_eq!(OUTPUT_FILE_TAG, "output-file");
        assert_eq!(STATUS_TAG, "status");
        assert_eq!(SUMMARY_TAG, "summary");
        assert_eq!(WORKTREE_TAG, "worktree");
        assert_eq!(WORKTREE_PATH_TAG, "worktreePath");
        assert_eq!(WORKTREE_BRANCH_TAG, "worktreeBranch");
    }

    #[test]
    fn agent_summary_three_statuses() {
        assert_eq!(
            agent_task_summary(AgentTaskStatus::Completed, "refactor auth", None),
            "Agent \"refactor auth\" completed"
        );
        assert_eq!(
            agent_task_summary(AgentTaskStatus::Failed, "audit", Some("timeout")),
            "Agent \"audit\" failed: timeout"
        );
        assert_eq!(
            agent_task_summary(AgentTaskStatus::Failed, "audit", None),
            "Agent \"audit\" failed: Unknown error"
        );
        assert_eq!(
            agent_task_summary(AgentTaskStatus::Stopped, "migrate", None),
            "Agent \"migrate\" was stopped"
        );
    }

    #[test]
    fn session_summary_completed_vs_failed() {
        assert_eq!(
            background_session_summary(true, "deploy"),
            "Background session \"deploy\" completed"
        );
        assert_eq!(
            background_session_summary(false, "deploy"),
            "Background session \"deploy\" failed"
        );
    }

    #[test]
    fn shell_bash_summary_with_and_without_exit_code() {
        assert_eq!(
            shell_bash_summary(AgentTaskStatus::Completed, "npm test", Some(0)),
            "Background command \"npm test\" completed (exit code 0)"
        );
        assert_eq!(
            shell_bash_summary(AgentTaskStatus::Failed, "npm test", Some(1)),
            "Background command \"npm test\" failed with exit code 1"
        );
        assert_eq!(
            shell_bash_summary(AgentTaskStatus::Completed, "sleep 5", None),
            "Background command \"sleep 5\" completed"
        );
        assert_eq!(
            shell_bash_summary(AgentTaskStatus::Stopped, "long-run", None),
            "Background command \"long-run\" was stopped"
        );
    }

    #[test]
    fn shell_monitor_summary_phrases_per_status() {
        assert_eq!(
            shell_monitor_summary(AgentTaskStatus::Completed, "log-tail", None),
            "Monitor \"log-tail\" stream ended"
        );
        assert_eq!(
            shell_monitor_summary(AgentTaskStatus::Failed, "log-tail", Some(2)),
            "Monitor \"log-tail\" script failed (exit 2)"
        );
        assert_eq!(
            shell_monitor_summary(AgentTaskStatus::Failed, "log-tail", None),
            "Monitor \"log-tail\" script failed"
        );
        assert_eq!(
            shell_monitor_summary(AgentTaskStatus::Stopped, "log-tail", None),
            "Monitor \"log-tail\" stopped"
        );
    }

    #[test]
    fn stalled_shell_summary_uses_bash_prefix() {
        let s = stalled_shell_summary("prompt-cmd");
        assert!(s.starts_with(BACKGROUND_BASH_SUMMARY_PREFIX));
        assert!(s.ends_with("appears to be waiting for interactive input"));
    }

    #[test]
    fn escape_xml_replaces_all_five_metachars() {
        assert_eq!(escape_xml("a<b>&\"'c"), "a&lt;b&gt;&amp;&quot;&apos;c");
    }

    #[test]
    fn build_minimal_notification_matches_ts_shape() {
        let xml = build_task_notification_xml(&TaskNotificationInputs {
            task_id: "t1",
            tool_use_id: None,
            output_file: "/tmp/out.txt",
            status: Some("completed"),
            summary: "Agent \"x\" completed",
            result_section: "",
            usage_section: "",
            worktree: None,
        });
        assert!(xml.starts_with("<task-notification>\n"));
        assert!(xml.contains("<task-id>t1</task-id>"));
        assert!(xml.contains("<output-file>/tmp/out.txt</output-file>"));
        assert!(xml.contains("<status>completed</status>"));
        assert!(xml.contains("<summary>Agent &quot;x&quot; completed</summary>"));
        assert!(xml.ends_with("</task-notification>"));
        // No tool-use-id, no worktree.
        assert!(!xml.contains("<tool-use-id>"));
        assert!(!xml.contains("<worktree>"));
    }

    #[test]
    fn build_full_notification_carries_tool_use_and_worktree() {
        let xml = build_task_notification_xml(&TaskNotificationInputs {
            task_id: "t1",
            tool_use_id: Some("tu-42"),
            output_file: "/tmp/out.txt",
            status: Some("completed"),
            summary: "done",
            result_section: "\n<result>final text</result>",
            usage_section: "\n<usage>input:100 output:200</usage>",
            worktree: Some(WorktreeBlock {
                path: "/tmp/wt",
                branch: "worktree-agent-abc",
            }),
        });
        assert!(xml.contains("<tool-use-id>tu-42</tool-use-id>"));
        assert!(xml.contains("<result>final text</result>"));
        assert!(xml.contains("<usage>input:100 output:200</usage>"));
        assert!(xml.contains("<worktree>"));
        assert!(xml.contains("<worktreePath>/tmp/wt</worktreePath>"));
        assert!(xml.contains("<worktreeBranch>worktree-agent-abc</worktreeBranch>"));
    }
}
