use serde_json::Value;

use super::types::{PermissionDecision, PermissionMode, PermissionRule, ToolPermissionContext};

/// External hook for plan mode checking.
/// Returns true if the tool should be blocked (requires Ask) due to plan mode.
///
/// This is set by claude-tools::plan_mode to avoid a direct dependency from
/// claude-core to claude-tools.
pub type PlanModeChecker = fn(tool_name: &str, is_read_only: bool) -> bool;

/// Registered plan mode checker.
static PLAN_MODE_CHECKER: std::sync::Mutex<Option<PlanModeChecker>> = std::sync::Mutex::new(None);

/// Register the plan mode checker. Called during startup.
pub fn register_plan_mode_checker(checker: PlanModeChecker) {
    let mut guard = PLAN_MODE_CHECKER.lock().unwrap();
    *guard = Some(checker);
}

/// Check if plan mode blocks this tool.
fn check_plan_mode_block(tool_name: &str, is_read_only: bool) -> bool {
    let guard = PLAN_MODE_CHECKER.lock().unwrap();
    if let Some(checker) = *guard {
        checker(tool_name, is_read_only)
    } else {
        false
    }
}

/// Synchronous permission evaluation (no user prompting).
/// Returns Ask when user interaction is needed.
pub fn evaluate_permission_sync(
    tool_name: &str,
    input: &Value,
    ctx: &ToolPermissionContext,
    is_read_only: bool,
) -> PermissionDecision {
    // 0. Plan mode check -- when plan mode is active, write tools require Ask
    if check_plan_mode_block(tool_name, is_read_only) {
        return PermissionDecision::Ask {
            message: format!(
                "Tool '{}' is blocked in plan mode. Only read-only exploration is allowed. \
                 Use ExitPlanMode to present your plan and resume execution.",
                tool_name
            ),
        };
    }

    // 1. Check deny rules -> Deny
    if matches_any_rule(tool_name, input, &ctx.deny_rules) {
        return PermissionDecision::Deny {
            message: format!("Tool '{}' is denied by a deny rule.", tool_name),
        };
    }

    // 2. Check allow rules -> Allow
    if matches_any_rule(tool_name, input, &ctx.allow_rules) {
        return PermissionDecision::Allow;
    }

    // 3. Check ask rules -> Ask
    if matches_any_rule(tool_name, input, &ctx.ask_rules) {
        return PermissionDecision::Ask {
            message: format!("Tool '{}' requires user confirmation.", tool_name),
        };
    }

    // 4. Mode default: Bypass->Allow, Default->(readonly?Allow:Ask), Interactive->Ask
    match ctx.mode {
        PermissionMode::Bypass => PermissionDecision::Allow,
        PermissionMode::Default => {
            if is_read_only {
                PermissionDecision::Allow
            } else {
                PermissionDecision::Ask {
                    message: format!(
                        "Tool '{}' requires user confirmation (write operation).",
                        tool_name
                    ),
                }
            }
        }
        PermissionMode::InteractiveOnly => PermissionDecision::Ask {
            message: format!(
                "Tool '{}' requires user confirmation (interactive mode).",
                tool_name
            ),
        },
    }
}

fn matches_any_rule(
    tool_name: &str,
    _input: &Value,
    rules: &std::collections::HashMap<String, Vec<PermissionRule>>,
) -> bool {
    for rule_list in rules.values() {
        for rule in rule_list {
            if matches_rule(tool_name, rule) {
                return true;
            }
        }
    }
    false
}

fn matches_rule(tool_name: &str, rule: &PermissionRule) -> bool {
    // tool: "*" matches everything, otherwise exact match
    rule.tool == "*" || rule.tool == tool_name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ctx() -> ToolPermissionContext {
        ToolPermissionContext::default()
    }

    #[test]
    fn test_plan_mode_blocks_write_tools() {
        // Register a plan mode checker that always blocks write tools
        register_plan_mode_checker(|_name, is_read_only| !is_read_only);

        let ctx = empty_ctx();
        let decision = evaluate_permission_sync("Bash", &serde_json::json!({}), &ctx, false);
        assert!(matches!(decision, PermissionDecision::Ask { .. }));

        // Read-only tools should still be allowed
        let decision = evaluate_permission_sync("Read", &serde_json::json!({}), &ctx, true);
        assert!(matches!(decision, PermissionDecision::Allow));

        // Clean up
        register_plan_mode_checker(|_, _| false);
    }

    #[test]
    fn test_plan_mode_message_mentions_plan() {
        register_plan_mode_checker(|_, is_read_only| !is_read_only);

        let ctx = empty_ctx();
        let decision = evaluate_permission_sync("Edit", &serde_json::json!({}), &ctx, false);
        if let PermissionDecision::Ask { message } = decision {
            assert!(
                message.contains("plan mode"),
                "message should mention plan mode"
            );
        } else {
            panic!("expected Ask decision");
        }

        register_plan_mode_checker(|_, _| false);
    }

    #[test]
    fn test_no_plan_mode_checker_allows_through() {
        // When no checker is registered, plan mode check is a no-op
        let mut guard = PLAN_MODE_CHECKER.lock().unwrap();
        *guard = None;
        drop(guard);

        let ctx = empty_ctx();
        // Default mode with read-only should allow
        let decision = evaluate_permission_sync("Read", &serde_json::json!({}), &ctx, true);
        assert!(matches!(decision, PermissionDecision::Allow));
    }
}
