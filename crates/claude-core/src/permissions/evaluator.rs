//! The complete 5-step permission resolution pipeline.
//!
//! This implements the exact algorithm from the TypeScript `hasPermissionsToUseTool` function:
//!
//! 1. **Rule checks** (deny -> ask -> tool checkPermissions -> content rules -> safety checks)
//! 2. **Mode override** (bypassPermissions -> allow rules)
//! 3. **Transform** (passthrough -> ask)
//! 4. **Mode apply** (dontAsk -> deny, auto -> classifier, headless -> hooks)
//! 5. **Return**

use serde_json::Value;

use super::filesystem;
use super::types::{
    PermissionAllowDecision, PermissionAskDecision, PermissionBehavior, PermissionDecision,
    PermissionDecisionReason, PermissionMode, PermissionResult, PermissionRule,
    PermissionRuleSource, PermissionRuleValue, PermissionUpdate, ToolPermissionContext,
};

// ============================================================================
// Tool trait for permission checking
// ============================================================================

/// Trait that tools implement to participate in the permission pipeline.
/// This is the Rust equivalent of the TS `Tool` interface's permission-related methods.
pub trait ToolPermissions {
    /// The tool's name (e.g., "Bash", "Edit", "Read").
    fn name(&self) -> &str;

    /// Check tool-specific permissions for the given input.
    /// Returns a PermissionResult (which can be Passthrough for "no opinion").
    fn check_permissions(&self, input: &Value, context: &ToolPermissionContext)
        -> PermissionResult;

    /// Whether this tool requires user interaction even in bypass mode.
    fn requires_user_interaction(&self) -> bool {
        false
    }

    /// Whether this tool is read-only.
    fn is_read_only(&self) -> bool {
        false
    }

    /// Get the file path this tool operates on, if applicable.
    fn get_path(&self, input: &Value) -> Option<String> {
        let _ = input;
        None
    }

    /// MCP info for the tool, if it's an MCP tool.
    fn mcp_info(&self) -> Option<McpToolInfo> {
        None
    }
}

/// Simple tool permissions implementation for basic permission checks.
/// Use when you only have a tool name and read-only flag.
pub struct SimpleToolPermissions {
    name: String,
    read_only: bool,
}

impl SimpleToolPermissions {
    pub fn new(name: &str, read_only: bool) -> Self {
        Self {
            name: name.to_string(),
            read_only,
        }
    }
}

impl ToolPermissions for SimpleToolPermissions {
    fn name(&self) -> &str {
        &self.name
    }

    fn check_permissions(
        &self,
        _input: &Value,
        _context: &ToolPermissionContext,
    ) -> PermissionResult {
        PermissionResult::passthrough("")
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }
}

/// MCP tool identification info.
#[derive(Clone, Debug)]
pub struct McpToolInfo {
    pub server_name: String,
    pub tool_name: Option<String>,
}

/// Get the canonical name used for permission rule matching.
/// For MCP tools, uses the fully qualified mcp__server__tool name.
pub fn get_tool_name_for_permission_check(tool: &dyn ToolPermissions) -> String {
    if let Some(mcp) = tool.mcp_info() {
        match mcp.tool_name {
            Some(ref t) => format!("mcp__{}__{}", mcp.server_name, t),
            None => format!("mcp__{}", mcp.server_name),
        }
    } else {
        tool.name().to_string()
    }
}

/// Parse MCP info from a string like "mcp__server__tool".
pub fn mcp_info_from_string(name: &str) -> Option<McpToolInfo> {
    if !name.starts_with("mcp__") {
        return None;
    }
    let rest = &name[5..];
    if let Some(idx) = rest.find("__") {
        let server_name = rest[..idx].to_string();
        let tool_name = rest[idx + 2..].to_string();
        Some(McpToolInfo {
            server_name,
            tool_name: if tool_name.is_empty() {
                None
            } else {
                Some(tool_name)
            },
        })
    } else {
        Some(McpToolInfo {
            server_name: rest.to_string(),
            tool_name: None,
        })
    }
}

// ============================================================================
// External hook for plan mode checking
// ============================================================================

/// Returns true if the tool should be blocked (requires Ask) due to plan mode.
pub type PlanModeChecker = fn(tool_name: &str, is_read_only: bool) -> bool;

static PLAN_MODE_CHECKER: std::sync::OnceLock<PlanModeChecker> = std::sync::OnceLock::new();

/// Register the plan mode checker. Called during startup.
pub fn register_plan_mode_checker(checker: PlanModeChecker) {
    let _ = PLAN_MODE_CHECKER.set(checker);
}

/// Check if plan mode blocks this tool.
fn check_plan_mode_block(tool_name: &str, is_read_only: bool) -> bool {
    if let Some(checker) = PLAN_MODE_CHECKER.get() {
        checker(tool_name, is_read_only)
    } else {
        false
    }
}

// ============================================================================
// Rule Gathering
// ============================================================================

/// Get all allow rules from all sources as structured PermissionRule objects.
pub fn get_allow_rules(context: &ToolPermissionContext) -> Vec<PermissionRule> {
    gather_rules(context, &PermissionBehavior::Allow)
}

/// Get all deny rules from all sources.
pub fn get_deny_rules(context: &ToolPermissionContext) -> Vec<PermissionRule> {
    gather_rules(context, &PermissionBehavior::Deny)
}

/// Get all ask rules from all sources.
pub fn get_ask_rules(context: &ToolPermissionContext) -> Vec<PermissionRule> {
    gather_rules(context, &PermissionBehavior::Ask)
}

fn gather_rules(
    context: &ToolPermissionContext,
    behavior: &PermissionBehavior,
) -> Vec<PermissionRule> {
    let rules_by_source = context.rules_for_behavior(behavior);
    let mut result = Vec::new();

    for source in PermissionRuleSource::all_sources() {
        if let Some(rule_strings) = rules_by_source.get(source) {
            for rule_string in rule_strings {
                let rule_value = PermissionRuleValue::from_string(rule_string);
                result.push(PermissionRule {
                    source: source.clone(),
                    rule_behavior: behavior.clone(),
                    rule_value,
                });
            }
        }
    }

    result
}

// ============================================================================
// Tool-Level Rule Matching
// ============================================================================

/// Check if an entire tool matches a rule (not content-specific).
/// For example, "Bash" matches but "Bash(prefix:*)" does not.
/// Also handles MCP server-level matching.
fn tool_matches_rule(tool: &dyn ToolPermissions, rule: &PermissionRule) -> bool {
    // Rule must not have content to match the entire tool
    if rule.rule_value.rule_content.is_some() {
        return false;
    }

    let name_for_match = get_tool_name_for_permission_check(tool);

    // Direct tool name match
    if rule.rule_value.tool_name == name_for_match {
        return true;
    }

    // MCP server-level: rule "mcp__server1" matches tool "mcp__server1__tool1"
    let rule_mcp = mcp_info_from_string(&rule.rule_value.tool_name);
    let tool_mcp = mcp_info_from_string(&name_for_match);

    if let (Some(rule_info), Some(tool_info)) = (rule_mcp, tool_mcp) {
        return (rule_info.tool_name.is_none() || rule_info.tool_name.as_deref() == Some("*"))
            && rule_info.server_name == tool_info.server_name;
    }

    false
}

/// Check if the entire tool is in the always-allow rules.
pub fn tool_always_allowed_rule(
    context: &ToolPermissionContext,
    tool: &dyn ToolPermissions,
) -> Option<PermissionRule> {
    get_allow_rules(context)
        .into_iter()
        .find(|rule| tool_matches_rule(tool, rule))
}

/// Check if the entire tool is in the deny rules.
pub fn get_deny_rule_for_tool(
    context: &ToolPermissionContext,
    tool: &dyn ToolPermissions,
) -> Option<PermissionRule> {
    get_deny_rules(context)
        .into_iter()
        .find(|rule| tool_matches_rule(tool, rule))
}

/// Check if the entire tool is in the ask rules.
pub fn get_ask_rule_for_tool(
    context: &ToolPermissionContext,
    tool: &dyn ToolPermissions,
) -> Option<PermissionRule> {
    get_ask_rules(context)
        .into_iter()
        .find(|rule| tool_matches_rule(tool, rule))
}

/// Check if a specific agent type is denied via Agent(agentType) syntax.
pub fn get_deny_rule_for_agent(
    context: &ToolPermissionContext,
    agent_tool_name: &str,
    agent_type: &str,
) -> Option<PermissionRule> {
    get_deny_rules(context).into_iter().find(|rule| {
        rule.rule_value.tool_name == agent_tool_name
            && rule.rule_value.rule_content.as_deref() == Some(agent_type)
    })
}

/// Get the content-specific rules for a tool, mapped by their content string.
pub fn get_rule_by_contents_for_tool(
    context: &ToolPermissionContext,
    tool: &dyn ToolPermissions,
    behavior: &PermissionBehavior,
) -> std::collections::HashMap<String, PermissionRule> {
    let tool_name = get_tool_name_for_permission_check(tool);
    filesystem::get_rule_by_contents_for_tool_name(context, &tool_name, behavior)
}

// ============================================================================
// The Main Permission Pipeline
// ============================================================================

/// The 5-step synchronous permission evaluation.
///
/// This is the core algorithm from TS `hasPermissionsToUseToolInner`, adapted for
/// synchronous evaluation. The async parts (classifier, hooks) return Ask decisions
/// that the caller can handle asynchronously.
///
/// ## Pipeline Steps:
/// 1. Rule checks: deny -> ask -> tool checkPermissions -> content rules -> safety checks
/// 2. Mode override: bypassPermissions -> allow rules
/// 3. Transform passthrough -> ask
/// 4. Mode apply: dontAsk -> deny, auto -> ask (for async classifier), headless -> deny
/// 5. Return
pub fn evaluate_permission(
    tool: &dyn ToolPermissions,
    input: &Value,
    context: &ToolPermissionContext,
) -> PermissionDecision {
    let tool_name = tool.name();
    let is_read_only = tool.is_read_only();

    // Step 0: Plan mode check
    if check_plan_mode_block(tool_name, is_read_only) {
        return PermissionDecision::Ask(PermissionAskDecision {
            message: format!(
                "Tool '{}' is blocked in plan mode. Only read-only exploration is allowed. \
                 Use ExitPlanMode to present your plan and resume execution.",
                tool_name
            ),
            updated_input: None,
            decision_reason: Some(PermissionDecisionReason::Mode {
                mode: PermissionMode::Plan,
            }),
            suggestions: None,
            blocked_path: None,
            is_bash_security_check_for_misparsing: None,
        });
    }

    // ---------------------------------------------------------------
    // Step 1: Rule-based checks
    // ---------------------------------------------------------------

    // 1a. Entire tool is denied by rule
    if let Some(deny_rule) = get_deny_rule_for_tool(context, tool) {
        return PermissionDecision::deny(
            format!("Permission to use {} has been denied.", tool_name),
            PermissionDecisionReason::Rule { rule: deny_rule },
        );
    }

    // 1b. Entire tool has an ask rule
    if let Some(ask_rule) = get_ask_rule_for_tool(context, tool) {
        return PermissionDecision::ask_with_reason(
            super::types::create_permission_request_message(tool_name, None),
            PermissionDecisionReason::Rule { rule: ask_rule },
        );
    }

    // 1c. Ask the tool implementation for a permission result
    let tool_permission_result = tool.check_permissions(input, context);

    // 1d. Tool implementation denied permission
    if tool_permission_result.is_deny() {
        return match tool_permission_result {
            PermissionResult::Deny(d) => PermissionDecision::Deny(d),
            _ => unreachable!(),
        };
    }

    // 1e. Tool requires user interaction even in bypass mode
    if tool.requires_user_interaction() {
        if let PermissionResult::Ask(ask) = &tool_permission_result {
            return PermissionDecision::Ask(ask.clone());
        }
    }

    // 1f. Content-specific ask rules from tool.checkPermissions
    // (e.g. Bash(npm publish:*) -> {ask, type:'rule', ruleBehavior:'ask'})
    if let PermissionResult::Ask(ref ask) = tool_permission_result {
        if let Some(PermissionDecisionReason::Rule { ref rule }) = ask.decision_reason {
            if rule.rule_behavior == PermissionBehavior::Ask {
                return PermissionDecision::Ask(ask.clone());
            }
        }
    }

    // 1g. Safety checks (bypass-immune)
    if let PermissionResult::Ask(ref ask) = tool_permission_result {
        if let Some(PermissionDecisionReason::SafetyCheck { .. }) = &ask.decision_reason {
            return PermissionDecision::Ask(ask.clone());
        }
    }

    // ---------------------------------------------------------------
    // Step 2: Mode-based overrides
    // ---------------------------------------------------------------

    // 2a. BypassPermissions mode or plan mode with active auto mode.
    // Mirrors TS: plan mode bypasses only when auto mode is active, not merely
    // when bypassPermissions mode is feature-available.
    let should_bypass = context.mode == PermissionMode::BypassPermissions
        || (context.mode == PermissionMode::Plan
            && context.is_auto_mode_available.unwrap_or(false));

    if should_bypass {
        let updated_input = get_updated_input_or_fallback(&tool_permission_result, input);
        return PermissionDecision::Allow(PermissionAllowDecision {
            updated_input: Some(updated_input),
            user_modified: None,
            decision_reason: Some(PermissionDecisionReason::Mode {
                mode: context.mode.clone(),
            }),
            tool_use_id: None,
            accept_feedback: None,
        });
    }

    // 2b. Entire tool is always allowed
    if let Some(allow_rule) = tool_always_allowed_rule(context, tool) {
        let updated_input = get_updated_input_or_fallback(&tool_permission_result, input);
        return PermissionDecision::Allow(PermissionAllowDecision {
            updated_input: Some(updated_input),
            user_modified: None,
            decision_reason: Some(PermissionDecisionReason::Rule { rule: allow_rule }),
            tool_use_id: None,
            accept_feedback: None,
        });
    }

    // ---------------------------------------------------------------
    // Step 3: Convert passthrough to ask
    // ---------------------------------------------------------------
    let result: PermissionDecision = match tool_permission_result {
        PermissionResult::Passthrough {
            decision_reason,
            suggestions,
            ..
        } => {
            let msg = super::types::create_permission_request_message(
                tool_name,
                decision_reason.as_ref(),
            );
            PermissionDecision::Ask(PermissionAskDecision {
                message: msg,
                updated_input: None,
                decision_reason,
                suggestions,
                blocked_path: None,
                is_bash_security_check_for_misparsing: None,
            })
        }
        PermissionResult::Allow(d) => PermissionDecision::Allow(d),
        PermissionResult::Ask(d) => PermissionDecision::Ask(d),
        PermissionResult::Deny(d) => PermissionDecision::Deny(d),
    };

    // ---------------------------------------------------------------
    // Step 4: Mode-based transformations on Ask decisions
    // ---------------------------------------------------------------
    if result.is_ask() {
        // 4a. DontAsk mode: convert ask -> deny
        if context.mode == PermissionMode::DontAsk {
            return PermissionDecision::deny(
                format!(
                    "Permission denied: {} cannot run in don't-ask mode without explicit permission. \
                     Add a permission rule to allow this tool.",
                    tool_name
                ),
                PermissionDecisionReason::Mode {
                    mode: PermissionMode::DontAsk,
                },
            );
        }

        // 4b. Auto mode: return ask decision for caller to handle with classifier
        // The caller (async layer) will run the classifier and potentially convert
        // this to allow or deny.
        if context.mode == PermissionMode::Auto {
            // Non-classifier-approvable safety checks stay immune
            if let PermissionDecision::Ask(ref ask) = result {
                if let Some(PermissionDecisionReason::SafetyCheck {
                    classifier_approvable: false,
                    ..
                }) = &ask.decision_reason
                {
                    if context.should_avoid_permission_prompts {
                        return PermissionDecision::deny(
                            ask.message.clone(),
                            PermissionDecisionReason::AsyncAgent {
                                reason: "Safety check requires interactive approval and \
                                         permission prompts are not available in this context"
                                    .to_string(),
                            },
                        );
                    }
                    return result;
                }
            }
            // In synchronous mode, we return the ask decision as-is.
            // The async caller will feed this to the classifier.
            return result;
        }

        // 4c. Headless/shouldAvoidPermissionPrompts: auto-deny
        if context.should_avoid_permission_prompts {
            return PermissionDecision::deny(
                format!(
                    "Permission denied: {} was not allowed and permission prompts are not \
                     available in this context.",
                    tool_name
                ),
                PermissionDecisionReason::AsyncAgent {
                    reason: "Permission prompts are not available in this context".to_string(),
                },
            );
        }
    }

    // ---------------------------------------------------------------
    // Step 5: Return
    // ---------------------------------------------------------------
    result
}

/// Check only the rule-based steps of the pipeline (steps 1a-1g).
///
/// Returns a deny/ask decision if a rule blocks the tool, or None if no rule objects.
/// Unlike `evaluate_permission`, this does NOT run mode-based transformations,
/// the classifier, or hooks.
pub fn check_rule_based_permissions(
    tool: &dyn ToolPermissions,
    input: &Value,
    context: &ToolPermissionContext,
) -> Option<PermissionDecision> {
    // 1a. Entire tool is denied by rule
    if let Some(deny_rule) = get_deny_rule_for_tool(context, tool) {
        return Some(PermissionDecision::deny(
            format!("Permission to use {} has been denied.", tool.name()),
            PermissionDecisionReason::Rule { rule: deny_rule },
        ));
    }

    // 1b. Entire tool has an ask rule
    if let Some(ask_rule) = get_ask_rule_for_tool(context, tool) {
        return Some(PermissionDecision::ask_with_reason(
            super::types::create_permission_request_message(tool.name(), None),
            PermissionDecisionReason::Rule { rule: ask_rule },
        ));
    }

    // 1c. Tool-specific permission check
    let tool_result = tool.check_permissions(input, context);

    // 1d. Tool implementation denied
    if tool_result.is_deny() {
        return match tool_result {
            PermissionResult::Deny(d) => Some(PermissionDecision::Deny(d)),
            _ => None,
        };
    }

    // 1f. Content-specific ask rules
    if let PermissionResult::Ask(ref ask) = tool_result {
        if let Some(PermissionDecisionReason::Rule { ref rule }) = ask.decision_reason {
            if rule.rule_behavior == PermissionBehavior::Ask {
                return Some(PermissionDecision::Ask(ask.clone()));
            }
        }
    }

    // 1g. Safety checks (bypass-immune)
    if let PermissionResult::Ask(ref ask) = tool_result {
        if let Some(PermissionDecisionReason::SafetyCheck { .. }) = &ask.decision_reason {
            return Some(PermissionDecision::Ask(ask.clone()));
        }
    }

    // No rule-based objection
    None
}

/// Extract updatedInput from a permission result, falling back to the original input.
fn get_updated_input_or_fallback(result: &PermissionResult, fallback: &Value) -> Value {
    match result {
        PermissionResult::Allow(d) => d.updated_input.clone().unwrap_or_else(|| fallback.clone()),
        PermissionResult::Ask(d) => d.updated_input.clone().unwrap_or_else(|| fallback.clone()),
        _ => fallback.clone(),
    }
}

/// Apply permission rules to context (additive -- for initial setup).
/// Converts rules to AddRules updates and applies them.
pub fn apply_permission_rules_to_context(
    mut context: ToolPermissionContext,
    rules: &[PermissionRule],
) -> ToolPermissionContext {
    for rule in rules {
        let source = &rule.source;
        let rule_string = rule.rule_value.to_rule_string();

        let rules_map = context.rules_for_behavior_mut(&rule.rule_behavior);
        rules_map
            .entry(source.clone())
            .or_default()
            .push(rule_string);
    }
    context
}

/// Apply a single PermissionUpdate to a ToolPermissionContext.
pub fn apply_permission_update(
    mut context: ToolPermissionContext,
    update: &PermissionUpdate,
) -> ToolPermissionContext {
    match update {
        PermissionUpdate::AddRules {
            destination,
            rules,
            behavior,
        } => {
            let rules_map = context.rules_for_behavior_mut(behavior);
            let entry = rules_map.entry(destination.clone()).or_default();
            for rule in rules {
                let rule_string = rule.to_rule_string();
                if !entry.contains(&rule_string) {
                    entry.push(rule_string);
                }
            }
        }
        PermissionUpdate::ReplaceRules {
            destination,
            rules,
            behavior,
        } => {
            let rules_map = context.rules_for_behavior_mut(behavior);
            let new_rules: Vec<String> = rules.iter().map(|r| r.to_rule_string()).collect();
            rules_map.insert(destination.clone(), new_rules);
        }
        PermissionUpdate::RemoveRules {
            destination,
            rules,
            behavior,
        } => {
            let rules_map = context.rules_for_behavior_mut(behavior);
            if let Some(existing) = rules_map.get_mut(destination) {
                let to_remove: Vec<String> = rules.iter().map(|r| r.to_rule_string()).collect();
                existing.retain(|r| !to_remove.contains(r));
            }
        }
        PermissionUpdate::SetMode { mode, .. } => {
            context.mode = mode.clone();
        }
        PermissionUpdate::AddDirectories {
            destination,
            directories,
        } => {
            for dir in directories {
                context.additional_working_directories.insert(
                    dir.clone(),
                    super::types::AdditionalWorkingDirectory {
                        path: dir.clone(),
                        source: destination.clone(),
                    },
                );
            }
        }
        PermissionUpdate::RemoveDirectories { directories, .. } => {
            for dir in directories {
                context.additional_working_directories.remove(dir);
            }
        }
    }
    context
}

/// Apply multiple permission updates sequentially.
pub fn apply_permission_updates(
    mut context: ToolPermissionContext,
    updates: &[PermissionUpdate],
) -> ToolPermissionContext {
    for update in updates {
        context = apply_permission_update(context, update);
    }
    context
}

/// Sync permission rules from disk (replacement -- for settings changes).
/// Clears all disk-based sources before applying new rules to avoid stale entries.
pub fn sync_permission_rules_from_disk(
    mut context: ToolPermissionContext,
    rules: &[PermissionRule],
) -> ToolPermissionContext {
    // Clear all disk-based source:behavior combos
    let disk_sources = [
        PermissionRuleSource::UserSettings,
        PermissionRuleSource::ProjectSettings,
        PermissionRuleSource::LocalSettings,
        PermissionRuleSource::FlagSettings,
        PermissionRuleSource::PolicySettings,
    ];
    let behaviors = [
        PermissionBehavior::Allow,
        PermissionBehavior::Deny,
        PermissionBehavior::Ask,
    ];

    for source in &disk_sources {
        for behavior in &behaviors {
            let rules_map = context.rules_for_behavior_mut(behavior);
            rules_map.insert(source.clone(), Vec::new());
        }
    }

    // Re-add rules from disk
    apply_permission_rules_to_context(context, rules)
}

/// Async variant of [`evaluate_permission`] that runs the YOLO classifier
/// for `PermissionMode::Auto`. The synchronous evaluator returns Ask when
/// it's in Auto mode and a classifier-approvable rule fires; this wrapper
/// hands that Ask to [`crate::yolo_classifier::classify_action`] and
/// converts the verdict to Allow/Deny when the classifier is registered.
///
/// Falls back to the sync result unchanged when:
/// - the sync result is not Ask (no classifier needed)
/// - the mode is not Auto
/// - no secondary model is registered (classifier returns None)
/// - the classifier itself errors (callers should treat the Ask as the
///   safest fallback)
pub async fn evaluate_permission_async<T: ToolPermissions>(
    tool: &T,
    input: &Value,
    context: &ToolPermissionContext,
    recent_transcript: &str,
) -> PermissionDecision {
    use crate::yolo_classifier::classify_action;
    use tokio_util::sync::CancellationToken;

    let sync = evaluate_permission(tool, input, context);
    if context.mode != PermissionMode::Auto {
        return sync;
    }
    let PermissionDecision::Ask(ref ask) = sync else {
        return sync;
    };
    // Honor the safety-check immunity already enforced by the sync layer.
    if let Some(PermissionDecisionReason::SafetyCheck {
        classifier_approvable: false,
        ..
    }) = &ask.decision_reason
    {
        return sync;
    }

    let input_json = serde_json::to_string(input).unwrap_or_else(|_| input.to_string());
    let verdict = match classify_action(
        tool.name(),
        &input_json,
        recent_transcript,
        CancellationToken::new(),
    )
    .await
    {
        Ok(Some(v)) => v,
        // No classifier registered or call failed → leave the Ask in place.
        _ => return sync,
    };

    if verdict.should_block {
        PermissionDecision::deny(
            verdict.reason.clone(),
            PermissionDecisionReason::AsyncAgent {
                reason: format!("YOLO classifier blocked: {}", verdict.reason),
            },
        )
    } else {
        PermissionDecision::Allow(PermissionAllowDecision {
            updated_input: None,
            user_modified: None,
            decision_reason: Some(PermissionDecisionReason::AsyncAgent {
                reason: format!("YOLO classifier allowed: {}", verdict.reason),
            }),
            tool_use_id: None,
            accept_feedback: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple test tool for permission checking.
    struct TestTool {
        name: String,
        read_only: bool,
    }

    impl TestTool {
        fn new(name: &str, read_only: bool) -> Self {
            TestTool {
                name: name.to_string(),
                read_only,
            }
        }
    }

    impl ToolPermissions for TestTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn check_permissions(
            &self,
            _input: &Value,
            _context: &ToolPermissionContext,
        ) -> PermissionResult {
            PermissionResult::passthrough(format!(
                "Claude requested permissions to use {}, but you haven't granted it yet.",
                self.name
            ))
        }

        fn is_read_only(&self) -> bool {
            self.read_only
        }
    }

    fn empty_ctx() -> ToolPermissionContext {
        ToolPermissionContext::default()
    }

    #[tokio::test]
    async fn async_evaluator_falls_through_when_no_classifier_registered() {
        // Auto mode + Ask sync verdict + no secondary model registered →
        // returns the Ask unchanged (callers fall back to interactive
        // approval). This guards against the wrapper accidentally
        // erroring out in the no-LLM path.
        let mut ctx = empty_ctx();
        ctx.mode = PermissionMode::Auto;
        let tool = TestTool::new("Read", true);
        let decision = evaluate_permission_async(
            &tool,
            &serde_json::json!({}),
            &ctx,
            "user: hi\nassistant: hello",
        )
        .await;
        assert!(
            decision.is_ask(),
            "expected Ask passthrough, got {:?}",
            decision
        );
    }

    #[tokio::test]
    async fn async_evaluator_passes_through_non_auto_mode() {
        // Default mode → wrapper is a thin proxy over the sync evaluator.
        let ctx = empty_ctx();
        let tool = TestTool::new("Read", true);
        let sync_decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        let async_decision =
            evaluate_permission_async(&tool, &serde_json::json!({}), &ctx, "").await;
        assert_eq!(
            std::mem::discriminant(&sync_decision),
            std::mem::discriminant(&async_decision)
        );
    }

    #[test]
    fn test_default_mode_read_only_passthrough_to_ask() {
        let ctx = empty_ctx();
        let tool = TestTool::new("Read", true);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        // Read-only in default mode: tool returns passthrough -> becomes ask
        assert!(decision.is_ask());
    }

    #[test]
    fn test_deny_rule_takes_precedence() {
        let mut ctx = empty_ctx();
        ctx.always_deny_rules
            .insert(PermissionRuleSource::Session, vec!["Bash".to_string()]);

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        assert!(decision.is_deny());
    }

    #[test]
    fn test_allow_rule() {
        let mut ctx = empty_ctx();
        ctx.always_allow_rules
            .insert(PermissionRuleSource::Session, vec!["Bash".to_string()]);

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        assert!(decision.is_allow());
    }

    #[test]
    fn test_deny_before_allow() {
        let mut ctx = empty_ctx();
        ctx.always_deny_rules
            .insert(PermissionRuleSource::Session, vec!["Bash".to_string()]);
        ctx.always_allow_rules
            .insert(PermissionRuleSource::Session, vec!["Bash".to_string()]);

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        // Deny rules at step 1a, before allow rules at step 2b
        assert!(decision.is_deny());
    }

    #[test]
    fn test_bypass_mode_allows_all() {
        let mut ctx = empty_ctx();
        ctx.mode = PermissionMode::BypassPermissions;

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        assert!(decision.is_allow());
        if let PermissionDecision::Allow(allow) = &decision {
            if let Some(PermissionDecisionReason::Mode { mode }) = &allow.decision_reason {
                assert_eq!(*mode, PermissionMode::BypassPermissions);
            } else {
                panic!("expected mode reason");
            }
        }
    }

    #[test]
    fn test_dont_ask_mode_converts_ask_to_deny() {
        let mut ctx = empty_ctx();
        ctx.mode = PermissionMode::DontAsk;

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        assert!(decision.is_deny());
        if let PermissionDecision::Deny(deny) = &decision {
            if let PermissionDecisionReason::Mode { mode } = &deny.decision_reason {
                assert_eq!(*mode, PermissionMode::DontAsk);
            } else {
                panic!("expected mode reason");
            }
        }
    }

    #[test]
    fn test_headless_auto_deny() {
        let mut ctx = empty_ctx();
        ctx.should_avoid_permission_prompts = true;

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        assert!(decision.is_deny());
    }

    #[test]
    fn test_plan_mode_with_bypass_available_still_asks() {
        let mut ctx = empty_ctx();
        ctx.mode = PermissionMode::Plan;
        ctx.is_bypass_permissions_mode_available = true;
        ctx.is_auto_mode_available = Some(false);
        // Reset plan mode checker to not block
        register_plan_mode_checker(|_, _| false);

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        assert!(decision.is_ask());
    }

    #[test]
    fn test_plan_mode_with_auto_mode_allows() {
        let mut ctx = empty_ctx();
        ctx.mode = PermissionMode::Plan;
        ctx.is_bypass_permissions_mode_available = false;
        ctx.is_auto_mode_available = Some(true);
        // Reset plan mode checker to not block
        register_plan_mode_checker(|_, _| false);

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        assert!(decision.is_allow());
    }

    #[test]
    fn test_ask_rule_takes_precedence_over_allow() {
        let mut ctx = empty_ctx();
        ctx.always_ask_rules
            .insert(PermissionRuleSource::Session, vec!["Bash".to_string()]);
        ctx.always_allow_rules
            .insert(PermissionRuleSource::Session, vec!["Bash".to_string()]);

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        // Ask rule at step 1b, before allow at step 2b
        assert!(decision.is_ask());
    }

    #[test]
    fn test_deny_rule_beats_bypass_mode() {
        let mut ctx = empty_ctx();
        ctx.mode = PermissionMode::BypassPermissions;
        ctx.always_deny_rules
            .insert(PermissionRuleSource::Session, vec!["Bash".to_string()]);

        let tool = TestTool::new("Bash", false);
        let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
        // Deny is at step 1a, bypass is at step 2a
        assert!(decision.is_deny());
    }

    #[test]
    fn test_apply_permission_update_add_rules() {
        let ctx = empty_ctx();
        let updated = apply_permission_update(
            ctx,
            &PermissionUpdate::AddRules {
                destination: PermissionRuleSource::Session,
                rules: vec![PermissionRuleValue {
                    tool_name: "Bash".to_string(),
                    rule_content: Some("npm install".to_string()),
                }],
                behavior: PermissionBehavior::Allow,
            },
        );
        let rules = updated
            .always_allow_rules
            .get(&PermissionRuleSource::Session);
        assert!(rules.is_some());
        assert!(rules.unwrap().contains(&"Bash(npm install)".to_string()));
    }

    #[test]
    fn test_apply_permission_update_set_mode() {
        let ctx = empty_ctx();
        let updated = apply_permission_update(
            ctx,
            &PermissionUpdate::SetMode {
                destination: PermissionRuleSource::Session,
                mode: PermissionMode::AcceptEdits,
            },
        );
        assert_eq!(updated.mode, PermissionMode::AcceptEdits);
    }

    #[test]
    fn test_apply_permission_update_remove_rules() {
        let mut ctx = empty_ctx();
        ctx.always_allow_rules.insert(
            PermissionRuleSource::Session,
            vec!["Bash".to_string(), "Bash(npm install)".to_string()],
        );

        let updated = apply_permission_update(
            ctx,
            &PermissionUpdate::RemoveRules {
                destination: PermissionRuleSource::Session,
                rules: vec![PermissionRuleValue {
                    tool_name: "Bash".to_string(),
                    rule_content: Some("npm install".to_string()),
                }],
                behavior: PermissionBehavior::Allow,
            },
        );
        let rules = updated
            .always_allow_rules
            .get(&PermissionRuleSource::Session)
            .unwrap();
        assert!(rules.contains(&"Bash".to_string()));
        assert!(!rules.contains(&"Bash(npm install)".to_string()));
    }

    #[test]
    fn test_mcp_info_from_string() {
        let info = mcp_info_from_string("mcp__server1__tool1");
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.server_name, "server1");
        assert_eq!(info.tool_name, Some("tool1".to_string()));

        let info2 = mcp_info_from_string("mcp__server1");
        assert!(info2.is_some());
        let info2 = info2.unwrap();
        assert_eq!(info2.server_name, "server1");
        assert!(info2.tool_name.is_none());

        assert!(mcp_info_from_string("Bash").is_none());
    }

    #[test]
    fn test_check_rule_based_permissions_no_rules() {
        let ctx = empty_ctx();
        let tool = TestTool::new("Bash", false);
        let result = check_rule_based_permissions(&tool, &serde_json::json!({}), &ctx);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_rule_based_permissions_deny_rule() {
        let mut ctx = empty_ctx();
        ctx.always_deny_rules
            .insert(PermissionRuleSource::Session, vec!["Bash".to_string()]);
        let tool = TestTool::new("Bash", false);
        let result = check_rule_based_permissions(&tool, &serde_json::json!({}), &ctx);
        assert!(result.is_some());
        assert!(result.unwrap().is_deny());
    }

    #[test]
    fn test_sync_permission_rules_clears_old_rules() {
        let mut ctx = empty_ctx();
        ctx.always_allow_rules.insert(
            PermissionRuleSource::UserSettings,
            vec!["OldTool".to_string()],
        );

        let new_rules = vec![PermissionRule {
            source: PermissionRuleSource::UserSettings,
            rule_behavior: PermissionBehavior::Allow,
            rule_value: PermissionRuleValue {
                tool_name: "NewTool".to_string(),
                rule_content: None,
            },
        }];

        let updated = sync_permission_rules_from_disk(ctx, &new_rules);
        let rules = updated
            .always_allow_rules
            .get(&PermissionRuleSource::UserSettings)
            .unwrap();
        assert!(rules.contains(&"NewTool".to_string()));
        assert!(!rules.contains(&"OldTool".to_string()));
    }
}
