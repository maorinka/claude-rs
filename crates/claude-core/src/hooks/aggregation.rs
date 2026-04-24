use super::types::{AggregatedHookResult, HookResult, PermissionBehavior};
use tracing::debug;

// ============================================================================
// Hook result aggregation
// ============================================================================

/// Aggregate multiple individual hook results into a single result.
///
/// Aggregation rules (matching TypeScript):
///
/// **Permission behavior precedence**: deny > ask > allow > passthrough
///   - If any hook says "deny", the aggregate is "deny"
///   - If any hook says "ask" (and none say "deny"), aggregate is "ask"
///   - If any hook says "allow" (and none say "deny" or "ask"), aggregate is "allow"
///   - "passthrough" never overrides any other behavior
///
/// **Blocking errors**: Collected from all hooks.
///
/// **Additional contexts**: Collected from all hooks into a Vec (not deduplicated).
///
/// **prevent_continuation**: True if any hook requested it.
///
/// **stop_reason**: Last one wins (but typically only one hook sets it).
///
/// **updated_input**: Last "allow" or "ask" hook's updatedInput wins;
///   passthrough hooks also propagate their updatedInput separately.
///
/// **updated_mcp_tool_output**: Last one wins.
///
/// **permission_request_result**: Last one wins.
///
/// **retry**: True if any hook requested it.
///
/// **watch_paths**: Accumulated from all hooks.
pub fn aggregate_hook_results(results: Vec<HookResult>) -> AggregatedHookResult {
    let mut aggregated = AggregatedHookResult::default();
    let mut current_permission_behavior: Option<PermissionBehavior> = None;

    for result in &results {
        // Collect blocking errors
        if let Some(ref be) = result.blocking_error {
            aggregated.blocking_errors.push(be.clone());
        }

        // prevent_continuation: any true wins
        if result.prevent_continuation == Some(true) {
            aggregated.prevent_continuation = true;
            debug!(
                "Hook {} requested preventContinuation",
                result.command_display
            );
        }

        // stop_reason: last one wins
        if let Some(ref reason) = result.stop_reason {
            aggregated.stop_reason = Some(reason.clone());
        }

        // Collect additional contexts
        if let Some(ref ctx) = result.additional_context {
            aggregated.additional_contexts.push(ctx.clone());
        }

        // initial_user_message: last one wins
        if let Some(ref msg) = result.initial_user_message {
            aggregated.initial_user_message = Some(msg.clone());
        }

        // watch_paths: accumulate
        if let Some(ref paths) = result.watch_paths {
            aggregated.watch_paths.extend(paths.iter().cloned());
        }

        // updated_mcp_tool_output: last one wins
        if let Some(ref output) = result.updated_mcp_tool_output {
            aggregated.updated_mcp_tool_output = Some(output.clone());
        }

        // Permission behavior with precedence: deny > ask > allow > passthrough
        if let Some(ref behavior) = result.permission_behavior {
            debug!(
                "Hook {} returned permissionDecision: {:?}{}",
                result.command_display,
                behavior,
                result
                    .hook_permission_decision_reason
                    .as_ref()
                    .map(|r| format!(" (reason: {})", r))
                    .unwrap_or_default()
            );

            let should_update = match behavior {
                PermissionBehavior::Deny => {
                    // deny always takes precedence
                    true
                }
                PermissionBehavior::Ask => {
                    // ask takes precedence over allow but not deny
                    current_permission_behavior
                        .as_ref()
                        .map(|c| *c != PermissionBehavior::Deny)
                        .unwrap_or(true)
                }
                PermissionBehavior::Allow => {
                    // allow only if no other behavior set
                    current_permission_behavior.is_none()
                }
                PermissionBehavior::Passthrough => {
                    // passthrough doesn't set permission behavior
                    false
                }
            };

            if should_update {
                current_permission_behavior = Some(behavior.clone());
                aggregated.permission_behavior = Some(behavior.clone());
                aggregated.hook_permission_decision_reason =
                    result.hook_permission_decision_reason.clone();
            }

            // updatedInput: propagate from allow or ask decisions
            if (*behavior == PermissionBehavior::Allow || *behavior == PermissionBehavior::Ask)
                && result.updated_input.is_some()
            {
                aggregated.updated_input = result.updated_input.clone();
                debug!(
                    "Hook {} modified tool input keys: [{}]",
                    result.command_display,
                    result
                        .updated_input
                        .as_ref()
                        .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_default()
                );
            }
        }

        // updatedInput for passthrough case (no permission decision):
        // hooks can modify input without making a permission decision
        if result.updated_input.is_some() && result.permission_behavior.is_none() {
            aggregated.updated_input = result.updated_input.clone();
            debug!(
                "Hook {} (passthrough) modified tool input keys: [{}]",
                result.command_display,
                result
                    .updated_input
                    .as_ref()
                    .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default()
            );
        }

        // permission_request_result: last one wins
        if let Some(ref prr) = result.permission_request_result {
            aggregated.permission_request_result = Some(prr.clone());
        }

        // retry: any true wins
        if result.retry == Some(true) {
            aggregated.retry = Some(true);
        }
    }

    aggregated.individual_results = results;
    aggregated
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::types::{HookBlockingError, HookOutcome};

    fn make_result(behavior: Option<PermissionBehavior>) -> HookResult {
        HookResult {
            permission_behavior: behavior,
            ..Default::default()
        }
    }

    #[test]
    fn test_deny_overrides_allow() {
        let results = vec![
            make_result(Some(PermissionBehavior::Allow)),
            make_result(Some(PermissionBehavior::Deny)),
        ];
        let agg = aggregate_hook_results(results);
        assert_eq!(agg.permission_behavior, Some(PermissionBehavior::Deny));
    }

    #[test]
    fn test_deny_overrides_ask() {
        let results = vec![
            make_result(Some(PermissionBehavior::Ask)),
            make_result(Some(PermissionBehavior::Deny)),
        ];
        let agg = aggregate_hook_results(results);
        assert_eq!(agg.permission_behavior, Some(PermissionBehavior::Deny));
    }

    #[test]
    fn test_ask_overrides_allow() {
        let results = vec![
            make_result(Some(PermissionBehavior::Allow)),
            make_result(Some(PermissionBehavior::Ask)),
        ];
        let agg = aggregate_hook_results(results);
        assert_eq!(agg.permission_behavior, Some(PermissionBehavior::Ask));
    }

    #[test]
    fn test_allow_does_not_override_ask() {
        let results = vec![
            make_result(Some(PermissionBehavior::Ask)),
            make_result(Some(PermissionBehavior::Allow)),
        ];
        let agg = aggregate_hook_results(results);
        assert_eq!(agg.permission_behavior, Some(PermissionBehavior::Ask));
    }

    #[test]
    fn test_passthrough_does_not_set_behavior() {
        let results = vec![make_result(Some(PermissionBehavior::Passthrough))];
        let agg = aggregate_hook_results(results);
        assert_eq!(agg.permission_behavior, None);
    }

    #[test]
    fn test_blocking_errors_collected() {
        let results = vec![
            HookResult {
                outcome: HookOutcome::Blocking,
                blocking_error: Some(HookBlockingError {
                    blocking_error: "error 1".to_string(),
                    command: "cmd1".to_string(),
                }),
                ..Default::default()
            },
            HookResult {
                outcome: HookOutcome::Blocking,
                blocking_error: Some(HookBlockingError {
                    blocking_error: "error 2".to_string(),
                    command: "cmd2".to_string(),
                }),
                ..Default::default()
            },
        ];
        let agg = aggregate_hook_results(results);
        assert_eq!(agg.blocking_errors.len(), 2);
    }

    #[test]
    fn test_additional_contexts_collected() {
        let results = vec![
            HookResult {
                additional_context: Some("ctx1".to_string()),
                ..Default::default()
            },
            HookResult {
                additional_context: Some("ctx2".to_string()),
                ..Default::default()
            },
        ];
        let agg = aggregate_hook_results(results);
        assert_eq!(agg.additional_contexts, vec!["ctx1", "ctx2"]);
    }

    #[test]
    fn test_prevent_continuation_any_true() {
        let results = vec![
            HookResult {
                prevent_continuation: Some(false),
                ..Default::default()
            },
            HookResult {
                prevent_continuation: Some(true),
                stop_reason: Some("stopped".to_string()),
                ..Default::default()
            },
        ];
        let agg = aggregate_hook_results(results);
        assert!(agg.prevent_continuation);
        assert_eq!(agg.stop_reason, Some("stopped".to_string()));
    }

    #[test]
    fn test_empty_results() {
        let agg = aggregate_hook_results(vec![]);
        assert!(!agg.has_blocking_errors());
        assert!(!agg.prevent_continuation);
        assert!(agg.permission_behavior.is_none());
        assert!(agg.additional_contexts.is_empty());
    }
}
