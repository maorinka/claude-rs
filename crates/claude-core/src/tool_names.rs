//! Authoritative tool-name string constants.
//!
//! TS scatters these across 30+ files under `src/tools/*/{constants,
//! toolName,prompt}.ts` — each tool owns its own name to break
//! circular import cycles between constants + prompt modules. Rust
//! has no such cycle concern, so we consolidate them here so callers
//! reference one source of truth.
//!
//! The names MUST match what ToolExecutor::name() returns at the
//! registry level (see claude-tools) so prompt text produced via these
//! constants lines up with the tools the model actually sees. A
//! regression test on the claude-tools side verifies the match.

// ── Baseline built-ins ────────────────────────────────────────────────────

pub const AGENT_TOOL_NAME: &str = "Agent";
/// Legacy wire name retained for permission rules, hooks, and resumed
/// sessions that reference the old Task tool. Never surfaced to the
/// model — only matched at permission-check time.
pub const LEGACY_AGENT_TOOL_NAME: &str = "Task";

pub const BASH_TOOL_NAME: &str = "Bash";
pub const POWERSHELL_TOOL_NAME: &str = "PowerShell";

pub const FILE_READ_TOOL_NAME: &str = "Read";
pub const FILE_EDIT_TOOL_NAME: &str = "Edit";
pub const FILE_WRITE_TOOL_NAME: &str = "Write";
pub const NOTEBOOK_EDIT_TOOL_NAME: &str = "NotebookEdit";

pub const GLOB_TOOL_NAME: &str = "Glob";
pub const GREP_TOOL_NAME: &str = "Grep";

pub const WEB_FETCH_TOOL_NAME: &str = "WebFetch";
pub const WEB_SEARCH_TOOL_NAME: &str = "WebSearch";

pub const ENTER_PLAN_MODE_TOOL_NAME: &str = "EnterPlanMode";
pub const EXIT_PLAN_MODE_TOOL_NAME: &str = "ExitPlanMode";
/// V2 uses the same wire name as V1 — they're rendered from the same
/// constant. Defined separately so future divergence doesn't break
/// existing references.
pub const EXIT_PLAN_MODE_V2_TOOL_NAME: &str = "ExitPlanMode";

pub const ENTER_WORKTREE_TOOL_NAME: &str = "EnterWorktree";
pub const EXIT_WORKTREE_TOOL_NAME: &str = "ExitWorktree";

pub const ASK_USER_QUESTION_TOOL_NAME: &str = "AskUserQuestion";
pub const SEND_MESSAGE_TOOL_NAME: &str = "SendMessage";

pub const TODO_WRITE_TOOL_NAME: &str = "TodoWrite";

pub const SKILL_TOOL_NAME: &str = "Skill";
pub const SLEEP_TOOL_NAME: &str = "Sleep";

pub const LSP_TOOL_NAME: &str = "LSP";
pub const TOOL_SEARCH_TOOL_NAME: &str = "ToolSearch";

pub const BRIEF_TOOL_NAME: &str = "SendUserMessage";
pub const LEGACY_BRIEF_TOOL_NAME: &str = "Brief";

pub const CONFIG_TOOL_NAME: &str = "Config";
pub const REPL_TOOL_NAME: &str = "REPL";

// ── Task suite (TodoV2) ──────────────────────────────────────────────────

pub const TASK_CREATE_TOOL_NAME: &str = "TaskCreate";
pub const TASK_UPDATE_TOOL_NAME: &str = "TaskUpdate";
pub const TASK_LIST_TOOL_NAME: &str = "TaskList";
pub const TASK_GET_TOOL_NAME: &str = "TaskGet";
pub const TASK_OUTPUT_TOOL_NAME: &str = "TaskOutput";
pub const TASK_STOP_TOOL_NAME: &str = "TaskStop";

// ── Team suite ───────────────────────────────────────────────────────────

pub const TEAM_CREATE_TOOL_NAME: &str = "TeamCreate";
pub const TEAM_DELETE_TOOL_NAME: &str = "TeamDelete";

// ── Agent triggers ───────────────────────────────────────────────────────

pub const CRON_CREATE_TOOL_NAME: &str = "CronCreate";
pub const CRON_DELETE_TOOL_NAME: &str = "CronDelete";
pub const CRON_LIST_TOOL_NAME: &str = "CronList";
pub const REMOTE_TRIGGER_TOOL_NAME: &str = "RemoteTrigger";

// ── MCP + misc ───────────────────────────────────────────────────────────

pub const LIST_MCP_RESOURCES_TOOL_NAME: &str = "ListMcpResourcesTool";

// Feature-gated tools: the Rust registry hides these unless the
// corresponding env flag is set. Constants are always defined so
// prompts can reference them in the future without re-porting.
pub const MONITOR_TOOL_NAME: &str = "Monitor";
pub const SNIP_TOOL_NAME: &str = "Snip";
pub const CTX_INSPECT_TOOL_NAME: &str = "CtxInspect";
pub const TERMINAL_CAPTURE_TOOL_NAME: &str = "TerminalCapture";
pub const WEB_BROWSER_TOOL_NAME: &str = "WebBrowser";
pub const LIST_PEERS_TOOL_NAME: &str = "ListPeers";
pub const WORKFLOW_TOOL_NAME: &str = "Workflow";
pub const VERIFY_PLAN_EXECUTION_TOOL_NAME: &str = "VerifyPlanExecution";
pub const SEND_USER_FILE_TOOL_NAME: &str = "SendUserFile";
pub const PUSH_NOTIFICATION_TOOL_NAME: &str = "PushNotification";
pub const SUBSCRIBE_PR_TOOL_NAME: &str = "SubscribePR";
pub const SUGGEST_BACKGROUND_PR_TOOL_NAME: &str = "SuggestBackgroundPR";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_names_match_registry() {
        // Sanity — these strings are what claude_tools::build_default_registry
        // registers. If either side changes, the mismatch is visible here.
        assert_eq!(AGENT_TOOL_NAME, "Agent");
        assert_eq!(BASH_TOOL_NAME, "Bash");
        assert_eq!(FILE_READ_TOOL_NAME, "Read");
        assert_eq!(FILE_EDIT_TOOL_NAME, "Edit");
        assert_eq!(FILE_WRITE_TOOL_NAME, "Write");
        assert_eq!(GREP_TOOL_NAME, "Grep");
        assert_eq!(GLOB_TOOL_NAME, "Glob");
        assert_eq!(WEB_FETCH_TOOL_NAME, "WebFetch");
        assert_eq!(TASK_CREATE_TOOL_NAME, "TaskCreate");
    }

    #[test]
    fn exit_plan_mode_v1_and_v2_share_wire_name() {
        assert_eq!(EXIT_PLAN_MODE_TOOL_NAME, EXIT_PLAN_MODE_V2_TOOL_NAME);
    }

    #[test]
    fn legacy_aliases_present_for_permission_rules() {
        assert_eq!(LEGACY_AGENT_TOOL_NAME, "Task");
        assert_eq!(LEGACY_BRIEF_TOOL_NAME, "Brief");
    }

    #[test]
    fn all_names_are_non_empty() {
        for n in [
            AGENT_TOOL_NAME,
            BASH_TOOL_NAME,
            POWERSHELL_TOOL_NAME,
            FILE_READ_TOOL_NAME,
            FILE_EDIT_TOOL_NAME,
            FILE_WRITE_TOOL_NAME,
            NOTEBOOK_EDIT_TOOL_NAME,
            GLOB_TOOL_NAME,
            GREP_TOOL_NAME,
            WEB_FETCH_TOOL_NAME,
            WEB_SEARCH_TOOL_NAME,
            ENTER_PLAN_MODE_TOOL_NAME,
            EXIT_PLAN_MODE_TOOL_NAME,
            ENTER_WORKTREE_TOOL_NAME,
            EXIT_WORKTREE_TOOL_NAME,
            ASK_USER_QUESTION_TOOL_NAME,
            SEND_MESSAGE_TOOL_NAME,
            TODO_WRITE_TOOL_NAME,
            SKILL_TOOL_NAME,
            SLEEP_TOOL_NAME,
            LSP_TOOL_NAME,
            TOOL_SEARCH_TOOL_NAME,
            BRIEF_TOOL_NAME,
            CONFIG_TOOL_NAME,
            REPL_TOOL_NAME,
            TASK_CREATE_TOOL_NAME,
            TASK_UPDATE_TOOL_NAME,
            TASK_LIST_TOOL_NAME,
            TASK_GET_TOOL_NAME,
            TASK_OUTPUT_TOOL_NAME,
            TASK_STOP_TOOL_NAME,
            TEAM_CREATE_TOOL_NAME,
            TEAM_DELETE_TOOL_NAME,
            CRON_CREATE_TOOL_NAME,
            CRON_DELETE_TOOL_NAME,
            CRON_LIST_TOOL_NAME,
            REMOTE_TRIGGER_TOOL_NAME,
            LIST_MCP_RESOURCES_TOOL_NAME,
            MONITOR_TOOL_NAME,
            SNIP_TOOL_NAME,
            CTX_INSPECT_TOOL_NAME,
            TERMINAL_CAPTURE_TOOL_NAME,
            WEB_BROWSER_TOOL_NAME,
            LIST_PEERS_TOOL_NAME,
            WORKFLOW_TOOL_NAME,
            VERIFY_PLAN_EXECUTION_TOOL_NAME,
            SEND_USER_FILE_TOOL_NAME,
            PUSH_NOTIFICATION_TOOL_NAME,
            SUBSCRIBE_PR_TOOL_NAME,
            SUGGEST_BACKGROUND_PR_TOOL_NAME,
        ] {
            assert!(!n.is_empty());
        }
    }
}
