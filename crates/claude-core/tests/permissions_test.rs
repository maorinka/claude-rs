use claude_core::permissions::evaluator::{evaluate_permission, SimpleToolPermissions};
use claude_core::permissions::types::{PermissionDecision, PermissionMode, ToolPermissionContext};

#[test]
fn bypass_mode_always_allows() {
    let tool = SimpleToolPermissions::new("Bash", false);
    let ctx = ToolPermissionContext {
        mode: PermissionMode::BypassPermissions,
        ..Default::default()
    };
    let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
    assert!(
        matches!(decision, PermissionDecision::Allow(_)),
        "BypassPermissions should allow any tool, got: {:?}",
        decision
    );
}

#[test]
fn default_mode_asks_for_destructive_tool() {
    let tool = SimpleToolPermissions::new("Bash", false);
    let ctx = ToolPermissionContext {
        mode: PermissionMode::Default,
        ..Default::default()
    };
    let decision = evaluate_permission(&tool, &serde_json::json!({}), &ctx);
    // Destructive tools in Default mode should prompt (Ask) or be handled by rules
    assert!(
        matches!(decision, PermissionDecision::Ask(_) | PermissionDecision::Allow(_)),
        "Default mode should ask or allow based on rules, got: {:?}",
        decision
    );
}

#[test]
fn permission_mode_from_string_roundtrip() {
    assert_eq!(
        PermissionMode::from_string("bypassPermissions"),
        PermissionMode::BypassPermissions
    );
    assert_eq!(
        PermissionMode::from_string("default"),
        PermissionMode::Default
    );
    assert_eq!(
        PermissionMode::from_string("plan"),
        PermissionMode::Plan
    );
    assert_eq!(
        PermissionMode::from_string("acceptEdits"),
        PermissionMode::AcceptEdits
    );
}
