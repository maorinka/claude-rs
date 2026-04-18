pub mod aggregation;
pub mod matching;
pub mod runner;
pub mod ssrf;
pub mod tool_hooks;
pub mod types;

// ── Process-wide HookRunner handle ───────────────────────────────────────────
//
// Tools that run outside the REPL loop (e.g. TaskCreateTool firing
// TaskCreated hooks from claude-tools) need a way to reach the configured
// HookRunner without plumbing it through every ToolUseContext construction
// site. The CLI/TUI entry point installs one at startup via
// `set_global_runner`; tools that want to fire an event call
// `get_global_runner` and no-op if nothing was installed (matches the TS
// behaviour where hooks are optional config).

use std::sync::{Arc, OnceLock};

static GLOBAL_RUNNER: OnceLock<Arc<runner::HookRunner>> = OnceLock::new();

/// Install the process-wide HookRunner. Only the first call wins — subsequent
/// calls are silently ignored (matches `OnceLock` semantics).
pub fn set_global_runner(runner: Arc<runner::HookRunner>) {
    let _ = GLOBAL_RUNNER.set(runner);
}

/// Fetch the registered HookRunner if one has been installed.
pub fn get_global_runner() -> Option<Arc<runner::HookRunner>> {
    GLOBAL_RUNNER.get().cloned()
}

// Re-export the most commonly used types at the module level.
pub use aggregation::aggregate_hook_results;
pub use matching::{get_matching_hooks, matches_pattern, resolve_match_query, MatchedHook};
pub use runner::{
    get_pre_tool_hook_blocking_message, get_stop_hook_message,
    get_task_completed_hook_message, get_task_created_hook_message,
    get_teammate_idle_hook_message, get_user_prompt_submit_hook_blocking_message, HookRunner,
};
pub use tool_hooks::{
    resolve_hook_permission_decision, run_post_tool_use_failure_hooks,
    run_post_tool_use_hooks, run_pre_tool_use_hooks, PostToolUseFailureHookDecision,
    PostToolUseHookDecision, PreToolUseHookDecision, ResolvedPermission, RuleCheckResult,
};
pub use types::*;
