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

/// Fire a `StopFailure` hook via the global runner, if one is installed.
/// Logs blocking errors via tracing and returns the joined error message.
/// Returns `None` if no runner is installed, no hooks matched, or no hook
/// produced a blocking error.
///
/// Mirrors TS `executeStopFailureHooks` entry behaviour (without the full
/// lastMessage plumbing — callers pass the error string directly).
pub async fn fire_stop_failure(reason: &str) -> Option<String> {
    let runner = get_global_runner()?;
    let extra = serde_json::json!({ "error": reason });
    let result = runner
        .run_hooks(
            &types::HookEvent::StopFailure,
            extra,
            None,
            None,
            None,
            None,
        )
        .await;
    if result.blocking_errors.is_empty() {
        return None;
    }
    let msg = result
        .blocking_errors
        .iter()
        .map(runner::get_stop_hook_message)
        .collect::<Vec<_>>()
        .join("\n");
    tracing::warn!("StopFailure hook feedback: {}", msg);
    Some(msg)
}

// Re-export the most commonly used types at the module level.
pub use aggregation::aggregate_hook_results;
pub use matching::{get_matching_hooks, matches_pattern, resolve_match_query, MatchedHook};
pub use runner::{
    get_pre_tool_hook_blocking_message, get_stop_hook_message, get_task_completed_hook_message,
    get_task_created_hook_message, get_teammate_idle_hook_message,
    get_user_prompt_submit_hook_blocking_message, HookRunner,
};
pub use tool_hooks::{
    resolve_hook_permission_decision, run_post_tool_use_failure_hooks, run_post_tool_use_hooks,
    run_pre_tool_use_hooks, PostToolUseFailureHookDecision, PostToolUseHookDecision,
    PreToolUseHookDecision, ResolvedPermission, RuleCheckResult,
};
pub use types::*;
