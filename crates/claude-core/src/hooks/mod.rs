pub mod aggregation;
pub mod matching;
pub mod runner;
pub mod tool_hooks;
pub mod types;

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
