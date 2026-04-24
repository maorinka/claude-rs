use std::collections::HashMap;

use tracing::debug;

use super::runner::{get_pre_tool_hook_blocking_message, HookRunner};
use super::types::*;

// ============================================================================
// PreToolUse hook results
// ============================================================================

/// The result of running PreToolUse hooks, ready for the tool execution pipeline.
#[derive(Clone, Debug)]
pub struct PreToolUseHookDecision {
    /// If set, the hooks expressed a permission behavior.
    pub permission_behavior: Option<PermissionBehavior>,

    /// Human-readable reason for the permission decision.
    pub permission_reason: Option<String>,

    /// If a hook denied the tool, this contains the formatted denial message.
    pub denial_message: Option<String>,

    /// Modified tool input from hooks (if any).
    pub updated_input: Option<HashMap<String, serde_json::Value>>,

    /// Whether any hook asked to prevent continuation.
    pub prevent_continuation: bool,

    /// Stop reason if a hook asked to stop.
    pub stop_reason: Option<String>,

    /// Additional contexts to inject into the conversation.
    pub additional_contexts: Vec<String>,

    /// Blocking errors from hooks.
    pub blocking_errors: Vec<HookBlockingError>,

    /// Source of the hook that determined the permission behavior.
    pub hook_source: Option<String>,
}

/// Run PreToolUse hooks and interpret the results.
///
/// This mirrors the TypeScript `runPreToolUseHooks` generator function.
/// It calls `executePreToolHooks` internally and translates the aggregated
/// result into a `PreToolUseHookDecision`.
pub async fn run_pre_tool_use_hooks(
    runner: &HookRunner,
    tool_name: &str,
    tool_use_id: &str,
    tool_input: &serde_json::Value,
    permission_mode: Option<&str>,
    agent_id: Option<&str>,
    agent_type: Option<&str>,
) -> PreToolUseHookDecision {
    let extra_fields = serde_json::json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
        "tool_use_id": tool_use_id,
    });

    let aggregated = runner
        .run_hooks(
            &HookEvent::PreToolUse,
            extra_fields,
            permission_mode,
            agent_id,
            agent_type,
            None,
        )
        .await;

    let hook_name = format!("PreToolUse:{}", tool_name);

    // Translate blocking errors into denial messages
    let denial_message = if !aggregated.blocking_errors.is_empty() {
        Some(get_pre_tool_hook_blocking_message(
            &hook_name,
            &aggregated.blocking_errors[0],
        ))
    } else {
        None
    };

    // When a hook sets permissionBehavior, translate it into the decision.
    // The aggregation already applied precedence (deny > ask > allow).
    let permission_behavior = aggregated.permission_behavior.clone();
    let permission_reason = aggregated.hook_permission_decision_reason.clone();

    // If the hook denied via permission_behavior, format a denial message.
    let denial_message =
        if permission_behavior == Some(PermissionBehavior::Deny) && denial_message.is_none() {
            Some(
                permission_reason
                    .as_deref()
                    .map(|r| format!("Hook {} denied this tool: {}", hook_name, r))
                    .unwrap_or_else(|| format!("Hook {} denied this tool", hook_name)),
            )
        } else {
            denial_message
        };

    PreToolUseHookDecision {
        permission_behavior,
        permission_reason,
        denial_message,
        updated_input: aggregated.updated_input,
        prevent_continuation: aggregated.prevent_continuation,
        stop_reason: aggregated.stop_reason,
        additional_contexts: aggregated.additional_contexts,
        blocking_errors: aggregated.blocking_errors,
        hook_source: aggregated.hook_source,
    }
}

// ============================================================================
// PostToolUse hook results
// ============================================================================

/// The result of running PostToolUse hooks.
#[derive(Clone, Debug)]
pub struct PostToolUseHookDecision {
    /// Blocking errors from hooks.
    pub blocking_errors: Vec<HookBlockingError>,

    /// Whether any hook asked to prevent continuation.
    pub prevent_continuation: bool,

    /// Stop reason if a hook asked to stop.
    pub stop_reason: Option<String>,

    /// Additional contexts to inject into the conversation.
    pub additional_contexts: Vec<String>,

    /// Modified MCP tool output (if any).
    pub updated_mcp_tool_output: Option<serde_json::Value>,
}

/// Run PostToolUse hooks.
///
/// Mirrors the TypeScript `runPostToolUseHooks` generator function.
pub async fn run_post_tool_use_hooks(
    runner: &HookRunner,
    tool_name: &str,
    tool_use_id: &str,
    tool_input: &serde_json::Value,
    tool_response: &serde_json::Value,
    permission_mode: Option<&str>,
    agent_id: Option<&str>,
    agent_type: Option<&str>,
) -> PostToolUseHookDecision {
    let extra_fields = serde_json::json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
        "tool_response": tool_response,
        "tool_use_id": tool_use_id,
    });

    let aggregated = runner
        .run_hooks(
            &HookEvent::PostToolUse,
            extra_fields,
            permission_mode,
            agent_id,
            agent_type,
            None,
        )
        .await;

    PostToolUseHookDecision {
        blocking_errors: aggregated.blocking_errors,
        prevent_continuation: aggregated.prevent_continuation,
        stop_reason: aggregated.stop_reason,
        additional_contexts: aggregated.additional_contexts,
        updated_mcp_tool_output: aggregated.updated_mcp_tool_output,
    }
}

// ============================================================================
// PostToolUseFailure hook results
// ============================================================================

/// The result of running PostToolUseFailure hooks.
#[derive(Clone, Debug)]
pub struct PostToolUseFailureHookDecision {
    /// Blocking errors from hooks.
    pub blocking_errors: Vec<HookBlockingError>,

    /// Additional contexts to inject into the conversation.
    pub additional_contexts: Vec<String>,
}

/// Run PostToolUseFailure hooks.
///
/// Mirrors the TypeScript `executePostToolUseFailureHooks`.
#[allow(clippy::too_many_arguments)]
pub async fn run_post_tool_use_failure_hooks(
    runner: &HookRunner,
    tool_name: &str,
    tool_use_id: &str,
    tool_input: &serde_json::Value,
    error: &str,
    is_interrupt: Option<bool>,
    permission_mode: Option<&str>,
    agent_id: Option<&str>,
    agent_type: Option<&str>,
) -> PostToolUseFailureHookDecision {
    let extra_fields = serde_json::json!({
        "tool_name": tool_name,
        "tool_input": tool_input,
        "tool_use_id": tool_use_id,
        "error": error,
        "is_interrupt": is_interrupt,
    });

    let aggregated = runner
        .run_hooks(
            &HookEvent::PostToolUseFailure,
            extra_fields,
            permission_mode,
            agent_id,
            agent_type,
            None,
        )
        .await;

    PostToolUseFailureHookDecision {
        blocking_errors: aggregated.blocking_errors,
        additional_contexts: aggregated.additional_contexts,
    }
}

// ============================================================================
// Permission decision resolution
// ============================================================================

/// The behavior for a permission rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleCheckResult {
    /// No rule matched — defer to the hook/default.
    NoMatch,
    /// A deny rule matched, with an optional message.
    Deny(Option<String>),
    /// An ask rule matched — requires user interaction.
    Ask,
}

/// Resolve a PreToolUse hook's permission result into a final decision.
///
/// Encapsulates the invariant that hook 'allow' does NOT bypass deny/ask rules.
/// This mirrors the TypeScript `resolveHookPermissionDecision` function.
///
/// The `check_rule_permissions` callback is called to evaluate deny/ask rules
/// from settings.json against the (potentially updated) tool input.
///
/// Rules:
/// 1. Hook "allow" + deny rule => deny wins (hook allow does NOT bypass deny rules)
/// 2. Hook "allow" + ask rule  => ask wins (user must still confirm)
/// 3. Hook "allow" + no rules  => allow (hook provides the approval)
/// 4. Hook "deny"              => deny immediately
/// 5. Hook "ask"               => force the ask dialog, pass through forceDecision
/// 6. No hook / passthrough    => normal permission flow
pub async fn resolve_hook_permission_decision<F, Fut>(
    hook_decision: &PreToolUseHookDecision,
    tool_input: &serde_json::Value,
    check_rule_permissions: F,
) -> ResolvedPermission
where
    F: FnOnce(&serde_json::Value) -> Fut,
    Fut: std::future::Future<Output = RuleCheckResult>,
{
    // If the hook explicitly allowed the tool
    if hook_decision.permission_behavior == Some(PermissionBehavior::Allow) {
        let hook_input = if let Some(ref updated) = hook_decision.updated_input {
            // Hook provided updatedInput — merge with original
            let mut merged = tool_input.clone();
            if let Some(obj) = merged.as_object_mut() {
                for (k, v) in updated {
                    obj.insert(k.clone(), v.clone());
                }
            }
            merged
        } else {
            tool_input.clone()
        };

        // Hook allow does NOT bypass deny/ask rules — check them.
        let rule_check = check_rule_permissions(&hook_input).await;
        match rule_check {
            RuleCheckResult::NoMatch => {
                debug!("Hook approved tool use, bypassing permission prompt");
                return ResolvedPermission::Allow {
                    updated_input: hook_decision.updated_input.clone(),
                };
            },
            RuleCheckResult::Deny(msg) => {
                debug!("Hook approved tool use, but deny rule overrides: {:?}", msg);
                return ResolvedPermission::Deny { message: msg };
            },
            RuleCheckResult::Ask => {
                debug!("Hook approved tool use, but ask rule requires prompt");
                return ResolvedPermission::RequiresUserConfirmation {
                    updated_input: hook_decision.updated_input.clone(),
                    force_decision: None,
                };
            },
        }
    }

    // Hook denied
    if hook_decision.permission_behavior == Some(PermissionBehavior::Deny) {
        debug!("Hook denied tool use");
        return ResolvedPermission::Deny {
            message: hook_decision.denial_message.clone(),
        };
    }

    // Hook asked — force the ask dialog
    if hook_decision.permission_behavior == Some(PermissionBehavior::Ask) {
        return ResolvedPermission::RequiresUserConfirmation {
            updated_input: hook_decision.updated_input.clone(),
            force_decision: hook_decision.permission_reason.clone(),
        };
    }

    // No hook decision or passthrough — normal permission flow
    ResolvedPermission::NormalFlow {
        updated_input: hook_decision.updated_input.clone(),
    }
}

/// The resolved permission decision after combining hook results with rules.
#[derive(Clone, Debug)]
pub enum ResolvedPermission {
    /// Tool is allowed (hook approved and no deny/ask rules override).
    Allow {
        updated_input: Option<HashMap<String, serde_json::Value>>,
    },
    /// Tool is denied (either by hook or by a deny rule overriding hook allow).
    Deny { message: Option<String> },
    /// User confirmation is required (ask rule or hook ask).
    RequiresUserConfirmation {
        updated_input: Option<HashMap<String, serde_json::Value>>,
        /// If set, this is the hook's ask message to show in the dialog.
        force_decision: Option<String>,
    },
    /// No hook opinion — proceed with normal permission flow.
    NormalFlow {
        updated_input: Option<HashMap<String, serde_json::Value>>,
    },
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hook_decision(behavior: Option<PermissionBehavior>) -> PreToolUseHookDecision {
        PreToolUseHookDecision {
            permission_behavior: behavior,
            permission_reason: None,
            denial_message: None,
            updated_input: None,
            prevent_continuation: false,
            stop_reason: None,
            additional_contexts: vec![],
            blocking_errors: vec![],
            hook_source: None,
        }
    }

    #[tokio::test]
    async fn test_resolve_allow_no_rules() {
        let decision = make_hook_decision(Some(PermissionBehavior::Allow));
        let input = serde_json::json!({"command": "echo hello"});

        let result = resolve_hook_permission_decision(&decision, &input, |_| async {
            RuleCheckResult::NoMatch
        })
        .await;

        assert!(matches!(result, ResolvedPermission::Allow { .. }));
    }

    #[tokio::test]
    async fn test_resolve_allow_deny_rule_overrides() {
        let decision = make_hook_decision(Some(PermissionBehavior::Allow));
        let input = serde_json::json!({"command": "rm -rf /"});

        let result = resolve_hook_permission_decision(&decision, &input, |_| async {
            RuleCheckResult::Deny(Some("dangerous command".to_string()))
        })
        .await;

        match result {
            ResolvedPermission::Deny { message } => {
                assert_eq!(message, Some("dangerous command".to_string()));
            },
            _ => panic!("Expected Deny, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_resolve_allow_ask_rule_overrides() {
        let decision = make_hook_decision(Some(PermissionBehavior::Allow));
        let input = serde_json::json!({});

        let result =
            resolve_hook_permission_decision(&decision, &input, |_| async { RuleCheckResult::Ask })
                .await;

        assert!(matches!(
            result,
            ResolvedPermission::RequiresUserConfirmation { .. }
        ));
    }

    #[tokio::test]
    async fn test_resolve_deny() {
        let mut decision = make_hook_decision(Some(PermissionBehavior::Deny));
        decision.denial_message = Some("nope".to_string());
        let input = serde_json::json!({});

        let result = resolve_hook_permission_decision(&decision, &input, |_| async {
            RuleCheckResult::NoMatch
        })
        .await;

        match result {
            ResolvedPermission::Deny { message } => {
                assert_eq!(message, Some("nope".to_string()));
            },
            _ => panic!("Expected Deny"),
        }
    }

    #[tokio::test]
    async fn test_resolve_ask() {
        let mut decision = make_hook_decision(Some(PermissionBehavior::Ask));
        decision.permission_reason = Some("please confirm".to_string());
        let input = serde_json::json!({});

        let result = resolve_hook_permission_decision(&decision, &input, |_| async {
            RuleCheckResult::NoMatch
        })
        .await;

        match result {
            ResolvedPermission::RequiresUserConfirmation { force_decision, .. } => {
                assert_eq!(force_decision, Some("please confirm".to_string()));
            },
            _ => panic!("Expected RequiresUserConfirmation"),
        }
    }

    #[tokio::test]
    async fn test_resolve_no_hook() {
        let decision = make_hook_decision(None);
        let input = serde_json::json!({});

        let result = resolve_hook_permission_decision(&decision, &input, |_| async {
            RuleCheckResult::NoMatch
        })
        .await;

        assert!(matches!(result, ResolvedPermission::NormalFlow { .. }));
    }

    #[tokio::test]
    async fn test_resolve_passthrough() {
        let decision = make_hook_decision(Some(PermissionBehavior::Passthrough));
        let input = serde_json::json!({});

        let result = resolve_hook_permission_decision(&decision, &input, |_| async {
            RuleCheckResult::NoMatch
        })
        .await;

        assert!(matches!(result, ResolvedPermission::NormalFlow { .. }));
    }
}
