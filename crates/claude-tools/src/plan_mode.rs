use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// ---------------------------------------------------------------------------
// Global plan-mode state
// ---------------------------------------------------------------------------

/// Process-wide plan mode flag. When active, permission checks should return
/// Ask for ALL tools (preventing execution until ExitPlanMode is called).
static PLAN_MODE_ACTIVE: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

/// Track whether full instructions have been delivered (for sparse reminder).
static FULL_INSTRUCTIONS_DELIVERED: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

/// Plan file path for the current session.
static PLAN_FILE_PATH: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

/// Phase 4 variant: 0=control, 1=trim, 2=cut, 3=cap.
static PLAN_PHASE4_VARIANT: Lazy<AtomicU8> = Lazy::new(|| AtomicU8::new(0));

/// Check whether plan mode is currently active.
pub fn is_plan_mode_active() -> bool {
    PLAN_MODE_ACTIVE.load(Ordering::SeqCst)
}

/// Set plan mode state. Called by EnterPlanModeTool and ExitPlanModeTool.
pub fn set_plan_mode(active: bool) {
    PLAN_MODE_ACTIVE.store(active, Ordering::SeqCst);
    if !active {
        FULL_INSTRUCTIONS_DELIVERED.store(false, Ordering::SeqCst);
    }
}

/// Check if a tool should be blocked by plan mode.
///
/// In plan mode, only read-only tools, EnterPlanMode, ExitPlanMode, and
/// AskUserQuestion is allowed to proceed. All other tools should require explicit
/// user confirmation (Ask).
pub fn should_plan_mode_block(tool_name: &str, is_read_only: bool) -> bool {
    if !is_plan_mode_active() {
        return false;
    }

    match tool_name {
        "EnterPlanMode" | "ExitPlanMode" | "AskUserQuestion" | "AskUser" | "Brief" => false,
        _ if is_read_only => false,
        _ => true,
    }
}

/// Set the Phase 4 variant (0=control, 1=trim, 2=cut, 3=cap).
pub fn set_plan_phase4_variant(variant: u8) {
    PLAN_PHASE4_VARIANT.store(variant.min(3), Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Plan file management (mirrors TS utils/plans.ts)
// ---------------------------------------------------------------------------

/// Generate a plan file path under `~/.claude/plans/`.
/// Uses a simple session-based naming scheme.
fn ensure_plan_file_path() -> PathBuf {
    let mut guard = PLAN_FILE_PATH.lock().unwrap();
    if let Some(ref path) = *guard {
        return path.clone();
    }

    let plans_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("plans");

    // Create the plans directory if it doesn't exist
    let _ = std::fs::create_dir_all(&plans_dir);

    // Generate a slug from timestamp
    let slug = format!("plan-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let path = plans_dir.join(format!("{}.md", slug));

    *guard = Some(path.clone());
    path
}

/// Get the current plan file path (if set).
pub fn get_plan_file_path() -> Option<PathBuf> {
    PLAN_FILE_PATH.lock().unwrap().clone()
}

/// Read the plan from disk (if it exists).
fn read_plan_file() -> Option<String> {
    let path = get_plan_file_path()?;
    std::fs::read_to_string(&path).ok()
}

// ---------------------------------------------------------------------------
// Phase 4 variants (mirrors TS PLAN_PHASE4_* from messages.ts)
// ---------------------------------------------------------------------------

const PLAN_PHASE4_CONTROL: &str = "\
### Phase 4: Final Plan
Goal: Write your final plan to the plan file (the only file you can edit).
- Begin with a **Context** section: explain why this change is being made — the problem or need it addresses, what prompted it, and the intended outcome
- Include only your recommended approach, not all alternatives
- Ensure that the plan file is concise enough to scan quickly, but detailed enough to execute effectively
- Include the paths of critical files to be modified
- Reference existing functions and utilities you found that should be reused, with their file paths
- Include a verification section describing how to test the changes end-to-end (run the code, use MCP tools, run tests)";

const PLAN_PHASE4_TRIM: &str = "\
### Phase 4: Final Plan
Goal: Write your final plan to the plan file (the only file you can edit).
- One-line **Context**: what is being changed and why
- Include only your recommended approach, not all alternatives
- List the paths of files to be modified
- Reference existing functions and utilities to reuse, with their file paths
- End with **Verification**: the single command to run to confirm the change works (no numbered test procedures)";

const PLAN_PHASE4_CUT: &str = "\
### Phase 4: Final Plan
Goal: Write your final plan to the plan file (the only file you can edit).
- Do NOT write a Context or Background section. The user just told you what they want.
- List the paths of files to be modified and what changes in each (one line per file)
- Reference existing functions and utilities to reuse, with their file paths
- End with **Verification**: the single command that confirms the change works
- Most good plans are under 40 lines. Prose is a sign you are padding.";

const PLAN_PHASE4_CAP: &str = "\
### Phase 4: Final Plan
Goal: Write your final plan to the plan file (the only file you can edit).
- Do NOT write a Context, Background, or Overview section. The user just told you what they want.
- Do NOT restate the user's request. Do NOT write prose paragraphs.
- List the paths of files to be modified and what changes in each (one bullet per file)
- Reference existing functions to reuse, with file:line
- End with the single verification command
- **Hard limit: 40 lines.** If the plan is longer, delete prose — not file paths.";

fn get_phase4_section() -> &'static str {
    match PLAN_PHASE4_VARIANT.load(Ordering::SeqCst) {
        1 => PLAN_PHASE4_TRIM,
        2 => PLAN_PHASE4_CUT,
        3 => PLAN_PHASE4_CAP,
        _ => PLAN_PHASE4_CONTROL,
    }
}

// ---------------------------------------------------------------------------
// Plan mode system prompt section (injected via register_system_prompt_section)
// ---------------------------------------------------------------------------

/// Returns the plan mode section for the system prompt.
/// Full instructions on first call, sparse reminder on subsequent calls.
pub fn get_plan_mode_system_prompt_section() -> Option<String> {
    if !is_plan_mode_active() {
        return None;
    }

    let plan_file_path = ensure_plan_file_path();
    let plan_path_str = plan_file_path.display().to_string();

    if FULL_INSTRUCTIONS_DELIVERED.load(Ordering::SeqCst) {
        // Sparse reminder for subsequent turns
        Some(get_sparse_plan_instructions(&plan_path_str))
    } else {
        FULL_INSTRUCTIONS_DELIVERED.store(true, Ordering::SeqCst);
        Some(get_full_plan_instructions(&plan_path_str))
    }
}

/// Full 5-phase plan mode instructions (mirrors TS getPlanModeInstructions).
fn get_full_plan_instructions(plan_file_path: &str) -> String {
    let plan_file_info = if std::path::Path::new(plan_file_path).exists() {
        format!(
            "A plan file already exists at {}. You can read it and make incremental edits using the Edit tool.",
            plan_file_path
        )
    } else {
        format!(
            "No plan file exists yet. You should create your plan at {} using the Write tool.",
            plan_file_path
        )
    };

    let phase4 = get_phase4_section();

    format!(
        r#"Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits (with the exception of the plan file mentioned below), run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supercedes any other instructions you have received.

## Plan File Info:
{plan_file_info}

You should build your plan incrementally by writing to or editing this file. NOTE that this is the only file you are allowed to edit - other than this you are only allowed to take READ-ONLY actions.

## Plan Workflow

### Phase 1: Initial Understanding
Goal: Gain a comprehensive understanding of the user's request by reading through code and asking them questions. Critical: In this phase you should only use the Explore subagent type.

1. Focus on understanding the user's request and the code associated with their request. Actively search for existing functions, utilities, and patterns that can be reused — avoid proposing new code when suitable implementations already exist.

2. **Launch up to 3 Explore agents IN PARALLEL** (single message, multiple tool calls) to efficiently explore the codebase.
   - Use 1 agent when the task is isolated to known files, the user provided specific file paths, or you're making a small targeted change.
   - Use multiple agents when: the scope is uncertain, multiple areas of the codebase are involved, or you need to understand existing patterns before planning.
   - Quality over quantity - 3 agents maximum, but you should try to use the minimum number of agents necessary (usually just 1)
   - If using multiple agents: Provide each agent with a specific search focus or area to explore. Example: One agent searches for existing implementations, another explores related components, a third investigating testing patterns

### Phase 2: Design
Goal: Design an implementation approach.

Launch a Plan agent to design the implementation based on the user's intent and your exploration results from Phase 1.

**Guidelines:**
- **Default**: Launch at least 1 Plan agent for most tasks - it helps validate your understanding and consider alternatives
- **Skip agents**: Only for truly trivial tasks (typo fixes, single-line changes, simple renames)

In the agent prompt:
- Provide comprehensive background context from Phase 1 exploration including filenames and code path traces
- Describe requirements and constraints
- Request a detailed implementation plan

### Phase 3: Review
Goal: Review the plan(s) from Phase 2 and ensure alignment with the user's intentions.
1. Read the critical files identified by agents to deepen your understanding
2. Ensure that the plans align with the user's original request
3. Use AskUserQuestion to clarify any remaining questions with the user

{phase4}

### Phase 5: Call ExitPlanMode
At the very end of your turn, once you have asked the user questions and are happy with your final plan file - you should always call ExitPlanMode to indicate to the user that you are done planning.
This is critical - your turn should only end with either using the AskUserQuestion tool OR calling ExitPlanMode. Do not stop unless it's for these 2 reasons

**Important:** Use AskUserQuestion ONLY to clarify requirements or choose between approaches. Use ExitPlanMode to request plan approval. Do NOT ask about plan approval in any other way - no text questions, no AskUserQuestion. Phrases like "Is this plan okay?", "Should I proceed?", "How does this plan look?", "Any changes before we start?", or similar MUST use ExitPlanMode.

NOTE: At any point in time through this workflow you should feel free to ask the user questions or clarifications using the AskUserQuestion tool. Don't make large assumptions about user intent. The goal is to present a well researched plan to the user, and tie any loose ends before implementation begins."#,
        plan_file_info = plan_file_info,
        phase4 = phase4,
    )
}

/// Sparse plan mode reminder for subsequent turns (mirrors TS sparse variant).
fn get_sparse_plan_instructions(plan_file_path: &str) -> String {
    format!(
        "Plan mode still active (see full instructions earlier in conversation). \
         Read-only except plan file ({}). Follow 5-phase workflow. \
         End turns with AskUserQuestion (for clarifications) or ExitPlanMode (for plan approval). \
         Never ask about plan approval via text or AskUserQuestion.",
        plan_file_path
    )
}

/// Plan mode exit instruction injected after ExitPlanMode succeeds.
fn get_plan_exit_instruction() -> String {
    let plan_path = get_plan_file_path();
    let plan_ref = match plan_path {
        Some(ref p) => format!(
            " The plan file is located at {} if you need to reference it.",
            p.display()
        ),
        None => String::new(),
    };
    format!(
        "## Exited Plan Mode\n\n\
         You have exited plan mode. You can now make edits, run tools, and take actions.{}",
        plan_ref
    )
}

// ---------------------------------------------------------------------------
// EnterPlanModeTool (mirrors TS EnterPlanModeTool/prompt.ts)
// ---------------------------------------------------------------------------

/// Full EnterPlanMode description matching the TS external prompt.
const ENTER_PLAN_MODE_PROMPT: &str = r#"Use this tool proactively when you're about to start a non-trivial implementation task. Getting user sign-off on your approach before writing code prevents wasted effort and ensures alignment. This tool transitions you into plan mode where you can explore the codebase and design an implementation approach for user approval.

## When to Use This Tool

**Prefer using EnterPlanMode** for implementation tasks unless they're simple. Use it when ANY of these conditions apply:

1. **New Feature Implementation**: Adding meaningful new functionality
   - Example: "Add a logout button" - where should it go? What should happen on click?
   - Example: "Add form validation" - what rules? What error messages?

2. **Multiple Valid Approaches**: The task can be solved in several different ways
   - Example: "Add caching to the API" - could use Redis, in-memory, file-based, etc.
   - Example: "Improve performance" - many optimization strategies possible

3. **Code Modifications**: Changes that affect existing behavior or structure
   - Example: "Update the login flow" - what exactly should change?
   - Example: "Refactor this component" - what's the target architecture?

4. **Architectural Decisions**: The task requires choosing between patterns or technologies
   - Example: "Add real-time updates" - WebSockets vs SSE vs polling
   - Example: "Implement state management" - Redux vs Context vs custom solution

5. **Multi-File Changes**: The task will likely touch more than 2-3 files
   - Example: "Refactor the authentication system"
   - Example: "Add a new API endpoint with tests"

6. **Unclear Requirements**: You need to explore before understanding the full scope
   - Example: "Make the app faster" - need to profile and identify bottlenecks
   - Example: "Fix the bug in checkout" - need to investigate root cause

7. **User Preferences Matter**: The implementation could reasonably go multiple ways
   - If you would use AskUserQuestion to clarify the approach, use EnterPlanMode instead
   - Plan mode lets you explore first, then present options with context

## When NOT to Use This Tool

Only skip EnterPlanMode for simple tasks:
- Single-line or few-line fixes (typos, obvious bugs, small tweaks)
- Adding a single function with clear requirements
- Tasks where the user has given very specific, detailed instructions
- Pure research/exploration tasks (use the Agent tool with explore agent instead)

## What Happens in Plan Mode

In plan mode, you'll:
1. Thoroughly explore the codebase using Glob, Grep, and Read tools
2. Understand existing patterns and architecture
3. Design an implementation approach
4. Present your plan to the user for approval
5. Use AskUserQuestion if you need to clarify approaches
6. Exit plan mode with ExitPlanMode when ready to implement

## Important Notes

- This tool REQUIRES user approval - they must consent to entering plan mode
- If unsure whether to use it, err on the side of planning - it's better to get alignment upfront than to redo work
- Users appreciate being consulted before significant changes are made to their codebase"#;

pub struct EnterPlanModeTool;

#[async_trait]
impl ToolExecutor for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }

    fn description(&self) -> String {
        ENTER_PLAN_MODE_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        set_plan_mode(true);

        let plan_file_path = ensure_plan_file_path();
        let plan_path_str = plan_file_path.display().to_string();

        // Deliver full plan mode instructions on entry
        FULL_INSTRUCTIONS_DELIVERED.store(false, Ordering::SeqCst);
        let instructions = get_full_plan_instructions(&plan_path_str);

        Ok(ToolResultData {
            data: json!({
                "mode": "plan",
                "planFilePath": plan_path_str,
                "message": "Entered plan mode. You should now focus on exploring the \
                            codebase and designing an implementation approach.",
                "instructions": instructions,
            }),
            is_error: false,
        })
    }
}

// ---------------------------------------------------------------------------
// ExitPlanModeTool (mirrors TS ExitPlanModeTool/ExitPlanModeV2Tool.ts)
// ---------------------------------------------------------------------------

/// Full ExitPlanMode description matching the TS prompt.
const EXIT_PLAN_MODE_PROMPT: &str = r#"Use this tool when you are in plan mode and have finished writing your plan to the plan file and are ready for user approval.

## How This Tool Works
- You should have already written your plan to the plan file specified in the plan mode system message
- This tool does NOT take the plan content as a parameter - it will read the plan from the file you wrote
- This tool simply signals that you're done planning and ready for the user to review and approve
- The user will see the contents of your plan file when they review it

## When to Use This Tool
IMPORTANT: Only use this tool when the task requires planning the implementation steps of a task that requires writing code. For research tasks where you're gathering information, searching files, reading files or in general trying to understand the codebase - do NOT use this tool.

## Before Using This Tool
Ensure your plan is complete and unambiguous:
- If you have unresolved questions about requirements or approach, use AskUserQuestion first (in earlier phases)
- Once your plan is finalized, use THIS tool to request approval

**Important:** Do NOT use AskUserQuestion to ask "Is this plan okay?" or "Should I proceed?" - that's exactly what THIS tool does. ExitPlanMode inherently requests user approval of your plan.

## Examples

1. Initial task: "Search for and understand the implementation of vim mode in the codebase" - Do not use the exit plan mode tool because you are not planning the implementation steps of a task.
2. Initial task: "Help me implement yank mode for vim" - Use the exit plan mode tool after you have finished planning the implementation steps of the task.
3. Initial task: "Add a new feature to handle user authentication" - If unsure about auth method (OAuth, JWT, etc.), use AskUserQuestion first, then use exit plan mode tool after clarifying the approach."#;

pub struct ExitPlanModeTool;

#[async_trait]
impl ToolExecutor for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }

    fn description(&self) -> String {
        EXIT_PLAN_MODE_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        _input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        if !is_plan_mode_active() {
            return Ok(ToolResultData {
                data: json!({
                    "error": "You are not in plan mode. This tool is only for exiting plan \
                              mode after writing a plan. If your plan was already approved, \
                              continue with implementation."
                }),
                is_error: true,
            });
        }

        // Read the plan from the file
        let plan_content = read_plan_file();
        let plan_file_path = get_plan_file_path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        // Deactivate plan mode
        set_plan_mode(false);

        let exit_instruction = get_plan_exit_instruction();

        Ok(ToolResultData {
            data: json!({
                "mode": "normal",
                "plan": plan_content,
                "filePath": plan_file_path,
                "message": exit_instruction,
            }),
            is_error: false,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // test-only global-state serialization via std::sync::Mutex
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static PLAN_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn make_ctx() -> ToolUseContext {
        ToolUseContext::for_test(
            PathBuf::from("/tmp"),
            std::sync::Arc::new(std::sync::Mutex::new(crate::registry::ReadFileState::new())),
            crate::registry::PermissionMode::Default,
        )
    }

    fn reset_plan_state() {
        set_plan_mode(false);
        FULL_INSTRUCTIONS_DELIVERED.store(false, Ordering::SeqCst);
        *PLAN_FILE_PATH.lock().unwrap() = None;
    }

    #[tokio::test]
    async fn test_enter_plan_mode_activates_flag() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        reset_plan_state();
        assert!(!is_plan_mode_active());

        let tool = EnterPlanModeTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["mode"], "plan");
        assert!(is_plan_mode_active());

        reset_plan_state();
    }

    #[tokio::test]
    async fn test_exit_plan_mode_deactivates_flag() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        reset_plan_state();
        set_plan_mode(true);
        assert!(is_plan_mode_active());

        let tool = ExitPlanModeTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["mode"], "normal");
        assert!(!is_plan_mode_active());

        reset_plan_state();
    }

    #[tokio::test]
    async fn test_exit_plan_mode_errors_when_not_in_plan_mode() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        reset_plan_state();

        let tool = ExitPlanModeTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("not in plan mode"));
    }

    #[test]
    fn test_should_plan_mode_block_write_tools() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        set_plan_mode(true);

        assert!(should_plan_mode_block("Bash", false));
        assert!(should_plan_mode_block("Write", false));
        assert!(should_plan_mode_block("Edit", false));

        assert!(!should_plan_mode_block("Read", true));
        assert!(!should_plan_mode_block("Grep", true));
        assert!(!should_plan_mode_block("Glob", true));
        assert!(!should_plan_mode_block("LSP", true));

        assert!(!should_plan_mode_block("EnterPlanMode", true));
        assert!(!should_plan_mode_block("ExitPlanMode", false));
        assert!(!should_plan_mode_block("AskUserQuestion", true));
        assert!(!should_plan_mode_block("AskUser", true));
        assert!(!should_plan_mode_block("Brief", false));

        set_plan_mode(false);
    }

    #[test]
    fn test_plan_mode_does_not_block_when_inactive() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        set_plan_mode(false);

        assert!(!should_plan_mode_block("Bash", false));
        assert!(!should_plan_mode_block("Write", false));
        assert!(!should_plan_mode_block("Edit", false));
    }

    #[tokio::test]
    async fn test_enter_exit_roundtrip() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        reset_plan_state();

        let enter = EnterPlanModeTool;
        let result = enter
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(is_plan_mode_active());
        assert!(should_plan_mode_block("Bash", false));

        let exit = ExitPlanModeTool;
        let result = exit
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(!is_plan_mode_active());
        assert!(!should_plan_mode_block("Bash", false));

        reset_plan_state();
    }

    #[test]
    fn test_enter_plan_mode_properties() {
        let tool = EnterPlanModeTool;
        assert_eq!(tool.name(), "EnterPlanMode");
        assert!(tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn test_exit_plan_mode_properties() {
        let tool = ExitPlanModeTool;
        assert_eq!(tool.name(), "ExitPlanMode");
        assert!(!tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }

    #[tokio::test]
    async fn test_enter_plan_mode_includes_full_instructions() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        reset_plan_state();

        let tool = EnterPlanModeTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        let instructions = result.data["instructions"].as_str().unwrap();
        // Verify 5-phase workflow is present
        assert!(instructions.contains("Phase 1: Initial Understanding"));
        assert!(instructions.contains("Phase 2: Design"));
        assert!(instructions.contains("Phase 3: Review"));
        assert!(instructions.contains("Phase 4: Final Plan"));
        assert!(instructions.contains("Phase 5: Call ExitPlanMode"));
        // Verify plan file path is included
        assert!(result.data["planFilePath"].as_str().is_some());
        // Verify read-only constraint
        assert!(instructions.contains("MUST NOT make any edits"));
        assert!(instructions.contains("only file you are allowed to edit"));
        // Verify agent usage guidance
        assert!(instructions.contains("Explore agents IN PARALLEL"));
        assert!(instructions.contains("Plan agent"));
        // Verify turn-ending rules
        assert!(instructions.contains("AskUserQuestion"));
        assert!(instructions.contains("ExitPlanMode"));

        reset_plan_state();
    }

    #[test]
    fn test_enter_plan_mode_description_has_when_to_use() {
        let tool = EnterPlanModeTool;
        let desc = tool.description();
        assert!(desc.contains("When to Use This Tool"));
        assert!(desc.contains("When NOT to Use This Tool"));
        assert!(desc.contains("New Feature Implementation"));
        assert!(desc.contains("Multiple Valid Approaches"));
        assert!(desc.contains("Architectural Decisions"));
        assert!(desc.contains("Multi-File Changes"));
    }

    #[test]
    fn test_exit_plan_mode_description_has_guidance() {
        let tool = ExitPlanModeTool;
        let desc = tool.description();
        assert!(desc.contains("How This Tool Works"));
        assert!(desc.contains("When to Use This Tool"));
        assert!(desc.contains("Before Using This Tool"));
        assert!(desc.contains("Do NOT use AskUserQuestion to ask"));
    }

    #[test]
    fn test_system_prompt_section_none_when_inactive() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        reset_plan_state();
        assert!(get_plan_mode_system_prompt_section().is_none());
    }

    #[test]
    fn test_system_prompt_section_full_then_sparse() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        reset_plan_state();
        set_plan_mode(true);

        // First call: full instructions
        let first = get_plan_mode_system_prompt_section().unwrap();
        assert!(first.contains("Phase 1: Initial Understanding"));
        assert!(first.contains("Phase 2: Design"));
        assert!(first.contains("Phase 5: Call ExitPlanMode"));

        // Second call: sparse reminder
        let second = get_plan_mode_system_prompt_section().unwrap();
        assert!(second.contains("Plan mode still active"));
        assert!(second.contains("5-phase workflow"));
        assert!(!second.contains("Phase 1: Initial Understanding"));

        reset_plan_state();
    }

    #[test]
    fn test_phase4_variants() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();

        set_plan_phase4_variant(0);
        assert!(get_phase4_section().contains("Begin with a **Context** section"));

        set_plan_phase4_variant(1);
        assert!(get_phase4_section().contains("One-line **Context**"));

        set_plan_phase4_variant(2);
        assert!(get_phase4_section().contains("Do NOT write a Context or Background"));
        assert!(get_phase4_section().contains("under 40 lines"));

        set_plan_phase4_variant(3);
        assert!(get_phase4_section().contains("Hard limit: 40 lines"));
        assert!(get_phase4_section().contains("Do NOT restate the user's request"));

        set_plan_phase4_variant(0);
    }

    #[tokio::test]
    async fn test_exit_plan_mode_returns_plan_content() {
        let _guard = PLAN_TEST_LOCK.lock().unwrap();
        reset_plan_state();
        set_plan_mode(true);

        // Create a test plan file
        let plan_path = ensure_plan_file_path();
        std::fs::write(&plan_path, "# Test Plan\n\n## Context\nTest plan content").unwrap();

        let tool = ExitPlanModeTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["mode"], "normal");
        assert!(result.data["plan"]
            .as_str()
            .unwrap()
            .contains("Test plan content"));
        assert!(result.data["filePath"].as_str().is_some());
        assert!(result.data["message"]
            .as_str()
            .unwrap()
            .contains("Exited Plan Mode"));

        // Cleanup
        let _ = std::fs::remove_file(&plan_path);
        reset_plan_state();
    }
}
