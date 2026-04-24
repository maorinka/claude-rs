//! Permission system for Claude Code.
//!
//! This module implements the complete permission pipeline that controls tool access:
//!
//! - **types**: Core type definitions (PermissionMode, PermissionDecision, PermissionRule, etc.)
//! - **evaluator**: The 5-step permission resolution pipeline
//! - **filesystem**: File path safety checks, working directory validation, rule matching
//! - **setup**: Initialization, dangerous permission detection, mode transitions

pub mod evaluator;
pub mod filesystem;
pub mod setup;
pub mod types;

// Re-export the most commonly used types at the module level.
pub use evaluator::{
    apply_permission_rules_to_context, apply_permission_update, apply_permission_updates,
    check_rule_based_permissions, evaluate_permission, get_allow_rules, get_ask_rules,
    get_deny_rule_for_agent, get_deny_rule_for_tool, get_deny_rules, get_rule_by_contents_for_tool,
    register_plan_mode_checker, sync_permission_rules_from_disk, tool_always_allowed_rule,
    McpToolInfo, PlanModeChecker, ToolPermissions,
};

pub use filesystem::{
    all_working_directories, check_path_safety_for_auto_edit, check_read_permission_for_tool,
    check_write_permission_for_tool, expand_path, generate_suggestions,
    get_file_read_ignore_patterns, get_paths_for_permission_check, matching_rule_for_input,
    normalize_case_for_comparison, normalize_patterns_to_path, path_in_allowed_working_path,
    path_in_working_path, PathSafetyResult, ToolType, DANGEROUS_DIRECTORIES, DANGEROUS_FILES,
};

pub use setup::{
    find_dangerous_classifier_permissions, find_overly_broad_bash_permissions,
    find_overly_broad_powershell_permissions, initial_permission_mode_from_cli,
    initialize_tool_permission_context, is_dangerous_bash_permission,
    is_dangerous_powershell_permission, is_dangerous_task_permission,
    is_overly_broad_bash_allow_rule, is_overly_broad_powershell_allow_rule,
    parse_tool_list_from_cli, prepare_context_for_plan_mode, restore_dangerous_permissions,
    strip_dangerous_permissions_for_auto_mode, transition_permission_mode, PermissionModeCliConfig,
    PermissionModeResult, ToolPermissionContextInit, AGENT_TOOL_NAME, BASH_TOOL_NAME,
    CROSS_PLATFORM_CODE_EXEC, DANGEROUS_BASH_PATTERNS, POWERSHELL_TOOL_NAME,
};

pub use types::{
    create_permission_request_message, escape_rule_content, get_legacy_tool_names,
    normalize_legacy_tool_name, unescape_rule_content, AdditionalWorkingDirectory,
    DangerousPermissionInfo, PermissionAllowDecision, PermissionAskDecision, PermissionBehavior,
    PermissionDecision, PermissionDecisionReason, PermissionDenyDecision, PermissionMode,
    PermissionResult, PermissionRule, PermissionRuleSource, PermissionRuleValue, PermissionUpdate,
    ToolPermissionContext, ToolPermissionRulesBySource,
};
