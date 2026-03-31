use serde_json::Value;

use super::types::{PermissionDecision, PermissionMode, PermissionRule, ToolPermissionContext};

/// Synchronous permission evaluation (no user prompting).
/// Returns Ask when user interaction is needed.
pub fn evaluate_permission_sync(
    tool_name: &str,
    input: &Value,
    ctx: &ToolPermissionContext,
    is_read_only: bool,
) -> PermissionDecision {
    // 1. Check deny rules → Deny
    if matches_any_rule(tool_name, input, &ctx.deny_rules) {
        return PermissionDecision::Deny {
            message: format!("Tool '{}' is denied by a deny rule.", tool_name),
        };
    }

    // 2. Check allow rules → Allow
    if matches_any_rule(tool_name, input, &ctx.allow_rules) {
        return PermissionDecision::Allow;
    }

    // 3. Check ask rules → Ask
    if matches_any_rule(tool_name, input, &ctx.ask_rules) {
        return PermissionDecision::Ask {
            message: format!("Tool '{}' requires user confirmation.", tool_name),
        };
    }

    // 4. Mode default: Bypass→Allow, Default→(readonly?Allow:Ask), Interactive→Ask
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
            message: format!("Tool '{}' requires user confirmation (interactive mode).", tool_name),
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
    // pattern matching: just return true for now if tool matches (full glob in later phase)
    rule.tool == "*" || rule.tool == tool_name
}
