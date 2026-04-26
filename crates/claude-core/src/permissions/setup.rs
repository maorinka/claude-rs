//! Permission setup: initialization, rule loading, dangerous permission detection,
//! and auto-mode gates.
//!
//! This module translates the TypeScript `permissionSetup.ts` logic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::evaluator::{
    apply_permission_rules_to_context, apply_permission_update, get_allow_rules,
};
use super::types::{
    normalize_legacy_tool_name, AdditionalWorkingDirectory, DangerousPermissionInfo,
    PermissionBehavior, PermissionMode, PermissionRule, PermissionRuleSource, PermissionRuleValue,
    PermissionUpdate, ToolPermissionContext, ToolPermissionRulesBySource,
};

// ============================================================================
// Constants: Tool Names
// ============================================================================

pub const BASH_TOOL_NAME: &str = "Bash";
pub const POWERSHELL_TOOL_NAME: &str = "PowerShell";
pub const AGENT_TOOL_NAME: &str = "Agent";

// ============================================================================
// Dangerous Bash/PowerShell Patterns
// ============================================================================

/// Cross-platform code-execution entry points shared between Bash and PowerShell.
pub const CROSS_PLATFORM_CODE_EXEC: &[&str] = &[
    "python", "python3", "python2", "node", "deno", "tsx", "ruby", "perl", "php", "lua", "npx",
    "bunx", "npm run", "yarn run", "pnpm run", "bun run", "bash", "sh", "ssh",
];

/// Dangerous Bash patterns (superset of CROSS_PLATFORM_CODE_EXEC).
pub const DANGEROUS_BASH_PATTERNS: &[&str] = &[
    "python", "python3", "python2", "node", "deno", "tsx", "ruby", "perl", "php", "lua", "npx",
    "bunx", "npm run", "yarn run", "pnpm run", "bun run", "bash", "sh", "ssh", "zsh", "fish",
    "eval", "exec", "env", "xargs", "sudo",
];

/// Additional dangerous PowerShell patterns (cmdlets, process spawners, etc.).
const DANGEROUS_POWERSHELL_PATTERNS: &[&str] = &[
    "pwsh",
    "powershell",
    "cmd",
    "wsl",
    "iex",
    "invoke-expression",
    "icm",
    "invoke-command",
    "start-process",
    "saps",
    "start",
    "start-job",
    "sajb",
    "start-threadjob",
    "register-objectevent",
    "register-engineevent",
    "register-wmievent",
    "register-scheduledjob",
    "new-pssession",
    "nsn",
    "enter-pssession",
    "etsn",
    "add-type",
    "new-object",
];

// ============================================================================
// isDangerousBashPermission
// ============================================================================

/// Checks if a Bash permission rule is dangerous for auto mode.
///
/// A rule is dangerous if it would auto-allow commands that execute arbitrary code,
/// bypassing the classifier's safety evaluation.
///
/// Dangerous patterns:
/// 1. Tool-level allow (Bash with no ruleContent) -- allows ALL commands
/// 2. Prefix rules for script interpreters (python:*, node:*, etc.)
/// 3. Wildcard rules matching interpreters (python*, node*, etc.)
pub fn is_dangerous_bash_permission(tool_name: &str, rule_content: Option<&str>) -> bool {
    if tool_name != BASH_TOOL_NAME {
        return false;
    }

    // Tool-level allow (Bash with no content, or Bash(*))
    match rule_content {
        None | Some("") => return true,
        _ => {}
    }

    let content = rule_content.unwrap().trim().to_lowercase();

    // Standalone wildcard
    if content == "*" {
        return true;
    }

    // Check against dangerous patterns
    for pattern in DANGEROUS_BASH_PATTERNS {
        let lower_pattern = pattern.to_lowercase();

        // Exact match
        if content == lower_pattern {
            return true;
        }

        // Prefix syntax: "python:*"
        if content == format!("{}:*", lower_pattern) {
            return true;
        }

        // Wildcard at end: "python*"
        if content == format!("{}*", lower_pattern) {
            return true;
        }

        // Wildcard with space: "python *"
        if content == format!("{} *", lower_pattern) {
            return true;
        }

        // Patterns like "python -*" matching "python -c 'code'"
        if content.starts_with(&format!("{} -", lower_pattern)) && content.ends_with('*') {
            return true;
        }
    }

    false
}

// ============================================================================
// isDangerousPowerShellPermission
// ============================================================================

/// Checks if a PowerShell permission rule is dangerous for auto mode.
pub fn is_dangerous_powershell_permission(tool_name: &str, rule_content: Option<&str>) -> bool {
    if tool_name != POWERSHELL_TOOL_NAME {
        return false;
    }

    match rule_content {
        None | Some("") => return true,
        _ => {}
    }

    let content = rule_content.unwrap().trim().to_lowercase();

    if content == "*" {
        return true;
    }

    // Combine cross-platform patterns with PS-specific ones
    let all_patterns: Vec<&str> = CROSS_PLATFORM_CODE_EXEC
        .iter()
        .chain(DANGEROUS_POWERSHELL_PATTERNS.iter())
        .copied()
        .collect();

    for pattern in &all_patterns {
        let lower_pattern = pattern.to_lowercase();

        if content == lower_pattern {
            return true;
        }
        if content == format!("{}:*", lower_pattern) {
            return true;
        }
        if content == format!("{}*", lower_pattern) {
            return true;
        }
        if content == format!("{} *", lower_pattern) {
            return true;
        }
        if content.starts_with(&format!("{} -", lower_pattern)) && content.ends_with('*') {
            return true;
        }

        // .exe variant: python -> python.exe
        let sp = lower_pattern.find(' ');
        let exe = match sp {
            None => format!("{}.exe", lower_pattern),
            Some(idx) => format!("{}.exe{}", &lower_pattern[..idx], &lower_pattern[idx..]),
        };

        if content == exe {
            return true;
        }
        if content == format!("{}:*", exe) {
            return true;
        }
        if content == format!("{}*", exe) {
            return true;
        }
        if content == format!("{} *", exe) {
            return true;
        }
        if content.starts_with(&format!("{} -", exe)) && content.ends_with('*') {
            return true;
        }
    }

    false
}

// ============================================================================
// isDangerousTaskPermission
// ============================================================================

/// Checks if an Agent (sub-agent) permission rule is dangerous for auto mode.
/// Any Agent allow rule would auto-approve sub-agent spawns before the classifier evaluates them.
pub fn is_dangerous_task_permission(tool_name: &str, _rule_content: Option<&str>) -> bool {
    normalize_legacy_tool_name(tool_name) == AGENT_TOOL_NAME
}

// ============================================================================
// isDangerousClassifierPermission
// ============================================================================

/// Checks if a permission rule is dangerous for auto mode (composite check).
fn is_dangerous_classifier_permission(tool_name: &str, rule_content: Option<&str>) -> bool {
    is_dangerous_bash_permission(tool_name, rule_content)
        || is_dangerous_powershell_permission(tool_name, rule_content)
        || is_dangerous_task_permission(tool_name, rule_content)
}

// ============================================================================
// findDangerousClassifierPermissions
// ============================================================================

/// Finds all dangerous permissions from rules loaded from disk and CLI arguments.
/// Returns structured info about each dangerous permission found.
pub fn find_dangerous_classifier_permissions(
    rules: &[PermissionRule],
    cli_allowed_tools: &[String],
) -> Vec<DangerousPermissionInfo> {
    let mut dangerous = Vec::new();

    // Check rules loaded from settings
    for rule in rules {
        if rule.rule_behavior == PermissionBehavior::Allow
            && is_dangerous_classifier_permission(
                &rule.rule_value.tool_name,
                rule.rule_value.rule_content.as_deref(),
            )
        {
            let rule_string = match &rule.rule_value.rule_content {
                Some(content) => format!("{}({})", rule.rule_value.tool_name, content),
                None => format!("{}(*)", rule.rule_value.tool_name),
            };
            dangerous.push(DangerousPermissionInfo {
                rule_value: rule.rule_value.clone(),
                source: rule.source.clone(),
                rule_display: rule_string,
                source_display: rule.source.display_name().to_string(),
            });
        }
    }

    // Check CLI --allowed-tools arguments
    for tool_spec in cli_allowed_tools {
        let parsed = parse_tool_spec(tool_spec);
        if is_dangerous_classifier_permission(&parsed.0, parsed.1.as_deref()) {
            let rule_display = match &parsed.1 {
                Some(_) => tool_spec.clone(),
                None => format!("{}(*)", parsed.0),
            };
            dangerous.push(DangerousPermissionInfo {
                rule_value: PermissionRuleValue {
                    tool_name: parsed.0,
                    rule_content: parsed.1,
                },
                source: PermissionRuleSource::CliArg,
                rule_display,
                source_display: "--allowed-tools".to_string(),
            });
        }
    }

    dangerous
}

/// Parse a tool spec like "Bash" or "Bash(pattern)" into (tool_name, optional_content).
fn parse_tool_spec(spec: &str) -> (String, Option<String>) {
    // Use regex to parse "Tool" or "Tool(content)"
    if let Some(open) = spec.find('(') {
        if spec.ends_with(')') {
            let tool_name = spec[..open].trim().to_string();
            let content = spec[open + 1..spec.len() - 1].trim().to_string();
            if content.is_empty() || content == "*" {
                return (tool_name, None);
            }
            return (tool_name, Some(content));
        }
    }
    (spec.to_string(), None)
}

// ============================================================================
// Overly Broad Bash/PowerShell Permissions
// ============================================================================

/// Checks if a Bash allow rule is overly broad (equivalent to YOLO mode).
pub fn is_overly_broad_bash_allow_rule(rule_value: &PermissionRuleValue) -> bool {
    rule_value.tool_name == BASH_TOOL_NAME && rule_value.rule_content.is_none()
}

/// PowerShell equivalent of is_overly_broad_bash_allow_rule.
pub fn is_overly_broad_powershell_allow_rule(rule_value: &PermissionRuleValue) -> bool {
    rule_value.tool_name == POWERSHELL_TOOL_NAME && rule_value.rule_content.is_none()
}

/// Finds all overly broad Bash allow rules from settings and CLI arguments.
pub fn find_overly_broad_bash_permissions(
    rules: &[PermissionRule],
    cli_allowed_tools: &[String],
) -> Vec<DangerousPermissionInfo> {
    let mut result = Vec::new();

    for rule in rules {
        if rule.rule_behavior == PermissionBehavior::Allow
            && is_overly_broad_bash_allow_rule(&rule.rule_value)
        {
            result.push(DangerousPermissionInfo {
                rule_value: rule.rule_value.clone(),
                source: rule.source.clone(),
                rule_display: format!("{}(*)", BASH_TOOL_NAME),
                source_display: rule.source.display_name().to_string(),
            });
        }
    }

    for tool_spec in cli_allowed_tools {
        let parsed = PermissionRuleValue::from_string(tool_spec);
        if is_overly_broad_bash_allow_rule(&parsed) {
            result.push(DangerousPermissionInfo {
                rule_value: parsed,
                source: PermissionRuleSource::CliArg,
                rule_display: format!("{}(*)", BASH_TOOL_NAME),
                source_display: "--allowed-tools".to_string(),
            });
        }
    }

    result
}

/// Finds all overly broad PowerShell allow rules.
pub fn find_overly_broad_powershell_permissions(
    rules: &[PermissionRule],
    cli_allowed_tools: &[String],
) -> Vec<DangerousPermissionInfo> {
    let mut result = Vec::new();

    for rule in rules {
        if rule.rule_behavior == PermissionBehavior::Allow
            && is_overly_broad_powershell_allow_rule(&rule.rule_value)
        {
            result.push(DangerousPermissionInfo {
                rule_value: rule.rule_value.clone(),
                source: rule.source.clone(),
                rule_display: format!("{}(*)", POWERSHELL_TOOL_NAME),
                source_display: rule.source.display_name().to_string(),
            });
        }
    }

    for tool_spec in cli_allowed_tools {
        let parsed = PermissionRuleValue::from_string(tool_spec);
        if is_overly_broad_powershell_allow_rule(&parsed) {
            result.push(DangerousPermissionInfo {
                rule_value: parsed,
                source: PermissionRuleSource::CliArg,
                rule_display: format!("{}(*)", POWERSHELL_TOOL_NAME),
                source_display: "--allowed-tools".to_string(),
            });
        }
    }

    result
}

// ============================================================================
// stripDangerousPermissionsForAutoMode / restoreDangerousPermissions
// ============================================================================

/// Prepares a ToolPermissionContext for auto mode by stripping dangerous permissions
/// that would bypass the classifier. Returns the cleaned context.
pub fn strip_dangerous_permissions_for_auto_mode(
    mut context: ToolPermissionContext,
) -> ToolPermissionContext {
    // Gather all allow rules as structured PermissionRule objects
    let rules: Vec<PermissionRule> = get_allow_rules(&context);

    let dangerous_permissions = find_dangerous_classifier_permissions(&rules, &[]);

    if dangerous_permissions.is_empty() {
        if context.stripped_dangerous_rules.is_none() {
            context.stripped_dangerous_rules = Some(HashMap::new());
        }
        return context;
    }

    // Build the stash of what we're stripping
    let mut stripped: ToolPermissionRulesBySource = HashMap::new();
    for perm in &dangerous_permissions {
        if !perm.source.is_update_destination() {
            continue;
        }
        stripped
            .entry(perm.source.clone())
            .or_default()
            .push(perm.rule_value.to_rule_string());
    }

    // Remove the dangerous permissions
    context = remove_dangerous_permissions(context, &dangerous_permissions);
    context.stripped_dangerous_rules = Some(stripped);

    context
}

/// Restores dangerous allow rules previously stashed by strip_dangerous_permissions_for_auto_mode.
/// Called when leaving auto mode so user's Bash(python:*), Agent(*), etc. rules work again.
pub fn restore_dangerous_permissions(mut context: ToolPermissionContext) -> ToolPermissionContext {
    let stash = match context.stripped_dangerous_rules.take() {
        Some(s) => s,
        None => return context,
    };

    for (source, rule_strings) in &stash {
        if rule_strings.is_empty() {
            continue;
        }
        let rules: Vec<PermissionRuleValue> = rule_strings
            .iter()
            .map(|s| PermissionRuleValue::from_string(s))
            .collect();
        context = apply_permission_update(
            context,
            &PermissionUpdate::AddRules {
                destination: source.clone(),
                rules,
                behavior: PermissionBehavior::Allow,
            },
        );
    }

    context.stripped_dangerous_rules = None;
    context
}

/// Remove dangerous permissions from context.
fn remove_dangerous_permissions(
    mut context: ToolPermissionContext,
    dangerous: &[DangerousPermissionInfo],
) -> ToolPermissionContext {
    // Group by source
    let mut rules_by_source: HashMap<PermissionRuleSource, Vec<PermissionRuleValue>> =
        HashMap::new();
    for perm in dangerous {
        if !perm.source.is_update_destination() {
            continue;
        }
        rules_by_source
            .entry(perm.source.clone())
            .or_default()
            .push(perm.rule_value.clone());
    }

    for (source, rules) in rules_by_source {
        context = apply_permission_update(
            context,
            &PermissionUpdate::RemoveRules {
                destination: source,
                rules,
                behavior: PermissionBehavior::Allow,
            },
        );
    }

    context
}

// ============================================================================
// Permission Mode Transition
// ============================================================================

/// Handles all state transitions when switching permission modes.
/// Centralizes side-effects so every activation path behaves identically.
///
/// Returns the (possibly modified) context. Caller is responsible for setting
/// the mode on the returned context.
pub fn transition_permission_mode(
    from_mode: &PermissionMode,
    to_mode: &PermissionMode,
    context: ToolPermissionContext,
) -> ToolPermissionContext {
    // Same mode -> no-op
    if from_mode == to_mode {
        return context;
    }

    let mut ctx = context;

    // Transitioning to auto mode: strip dangerous permissions
    let from_uses_classifier = *from_mode == PermissionMode::Auto;
    let to_uses_classifier = *to_mode == PermissionMode::Auto;

    if to_uses_classifier && !from_uses_classifier {
        ctx = strip_dangerous_permissions_for_auto_mode(ctx);
    } else if from_uses_classifier && !to_uses_classifier {
        ctx = restore_dangerous_permissions(ctx);
    }

    // Transitioning to plan mode: stash pre-plan mode
    if *to_mode == PermissionMode::Plan && *from_mode != PermissionMode::Plan {
        ctx.pre_plan_mode = Some(from_mode.clone());
    }

    // Leaving plan mode: clear pre-plan mode
    if *from_mode == PermissionMode::Plan && *to_mode != PermissionMode::Plan {
        ctx.pre_plan_mode = None;
    }

    ctx
}

/// Centralized plan-mode entry. Stashes the current mode as pre_plan_mode
/// so ExitPlanMode can restore it.
pub fn prepare_context_for_plan_mode(mut context: ToolPermissionContext) -> ToolPermissionContext {
    let current_mode = context.mode.clone();
    if current_mode == PermissionMode::Plan {
        return context;
    }
    context.pre_plan_mode = Some(current_mode);
    context
}

// ============================================================================
// CLI Parsing
// ============================================================================

/// Parse a list of tool specs from CLI arguments.
/// Handles comma-separated and space-separated lists, with parenthesized content.
pub fn parse_tool_list_from_cli(tools: &[String]) -> Vec<String> {
    let mut result = Vec::new();

    for tool_string in tools {
        if tool_string.is_empty() {
            continue;
        }

        let mut current = String::new();
        let mut in_parens = false;

        for ch in tool_string.chars() {
            match ch {
                '(' => {
                    in_parens = true;
                    current.push(ch);
                }
                ')' => {
                    in_parens = false;
                    current.push(ch);
                }
                ',' => {
                    if in_parens {
                        current.push(ch);
                    } else {
                        let trimmed = current.trim().to_string();
                        if !trimmed.is_empty() {
                            result.push(trimmed);
                        }
                        current.clear();
                    }
                }
                ' ' => {
                    if in_parens {
                        current.push(ch);
                    } else {
                        let trimmed = current.trim().to_string();
                        if !trimmed.is_empty() {
                            result.push(trimmed);
                        }
                        current.clear();
                    }
                }
                _ => {
                    current.push(ch);
                }
            }
        }

        let trimmed = current.trim().to_string();
        if !trimmed.is_empty() {
            result.push(trimmed);
        }
    }

    result
}

// ============================================================================
// Initial Permission Mode from CLI
// ============================================================================

/// Configuration for initializing permission mode from CLI flags.
pub struct PermissionModeCliConfig {
    pub permission_mode_cli: Option<String>,
    pub dangerously_skip_permissions: bool,
    pub disable_bypass_permissions: bool,
}

/// Result of permission mode initialization.
pub struct PermissionModeResult {
    pub mode: PermissionMode,
    pub notification: Option<String>,
}

/// Safely convert CLI flags to a PermissionMode.
/// Handles priority: dangerously_skip_permissions > CLI mode > settings default > default.
pub fn initial_permission_mode_from_cli(config: &PermissionModeCliConfig) -> PermissionModeResult {
    let mut ordered_modes: Vec<PermissionMode> = Vec::new();
    let mut notification: Option<String> = None;

    // Highest priority: --dangerously-skip-permissions
    if config.dangerously_skip_permissions {
        ordered_modes.push(PermissionMode::BypassPermissions);
    }

    // CLI --permission-mode flag
    if let Some(ref mode_str) = config.permission_mode_cli {
        let parsed = PermissionMode::from_string(mode_str);
        ordered_modes.push(parsed);
    }

    // Use first valid mode
    for mode in ordered_modes {
        if mode == PermissionMode::BypassPermissions && config.disable_bypass_permissions {
            notification = Some(
                "Bypass permissions mode was disabled by your organization policy".to_string(),
            );
            continue; // Skip disabled mode
        }
        return PermissionModeResult { mode, notification };
    }

    // Default
    PermissionModeResult {
        mode: PermissionMode::Default,
        notification,
    }
}

// ============================================================================
// initializeToolPermissionContext
// ============================================================================

/// Result of initializing the tool permission context.
pub struct ToolPermissionContextInit {
    pub tool_permission_context: ToolPermissionContext,
    pub warnings: Vec<String>,
    pub dangerous_permissions: Vec<DangerousPermissionInfo>,
    pub overly_broad_bash_permissions: Vec<DangerousPermissionInfo>,
}

/// Initialize the ToolPermissionContext from CLI arguments and loaded rules.
///
/// This is the Rust equivalent of the TS `initializeToolPermissionContext`.
pub fn initialize_tool_permission_context(
    allowed_tools_cli: &[String],
    disallowed_tools_cli: &[String],
    permission_mode: PermissionMode,
    allow_dangerously_skip_permissions: bool,
    add_dirs: &[String],
    rules_from_disk: &[PermissionRule],
    working_directory: PathBuf,
) -> ToolPermissionContextInit {
    // Parse CLI tool lists
    let parsed_allowed: Vec<String> = parse_tool_list_from_cli(allowed_tools_cli)
        .into_iter()
        .map(|s| {
            let rv = PermissionRuleValue::from_string(&s);
            rv.to_rule_string()
        })
        .collect();

    let parsed_disallowed = parse_tool_list_from_cli(disallowed_tools_cli);

    let warnings: Vec<String> = Vec::new();

    // Detect overly broad permissions
    let overly_broad_bash_permissions: Vec<DangerousPermissionInfo> = {
        let mut v = find_overly_broad_bash_permissions(rules_from_disk, &parsed_allowed);
        v.extend(find_overly_broad_powershell_permissions(
            rules_from_disk,
            &parsed_allowed,
        ));
        v
    };

    // Detect dangerous permissions for auto mode
    let dangerous_permissions = if permission_mode == PermissionMode::Auto {
        find_dangerous_classifier_permissions(rules_from_disk, &parsed_allowed)
    } else {
        Vec::new()
    };

    let is_bypass_available =
        permission_mode == PermissionMode::BypassPermissions || allow_dangerously_skip_permissions;

    let additional_working_directories: HashMap<String, AdditionalWorkingDirectory> =
        HashMap::new();

    // Build initial context
    let mut context = ToolPermissionContext {
        mode: permission_mode,
        additional_working_directories,
        always_allow_rules: {
            let mut m = HashMap::new();
            m.insert(PermissionRuleSource::CliArg, parsed_allowed);
            m
        },
        always_deny_rules: {
            let mut m = HashMap::new();
            m.insert(PermissionRuleSource::CliArg, parsed_disallowed);
            m
        },
        always_ask_rules: HashMap::new(),
        is_bypass_permissions_mode_available: is_bypass_available,
        stripped_dangerous_rules: None,
        should_avoid_permission_prompts: false,
        // Auto-mode classifier state is not wired yet; default to inactive.
        // Plan mode must not bypass permissions just because bypass mode is
        // feature-available.
        is_auto_mode_available: Some(false),
        pre_plan_mode: None,
        working_directory,
    };

    // Apply rules from disk
    context = apply_permission_rules_to_context(context, rules_from_disk);

    // Add directories from add_dirs
    for dir in add_dirs {
        let abs_dir = if Path::new(dir).is_absolute() {
            PathBuf::from(dir)
        } else {
            context.working_directory.join(dir)
        };

        if abs_dir.is_dir() {
            let dir_str = abs_dir.to_string_lossy().to_string();
            context.additional_working_directories.insert(
                dir_str.clone(),
                AdditionalWorkingDirectory {
                    path: dir_str,
                    source: PermissionRuleSource::CliArg,
                },
            );
        }
    }

    ToolPermissionContextInit {
        tool_permission_context: context,
        warnings,
        dangerous_permissions,
        overly_broad_bash_permissions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dangerous_bash_permission_tool_level() {
        assert!(is_dangerous_bash_permission("Bash", None));
        assert!(is_dangerous_bash_permission("Bash", Some("")));
        assert!(is_dangerous_bash_permission("Bash", Some("*")));
    }

    #[test]
    fn test_is_dangerous_bash_permission_interpreters() {
        assert!(is_dangerous_bash_permission("Bash", Some("python:*")));
        assert!(is_dangerous_bash_permission("Bash", Some("python3:*")));
        assert!(is_dangerous_bash_permission("Bash", Some("node:*")));
        assert!(is_dangerous_bash_permission("Bash", Some("ruby*")));
        assert!(is_dangerous_bash_permission("Bash", Some("perl *")));
        assert!(is_dangerous_bash_permission("Bash", Some("python -c *")));
    }

    #[test]
    fn test_is_dangerous_bash_permission_safe_patterns() {
        assert!(!is_dangerous_bash_permission("Bash", Some("npm install")));
        assert!(!is_dangerous_bash_permission("Bash", Some("git status")));
        assert!(!is_dangerous_bash_permission("Bash", Some("ls:*")));
    }

    #[test]
    fn test_is_dangerous_bash_permission_not_bash() {
        assert!(!is_dangerous_bash_permission("Edit", None));
        assert!(!is_dangerous_bash_permission("Read", Some("python:*")));
    }

    #[test]
    fn test_is_dangerous_powershell_permission() {
        assert!(is_dangerous_powershell_permission("PowerShell", None));
        assert!(is_dangerous_powershell_permission("PowerShell", Some("*")));
        assert!(is_dangerous_powershell_permission(
            "PowerShell",
            Some("iex:*")
        ));
        assert!(is_dangerous_powershell_permission(
            "PowerShell",
            Some("invoke-expression:*")
        ));
        assert!(is_dangerous_powershell_permission(
            "PowerShell",
            Some("start-process:*")
        ));
        assert!(is_dangerous_powershell_permission(
            "PowerShell",
            Some("python:*")
        ));
    }

    #[test]
    fn test_is_dangerous_powershell_permission_exe_variant() {
        assert!(is_dangerous_powershell_permission(
            "PowerShell",
            Some("python.exe:*")
        ));
    }

    #[test]
    fn test_is_dangerous_powershell_not_ps() {
        assert!(!is_dangerous_powershell_permission("Bash", Some("iex:*")));
    }

    #[test]
    fn test_is_dangerous_task_permission() {
        assert!(is_dangerous_task_permission("Agent", None));
        assert!(is_dangerous_task_permission("Task", Some("anything"))); // legacy name
        assert!(!is_dangerous_task_permission("Bash", None));
    }

    #[test]
    fn test_find_dangerous_classifier_permissions() {
        let rules = vec![
            PermissionRule {
                source: PermissionRuleSource::UserSettings,
                rule_behavior: PermissionBehavior::Allow,
                rule_value: PermissionRuleValue {
                    tool_name: "Bash".to_string(),
                    rule_content: None,
                },
            },
            PermissionRule {
                source: PermissionRuleSource::UserSettings,
                rule_behavior: PermissionBehavior::Allow,
                rule_value: PermissionRuleValue {
                    tool_name: "Edit".to_string(),
                    rule_content: None,
                },
            },
        ];

        let dangerous = find_dangerous_classifier_permissions(&rules, &[]);
        assert_eq!(dangerous.len(), 1);
        assert_eq!(dangerous[0].rule_value.tool_name, "Bash");
    }

    #[test]
    fn test_find_dangerous_from_cli() {
        let dangerous = find_dangerous_classifier_permissions(
            &[],
            &["Bash".to_string(), "Agent(Explore)".to_string()],
        );
        assert_eq!(dangerous.len(), 2); // Bash(*) and Agent(Explore)
    }

    #[test]
    fn test_parse_tool_list_from_cli() {
        assert_eq!(
            parse_tool_list_from_cli(&["Bash,Edit,Read".to_string()]),
            vec!["Bash", "Edit", "Read"]
        );

        assert_eq!(
            parse_tool_list_from_cli(&["Bash(npm install),Edit".to_string()]),
            vec!["Bash(npm install)", "Edit"]
        );

        assert_eq!(
            parse_tool_list_from_cli(&["Bash Edit Read".to_string()]),
            vec!["Bash", "Edit", "Read"]
        );
    }

    #[test]
    fn test_parse_tool_list_preserves_parens_content() {
        let result = parse_tool_list_from_cli(&["Bash(npm install, yarn add),Edit".to_string()]);
        assert_eq!(result, vec!["Bash(npm install, yarn add)", "Edit"]);
    }

    #[test]
    fn test_initial_permission_mode_default() {
        let config = PermissionModeCliConfig {
            permission_mode_cli: None,
            dangerously_skip_permissions: false,
            disable_bypass_permissions: false,
        };
        let result = initial_permission_mode_from_cli(&config);
        assert_eq!(result.mode, PermissionMode::Default);
    }

    #[test]
    fn test_initial_permission_mode_bypass() {
        let config = PermissionModeCliConfig {
            permission_mode_cli: None,
            dangerously_skip_permissions: true,
            disable_bypass_permissions: false,
        };
        let result = initial_permission_mode_from_cli(&config);
        assert_eq!(result.mode, PermissionMode::BypassPermissions);
    }

    #[test]
    fn test_initial_permission_mode_bypass_disabled() {
        let config = PermissionModeCliConfig {
            permission_mode_cli: None,
            dangerously_skip_permissions: true,
            disable_bypass_permissions: true,
        };
        let result = initial_permission_mode_from_cli(&config);
        assert_eq!(result.mode, PermissionMode::Default);
        assert!(result.notification.is_some());
    }

    #[test]
    fn test_initial_permission_mode_cli_flag() {
        let config = PermissionModeCliConfig {
            permission_mode_cli: Some("acceptEdits".to_string()),
            dangerously_skip_permissions: false,
            disable_bypass_permissions: false,
        };
        let result = initial_permission_mode_from_cli(&config);
        assert_eq!(result.mode, PermissionMode::AcceptEdits);
    }

    #[test]
    fn test_strip_dangerous_permissions() {
        let mut ctx = ToolPermissionContext::default();
        ctx.always_allow_rules.insert(
            PermissionRuleSource::Session,
            vec![
                "Bash".to_string(),
                "Edit".to_string(),
                "Bash(python:*)".to_string(),
            ],
        );

        let stripped = strip_dangerous_permissions_for_auto_mode(ctx);

        let session_rules = stripped
            .always_allow_rules
            .get(&PermissionRuleSource::Session)
            .unwrap();
        // Bash and Bash(python:*) should be stripped, Edit remains
        assert!(session_rules.contains(&"Edit".to_string()));
        assert!(!session_rules.contains(&"Bash".to_string()));
        assert!(!session_rules.contains(&"Bash(python:*)".to_string()));

        // Stash should contain the stripped rules
        assert!(stripped.stripped_dangerous_rules.is_some());
    }

    #[test]
    fn test_restore_dangerous_permissions() {
        let mut ctx = ToolPermissionContext::default();
        ctx.always_allow_rules
            .insert(PermissionRuleSource::Session, vec!["Edit".to_string()]);
        let mut stash: ToolPermissionRulesBySource = HashMap::new();
        stash.insert(
            PermissionRuleSource::Session,
            vec!["Bash".to_string(), "Bash(python:*)".to_string()],
        );
        ctx.stripped_dangerous_rules = Some(stash);

        let restored = restore_dangerous_permissions(ctx);

        let session_rules = restored
            .always_allow_rules
            .get(&PermissionRuleSource::Session)
            .unwrap();
        assert!(session_rules.contains(&"Edit".to_string()));
        assert!(session_rules.contains(&"Bash".to_string()));
        assert!(session_rules.contains(&"Bash(python:*)".to_string()));
        assert!(restored.stripped_dangerous_rules.is_none());
    }

    #[test]
    fn test_transition_permission_mode_same() {
        let ctx = ToolPermissionContext::default();
        let result = transition_permission_mode(
            &PermissionMode::Default,
            &PermissionMode::Default,
            ctx.clone(),
        );
        // Same mode -> context unchanged
        assert_eq!(result.mode, ctx.mode);
    }

    #[test]
    fn test_transition_to_plan_stashes_mode() {
        let ctx = ToolPermissionContext {
            mode: PermissionMode::AcceptEdits,
            ..Default::default()
        };
        let result =
            transition_permission_mode(&PermissionMode::AcceptEdits, &PermissionMode::Plan, ctx);
        assert_eq!(result.pre_plan_mode, Some(PermissionMode::AcceptEdits));
    }

    #[test]
    fn test_transition_from_plan_clears_pre_plan() {
        let ctx = ToolPermissionContext {
            mode: PermissionMode::Plan,
            pre_plan_mode: Some(PermissionMode::AcceptEdits),
            ..Default::default()
        };
        let result =
            transition_permission_mode(&PermissionMode::Plan, &PermissionMode::Default, ctx);
        assert!(result.pre_plan_mode.is_none());
    }

    #[test]
    fn test_overly_broad_bash() {
        let rules = vec![PermissionRule {
            source: PermissionRuleSource::UserSettings,
            rule_behavior: PermissionBehavior::Allow,
            rule_value: PermissionRuleValue {
                tool_name: "Bash".to_string(),
                rule_content: None,
            },
        }];

        let broad = find_overly_broad_bash_permissions(&rules, &[]);
        assert_eq!(broad.len(), 1);
    }

    #[test]
    fn test_overly_broad_bash_with_content_not_broad() {
        let rules = vec![PermissionRule {
            source: PermissionRuleSource::UserSettings,
            rule_behavior: PermissionBehavior::Allow,
            rule_value: PermissionRuleValue {
                tool_name: "Bash".to_string(),
                rule_content: Some("npm install".to_string()),
            },
        }];

        let broad = find_overly_broad_bash_permissions(&rules, &[]);
        assert!(broad.is_empty());
    }

    #[test]
    fn test_initialize_context() {
        let result = initialize_tool_permission_context(
            &["Bash(npm install)".to_string()],
            &["Agent".to_string()],
            PermissionMode::Default,
            false,
            &[],
            &[],
            PathBuf::from("/project"),
        );

        let ctx = &result.tool_permission_context;
        assert_eq!(ctx.mode, PermissionMode::Default);

        let allow_cli = ctx
            .always_allow_rules
            .get(&PermissionRuleSource::CliArg)
            .unwrap();
        assert!(allow_cli.contains(&"Bash(npm install)".to_string()));

        let deny_cli = ctx
            .always_deny_rules
            .get(&PermissionRuleSource::CliArg)
            .unwrap();
        assert!(deny_cli.contains(&"Agent".to_string()));
    }
}
