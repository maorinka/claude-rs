use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================================
// Permission Modes
// ============================================================================

/// The seven permission modes matching the TypeScript PermissionMode union.
///
/// External modes (user-addressable): Default, AcceptEdits, Plan, BypassPermissions, DontAsk
/// Internal modes: Auto, Bubble
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Normal permission checking -- reads allowed, writes prompt.
    #[default]
    Default,
    /// Auto-allow file edits within working directories.
    AcceptEdits,
    /// AI classifier decides whether to allow or deny.
    Auto,
    /// Plan-only mode -- only read operations allowed; writes blocked until ExitPlanMode.
    Plan,
    /// Skip all permission checks (dangerous).
    BypassPermissions,
    /// Like Default but converts Ask -> Deny (headless/non-interactive).
    DontAsk,
    /// Bubble up to parent (SDK agent delegation).
    Bubble,
}

impl PermissionMode {
    /// Parse a mode string (case-insensitive) into a PermissionMode.
    /// Returns Default for unrecognized strings.
    pub fn from_string(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "default" => PermissionMode::Default,
            "acceptedits" => PermissionMode::AcceptEdits,
            "auto" => PermissionMode::Auto,
            "plan" => PermissionMode::Plan,
            "bypasspermissions" => PermissionMode::BypassPermissions,
            "dontask" => PermissionMode::DontAsk,
            "bubble" => PermissionMode::Bubble,
            _ => PermissionMode::Default,
        }
    }

    /// Human-readable display title for this mode.
    pub fn title(&self) -> &'static str {
        match self {
            PermissionMode::Default => "Default",
            PermissionMode::AcceptEdits => "Accept Edits",
            PermissionMode::Auto => "Auto",
            PermissionMode::Plan => "Plan",
            PermissionMode::BypassPermissions => "Bypass Permissions",
            PermissionMode::DontAsk => "Don't Ask",
            PermissionMode::Bubble => "Bubble",
        }
    }

    /// Returns true if this is a user-addressable external mode.
    pub fn is_external(&self) -> bool {
        matches!(
            self,
            PermissionMode::Default
                | PermissionMode::AcceptEdits
                | PermissionMode::Plan
                | PermissionMode::BypassPermissions
                | PermissionMode::DontAsk
        )
    }
}

// ============================================================================
// Permission Behaviors
// ============================================================================

/// The three possible permission behaviors.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
}

// ============================================================================
// Permission Rule Source
// ============================================================================

/// Where a permission rule originated from.
/// Maps to the TypeScript PermissionRuleSource union.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionRuleSource {
    UserSettings,
    ProjectSettings,
    LocalSettings,
    FlagSettings,
    PolicySettings,
    CliArg,
    Command,
    Session,
}

impl PermissionRuleSource {
    /// Display name (lowercase) for this source, matching TS getSettingSourceDisplayNameLowercase.
    pub fn display_name(&self) -> &'static str {
        match self {
            PermissionRuleSource::UserSettings => "user settings",
            PermissionRuleSource::ProjectSettings => "project settings",
            PermissionRuleSource::LocalSettings => "local settings",
            PermissionRuleSource::FlagSettings => "flag settings",
            PermissionRuleSource::PolicySettings => "policy settings",
            PermissionRuleSource::CliArg => "--allowed-tools",
            PermissionRuleSource::Command => "command",
            PermissionRuleSource::Session => "session",
        }
    }

    /// Whether this source is a valid destination for permission updates.
    /// Sources like FlagSettings, PolicySettings, and Command are read-only.
    pub fn is_update_destination(&self) -> bool {
        matches!(
            self,
            PermissionRuleSource::UserSettings
                | PermissionRuleSource::ProjectSettings
                | PermissionRuleSource::LocalSettings
                | PermissionRuleSource::Session
                | PermissionRuleSource::CliArg
        )
    }

    /// All sources in the canonical order used for rule iteration.
    pub fn all_sources() -> &'static [PermissionRuleSource] {
        &[
            PermissionRuleSource::UserSettings,
            PermissionRuleSource::ProjectSettings,
            PermissionRuleSource::LocalSettings,
            PermissionRuleSource::FlagSettings,
            PermissionRuleSource::PolicySettings,
            PermissionRuleSource::CliArg,
            PermissionRuleSource::Command,
            PermissionRuleSource::Session,
        ]
    }

    /// The setting sources (disk-based) subset.
    pub fn setting_sources() -> &'static [PermissionRuleSource] {
        &[
            PermissionRuleSource::UserSettings,
            PermissionRuleSource::ProjectSettings,
            PermissionRuleSource::LocalSettings,
            PermissionRuleSource::FlagSettings,
            PermissionRuleSource::PolicySettings,
        ]
    }
}

// ============================================================================
// Permission Rule Value
// ============================================================================

/// The value of a permission rule -- specifies which tool and optional content.
/// Matches TS PermissionRuleValue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRuleValue {
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_content: Option<String>,
}

impl PermissionRuleValue {
    /// Parse a rule string like "Bash" or "Bash(npm install)" into a PermissionRuleValue.
    /// Handles escaped parentheses in content.
    pub fn from_string(rule_string: &str) -> Self {
        let open_paren = find_first_unescaped_char(rule_string, '(');
        if open_paren.is_none() {
            return PermissionRuleValue {
                tool_name: normalize_legacy_tool_name(rule_string),
                rule_content: None,
            };
        }
        let open_idx = open_paren.unwrap();

        let close_paren = find_last_unescaped_char(rule_string, ')');
        if close_paren.is_none() {
            return PermissionRuleValue {
                tool_name: normalize_legacy_tool_name(rule_string),
                rule_content: None,
            };
        }
        let close_idx = close_paren.unwrap();

        if close_idx <= open_idx {
            return PermissionRuleValue {
                tool_name: normalize_legacy_tool_name(rule_string),
                rule_content: None,
            };
        }

        // Closing paren must be at end
        if close_idx != rule_string.len() - 1 {
            return PermissionRuleValue {
                tool_name: normalize_legacy_tool_name(rule_string),
                rule_content: None,
            };
        }

        let tool_name = &rule_string[..open_idx];
        let raw_content = &rule_string[open_idx + 1..close_idx];

        // Missing tool name is malformed
        if tool_name.is_empty() {
            return PermissionRuleValue {
                tool_name: normalize_legacy_tool_name(rule_string),
                rule_content: None,
            };
        }

        // Empty content or standalone wildcard => tool-wide rule
        if raw_content.is_empty() || raw_content == "*" {
            return PermissionRuleValue {
                tool_name: normalize_legacy_tool_name(tool_name),
                rule_content: None,
            };
        }

        let rule_content = unescape_rule_content(raw_content);
        PermissionRuleValue {
            tool_name: normalize_legacy_tool_name(tool_name),
            rule_content: Some(rule_content),
        }
    }

    /// Convert back to the string representation "ToolName" or "ToolName(escaped_content)".
    pub fn to_rule_string(&self) -> String {
        match &self.rule_content {
            None => self.tool_name.clone(),
            Some(content) => {
                let escaped = escape_rule_content(content);
                format!("{}({})", self.tool_name, escaped)
            }
        }
    }

    /// Display string for user-facing output: "ToolName(*)" or "ToolName(content)".
    pub fn display_string(&self) -> String {
        match &self.rule_content {
            None => format!("{}(*)", self.tool_name),
            Some(content) => format!("{}({})", self.tool_name, content),
        }
    }
}

// ============================================================================
// Permission Rule
// ============================================================================

/// A permission rule with its source and behavior.
/// Matches TS PermissionRule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRule {
    pub source: PermissionRuleSource,
    pub rule_behavior: PermissionBehavior,
    pub rule_value: PermissionRuleValue,
}

// ============================================================================
// Permission Decisions & Results
// ============================================================================

/// Explanation of why a permission decision was made.
/// Matches TS PermissionDecisionReason union.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PermissionDecisionReason {
    /// Decision was made by a matching permission rule.
    Rule {
        rule: PermissionRule,
    },
    /// Decision was made by the current permission mode.
    Mode {
        mode: PermissionMode,
    },
    /// Decision was made by evaluating subcommand results (e.g., compound bash).
    SubcommandResults {
        /// Map of subcommand string -> its permission result.
        reasons: HashMap<String, PermissionResult>,
    },
    /// Decision was made by a permission prompt tool.
    PermissionPromptTool {
        permission_prompt_tool_name: String,
    },
    /// Decision was made by a hook.
    Hook {
        hook_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        hook_source: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Decision was made for an async agent that cannot prompt.
    AsyncAgent {
        reason: String,
    },
    /// Decision due to sandbox override.
    SandboxOverride {
        /// Either "excludedCommand" or "dangerouslyDisableSandbox"
        reason: String,
    },
    /// Decision was made by an AI classifier (auto mode).
    Classifier {
        classifier: String,
        reason: String,
    },
    /// Decision related to working directory restrictions.
    WorkingDir {
        reason: String,
    },
    /// Safety check decision (dangerous files, Windows path bypass, etc.).
    SafetyCheck {
        reason: String,
        /// When true, auto mode lets the classifier evaluate instead of forcing a prompt.
        classifier_approvable: bool,
    },
    /// Catch-all for other reasons.
    Other {
        reason: String,
    },
}

/// Result when permission is granted. Matches TS PermissionAllowDecision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionAllowDecision {
    /// Updated input after permission processing, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<serde_json::Value>,
    /// Whether the user modified the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_modified: Option<bool>,
    /// Why permission was granted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_reason: Option<PermissionDecisionReason>,
    /// Tool use ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// Feedback text on accept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept_feedback: Option<String>,
}

/// Result when user should be prompted. Matches TS PermissionAskDecision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionAskDecision {
    /// Human-readable message explaining why permission is needed.
    pub message: String,
    /// Updated input after permission processing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<serde_json::Value>,
    /// Why this ask was triggered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_reason: Option<PermissionDecisionReason>,
    /// Suggested permission updates the user could apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestions: Option<Vec<PermissionUpdate>>,
    /// Path that was blocked, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_path: Option<String>,
    /// Whether this ask was from a bash security check for misparsing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_bash_security_check_for_misparsing: Option<bool>,
}

/// Result when permission is denied. Matches TS PermissionDenyDecision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionDenyDecision {
    /// Human-readable denial message.
    pub message: String,
    /// Why permission was denied.
    pub decision_reason: PermissionDecisionReason,
    /// Tool use ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
}

/// A permission decision -- allow, ask, or deny.
/// Matches TS PermissionDecision<Input>.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "behavior", rename_all = "camelCase")]
pub enum PermissionDecision {
    Allow(PermissionAllowDecision),
    Ask(PermissionAskDecision),
    Deny(PermissionDenyDecision),
}

impl PermissionDecision {
    /// Quick constructors matching the TS pattern.
    pub fn allow() -> Self {
        PermissionDecision::Allow(PermissionAllowDecision {
            updated_input: None,
            user_modified: None,
            decision_reason: None,
            tool_use_id: None,
            accept_feedback: None,
        })
    }

    pub fn allow_with_reason(reason: PermissionDecisionReason) -> Self {
        PermissionDecision::Allow(PermissionAllowDecision {
            updated_input: None,
            user_modified: None,
            decision_reason: Some(reason),
            tool_use_id: None,
            accept_feedback: None,
        })
    }

    pub fn allow_with_input(
        input: serde_json::Value,
        reason: PermissionDecisionReason,
    ) -> Self {
        PermissionDecision::Allow(PermissionAllowDecision {
            updated_input: Some(input),
            user_modified: None,
            decision_reason: Some(reason),
            tool_use_id: None,
            accept_feedback: None,
        })
    }

    pub fn deny(message: impl Into<String>, reason: PermissionDecisionReason) -> Self {
        PermissionDecision::Deny(PermissionDenyDecision {
            message: message.into(),
            decision_reason: reason,
            tool_use_id: None,
        })
    }

    pub fn ask(message: impl Into<String>) -> Self {
        PermissionDecision::Ask(PermissionAskDecision {
            message: message.into(),
            updated_input: None,
            decision_reason: None,
            suggestions: None,
            blocked_path: None,
            is_bash_security_check_for_misparsing: None,
        })
    }

    pub fn ask_with_reason(
        message: impl Into<String>,
        reason: PermissionDecisionReason,
    ) -> Self {
        PermissionDecision::Ask(PermissionAskDecision {
            message: message.into(),
            updated_input: None,
            decision_reason: Some(reason),
            suggestions: None,
            blocked_path: None,
            is_bash_security_check_for_misparsing: None,
        })
    }

    pub fn ask_with_suggestions(
        message: impl Into<String>,
        reason: PermissionDecisionReason,
        suggestions: Vec<PermissionUpdate>,
    ) -> Self {
        PermissionDecision::Ask(PermissionAskDecision {
            message: message.into(),
            updated_input: None,
            decision_reason: Some(reason),
            suggestions: Some(suggestions),
            blocked_path: None,
            is_bash_security_check_for_misparsing: None,
        })
    }

    /// Get the behavior string.
    pub fn behavior(&self) -> &'static str {
        match self {
            PermissionDecision::Allow(_) => "allow",
            PermissionDecision::Ask(_) => "ask",
            PermissionDecision::Deny(_) => "deny",
        }
    }

    /// Get the decision reason, if any.
    pub fn decision_reason(&self) -> Option<&PermissionDecisionReason> {
        match self {
            PermissionDecision::Allow(d) => d.decision_reason.as_ref(),
            PermissionDecision::Ask(d) => d.decision_reason.as_ref(),
            PermissionDecision::Deny(d) => Some(&d.decision_reason),
        }
    }

    pub fn is_allow(&self) -> bool {
        matches!(self, PermissionDecision::Allow(_))
    }
    pub fn is_ask(&self) -> bool {
        matches!(self, PermissionDecision::Ask(_))
    }
    pub fn is_deny(&self) -> bool {
        matches!(self, PermissionDecision::Deny(_))
    }
}

/// Permission result with additional passthrough option.
/// Matches TS PermissionResult<Input>.
///
/// Passthrough means "I have no opinion, fall through to the next check".
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "behavior", rename_all = "camelCase")]
pub enum PermissionResult {
    Allow(PermissionAllowDecision),
    Ask(PermissionAskDecision),
    Deny(PermissionDenyDecision),
    Passthrough {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        decision_reason: Option<PermissionDecisionReason>,
        #[serde(skip_serializing_if = "Option::is_none")]
        suggestions: Option<Vec<PermissionUpdate>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        blocked_path: Option<String>,
    },
}

impl PermissionResult {
    pub fn passthrough(message: impl Into<String>) -> Self {
        PermissionResult::Passthrough {
            message: message.into(),
            decision_reason: None,
            suggestions: None,
            blocked_path: None,
        }
    }

    pub fn is_allow(&self) -> bool {
        matches!(self, PermissionResult::Allow(_))
    }
    pub fn is_ask(&self) -> bool {
        matches!(self, PermissionResult::Ask(_))
    }
    pub fn is_deny(&self) -> bool {
        matches!(self, PermissionResult::Deny(_))
    }
    pub fn is_passthrough(&self) -> bool {
        matches!(self, PermissionResult::Passthrough { .. })
    }

    /// Convert to PermissionDecision, turning Passthrough into Ask.
    pub fn into_decision(self, tool_name: &str) -> PermissionDecision {
        match self {
            PermissionResult::Allow(d) => PermissionDecision::Allow(d),
            PermissionResult::Ask(d) => PermissionDecision::Ask(d),
            PermissionResult::Deny(d) => PermissionDecision::Deny(d),
            PermissionResult::Passthrough {
                decision_reason,
                suggestions,
                ..
            } => {
                let msg = create_permission_request_message(tool_name, decision_reason.as_ref());
                PermissionDecision::Ask(PermissionAskDecision {
                    message: msg,
                    updated_input: None,
                    decision_reason,
                    suggestions,
                    blocked_path: None,
                    is_bash_security_check_for_misparsing: None,
                })
            }
        }
    }

    /// Get the decision reason, if any.
    pub fn decision_reason(&self) -> Option<&PermissionDecisionReason> {
        match self {
            PermissionResult::Allow(d) => d.decision_reason.as_ref(),
            PermissionResult::Ask(d) => d.decision_reason.as_ref(),
            PermissionResult::Deny(d) => Some(&d.decision_reason),
            PermissionResult::Passthrough {
                decision_reason, ..
            } => decision_reason.as_ref(),
        }
    }

    /// Get the suggestions, if any.
    pub fn suggestions(&self) -> Option<&Vec<PermissionUpdate>> {
        match self {
            PermissionResult::Ask(d) => d.suggestions.as_ref(),
            PermissionResult::Passthrough { suggestions, .. } => suggestions.as_ref(),
            _ => None,
        }
    }
}

// ============================================================================
// Permission Updates
// ============================================================================

/// Update operations for permission configuration.
/// Matches the TS PermissionUpdate discriminated union.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PermissionUpdate {
    AddRules {
        destination: PermissionRuleSource,
        rules: Vec<PermissionRuleValue>,
        behavior: PermissionBehavior,
    },
    ReplaceRules {
        destination: PermissionRuleSource,
        rules: Vec<PermissionRuleValue>,
        behavior: PermissionBehavior,
    },
    RemoveRules {
        destination: PermissionRuleSource,
        rules: Vec<PermissionRuleValue>,
        behavior: PermissionBehavior,
    },
    SetMode {
        destination: PermissionRuleSource,
        mode: PermissionMode,
    },
    AddDirectories {
        destination: PermissionRuleSource,
        directories: Vec<String>,
    },
    RemoveDirectories {
        destination: PermissionRuleSource,
        directories: Vec<String>,
    },
}

// ============================================================================
// Additional Working Directory
// ============================================================================

/// An additional directory included in permission scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdditionalWorkingDirectory {
    pub path: String,
    pub source: PermissionRuleSource,
}

// ============================================================================
// Rules By Source
// ============================================================================

/// Mapping of permission rules (as strings) grouped by their source.
pub type ToolPermissionRulesBySource = HashMap<PermissionRuleSource, Vec<String>>;

// ============================================================================
// Tool Permission Context
// ============================================================================

/// Context needed for permission checking in tools.
/// Matches TS ToolPermissionContext.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPermissionContext {
    pub mode: PermissionMode,
    pub additional_working_directories: HashMap<String, AdditionalWorkingDirectory>,
    pub always_allow_rules: ToolPermissionRulesBySource,
    pub always_deny_rules: ToolPermissionRulesBySource,
    pub always_ask_rules: ToolPermissionRulesBySource,
    pub is_bypass_permissions_mode_available: bool,
    /// Dangerous rules that were stripped when entering auto mode.
    pub stripped_dangerous_rules: Option<ToolPermissionRulesBySource>,
    /// Whether to avoid permission prompts (headless agents).
    pub should_avoid_permission_prompts: bool,
    /// Whether auto mode is available (gate check).
    pub is_auto_mode_available: Option<bool>,
    /// The mode before entering plan mode (for restoration).
    pub pre_plan_mode: Option<PermissionMode>,
    /// Working directory for the project.
    pub working_directory: PathBuf,
}

impl ToolPermissionContext {
    /// Get rules by source for a given behavior.
    pub fn rules_for_behavior(&self, behavior: &PermissionBehavior) -> &ToolPermissionRulesBySource {
        match behavior {
            PermissionBehavior::Allow => &self.always_allow_rules,
            PermissionBehavior::Deny => &self.always_deny_rules,
            PermissionBehavior::Ask => &self.always_ask_rules,
        }
    }

    /// Get mutable rules by source for a given behavior.
    pub fn rules_for_behavior_mut(
        &mut self,
        behavior: &PermissionBehavior,
    ) -> &mut ToolPermissionRulesBySource {
        match behavior {
            PermissionBehavior::Allow => &mut self.always_allow_rules,
            PermissionBehavior::Deny => &mut self.always_deny_rules,
            PermissionBehavior::Ask => &mut self.always_ask_rules,
        }
    }

    /// `true` iff `rule` appears in any source entry of
    /// `always_allow_rules`. Convenience lookup.
    pub fn is_always_allowed(&self, rule: &str) -> bool {
        self.always_allow_rules
            .values()
            .any(|rules| rules.iter().any(|r| r == rule))
    }

    /// Same for `always_deny_rules`.
    pub fn is_always_denied(&self, rule: &str) -> bool {
        self.always_deny_rules
            .values()
            .any(|rules| rules.iter().any(|r| r == rule))
    }

    /// Same for `always_ask_rules`.
    pub fn is_always_ask(&self, rule: &str) -> bool {
        self.always_ask_rules
            .values()
            .any(|rules| rules.iter().any(|r| r == rule))
    }

    /// Record an additional working directory under `path`,
    /// overwriting any prior entry for that path. Matches TS
    /// `Map.set(path, { path, source })` semantics.
    pub fn add_working_directory(&mut self, path: String, source: PermissionRuleSource) {
        self.additional_working_directories.insert(
            path.clone(),
            AdditionalWorkingDirectory { path, source },
        );
    }
}

// ============================================================================
// Dangerous Permission Info
// ============================================================================

/// Information about a dangerous permission rule that would bypass classifier safety.
/// Matches TS DangerousPermissionInfo.
#[derive(Clone, Debug)]
pub struct DangerousPermissionInfo {
    pub rule_value: PermissionRuleValue,
    pub source: PermissionRuleSource,
    /// The permission rule formatted for display, e.g. "Bash(*)" or "Bash(python:*)"
    pub rule_display: String,
    /// The source formatted for display, e.g. a file path or "--allowed-tools"
    pub source_display: String,
}

// ============================================================================
// Helper Functions: Rule Parsing
// ============================================================================

/// Maps legacy tool names to their canonical names.
/// When a tool is renamed, add old -> new here.
pub fn normalize_legacy_tool_name(name: &str) -> String {
    match name {
        "Task" => "Agent".to_string(),
        "KillShell" => "TaskStop".to_string(),
        "AgentOutputTool" => "TaskOutput".to_string(),
        "BashOutputTool" => "TaskOutput".to_string(),
        _ => name.to_string(),
    }
}

/// Get all legacy names that map to a given canonical name.
pub fn get_legacy_tool_names(canonical_name: &str) -> Vec<&'static str> {
    let mut result = Vec::new();
    let aliases: &[(&str, &str)] = &[
        ("Task", "Agent"),
        ("KillShell", "TaskStop"),
        ("AgentOutputTool", "TaskOutput"),
        ("BashOutputTool", "TaskOutput"),
    ];
    for (legacy, canonical) in aliases {
        if *canonical == canonical_name {
            result.push(*legacy);
        }
    }
    result
}

/// Escape special characters in rule content for safe storage.
/// Escapes backslashes then parentheses.
pub fn escape_rule_content(content: &str) -> String {
    content
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// Unescape special characters from rule content.
/// Reverses escape_rule_content: backslashes first, then parens.
pub fn unescape_rule_content(content: &str) -> String {
    content
        .replace("\\\\", "\\")
        .replace("\\(", "(")
        .replace("\\)", ")")
}

/// Find the first unescaped occurrence of `ch` in `s`.
/// A character is escaped if preceded by an odd number of backslashes.
fn find_first_unescaped_char(s: &str, ch: char) -> Option<usize> {
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == ch as u8 {
            let mut backslash_count = 0usize;
            let mut j = i;
            while j > 0 {
                j -= 1;
                if bytes[j] == b'\\' {
                    backslash_count += 1;
                } else {
                    break;
                }
            }
            if backslash_count % 2 == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Find the last unescaped occurrence of `ch` in `s`.
fn find_last_unescaped_char(s: &str, ch: char) -> Option<usize> {
    let bytes = s.as_bytes();
    for i in (0..bytes.len()).rev() {
        if bytes[i] == ch as u8 {
            let mut backslash_count = 0usize;
            let mut j = i;
            while j > 0 {
                j -= 1;
                if bytes[j] == b'\\' {
                    backslash_count += 1;
                } else {
                    break;
                }
            }
            if backslash_count % 2 == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Create a human-readable message explaining why permission was requested.
/// Matches TS createPermissionRequestMessage.
pub fn create_permission_request_message(
    tool_name: &str,
    decision_reason: Option<&PermissionDecisionReason>,
) -> String {
    if let Some(reason) = decision_reason {
        match reason {
            PermissionDecisionReason::Classifier {
                classifier, reason, ..
            } => {
                format!(
                    "Classifier '{}' requires approval for this {} command: {}",
                    classifier, tool_name, reason
                )
            }
            PermissionDecisionReason::Hook {
                hook_name, reason, ..
            } => match reason {
                Some(r) => format!("Hook '{}' blocked this action: {}", hook_name, r),
                None => format!(
                    "Hook '{}' requires approval for this {} command",
                    hook_name, tool_name
                ),
            },
            PermissionDecisionReason::Rule { rule } => {
                let rule_string = rule.rule_value.to_rule_string();
                let source_string = rule.source.display_name();
                format!(
                    "Permission rule '{}' from {} requires approval for this {} command",
                    rule_string, source_string, tool_name
                )
            }
            PermissionDecisionReason::SubcommandResults { reasons } => {
                let needs_approval: Vec<&String> = reasons
                    .iter()
                    .filter(|(_, result)| result.is_ask() || result.is_passthrough())
                    .map(|(cmd, _)| cmd)
                    .collect();
                if !needs_approval.is_empty() {
                    let n = needs_approval.len();
                    let parts_word = if n == 1 { "part" } else { "parts" };
                    let requires_word = if n == 1 { "requires" } else { "require" };
                    let cmds: Vec<&str> = needs_approval.iter().map(|s| s.as_str()).collect();
                    format!(
                        "This {} command contains multiple operations. The following {} {} approval: {}",
                        tool_name, parts_word, requires_word, cmds.join(", ")
                    )
                } else {
                    format!(
                        "This {} command contains multiple operations that require approval",
                        tool_name
                    )
                }
            }
            PermissionDecisionReason::PermissionPromptTool {
                permission_prompt_tool_name,
            } => {
                format!(
                    "Tool '{}' requires approval for this {} command",
                    permission_prompt_tool_name, tool_name
                )
            }
            PermissionDecisionReason::SandboxOverride { .. } => {
                "Run outside of the sandbox".to_string()
            }
            PermissionDecisionReason::WorkingDir { reason } => reason.clone(),
            PermissionDecisionReason::SafetyCheck { reason, .. } => reason.clone(),
            PermissionDecisionReason::Other { reason } => reason.clone(),
            PermissionDecisionReason::Mode { mode } => {
                format!(
                    "Current permission mode ({}) requires approval for this {} command",
                    mode.title(),
                    tool_name
                )
            }
            PermissionDecisionReason::AsyncAgent { reason } => reason.clone(),
        }
    } else {
        format!(
            "Claude requested permissions to use {}, but you haven't granted it yet.",
            tool_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_rule_value_from_string_simple() {
        let v = PermissionRuleValue::from_string("Bash");
        assert_eq!(v.tool_name, "Bash");
        assert!(v.rule_content.is_none());
    }

    #[test]
    fn test_permission_rule_value_from_string_with_content() {
        let v = PermissionRuleValue::from_string("Bash(npm install)");
        assert_eq!(v.tool_name, "Bash");
        assert_eq!(v.rule_content, Some("npm install".to_string()));
    }

    #[test]
    fn test_permission_rule_value_from_string_wildcard() {
        let v = PermissionRuleValue::from_string("Bash(*)");
        assert_eq!(v.tool_name, "Bash");
        assert!(v.rule_content.is_none()); // * => tool-wide
    }

    #[test]
    fn test_permission_rule_value_from_string_empty_parens() {
        let v = PermissionRuleValue::from_string("Bash()");
        assert_eq!(v.tool_name, "Bash");
        assert!(v.rule_content.is_none()); // empty => tool-wide
    }

    #[test]
    fn test_permission_rule_value_from_string_escaped() {
        let v = PermissionRuleValue::from_string(r#"Bash(python -c \"print\(1\)\")"#);
        assert_eq!(v.tool_name, "Bash");
        // The unescaped content should have real parens and quotes
        assert!(v.rule_content.is_some());
    }

    #[test]
    fn test_permission_rule_value_to_string() {
        let v = PermissionRuleValue {
            tool_name: "Bash".to_string(),
            rule_content: Some("npm install".to_string()),
        };
        assert_eq!(v.to_rule_string(), "Bash(npm install)");
    }

    #[test]
    fn test_permission_rule_value_to_string_no_content() {
        let v = PermissionRuleValue {
            tool_name: "Bash".to_string(),
            rule_content: None,
        };
        assert_eq!(v.to_rule_string(), "Bash");
    }

    #[test]
    fn test_legacy_tool_name_normalization() {
        assert_eq!(normalize_legacy_tool_name("Task"), "Agent");
        assert_eq!(normalize_legacy_tool_name("Bash"), "Bash");
        assert_eq!(normalize_legacy_tool_name("KillShell"), "TaskStop");
    }

    #[test]
    fn test_permission_mode_from_string() {
        assert_eq!(PermissionMode::from_string("default"), PermissionMode::Default);
        assert_eq!(
            PermissionMode::from_string("acceptEdits"),
            PermissionMode::AcceptEdits
        );
        assert_eq!(PermissionMode::from_string("auto"), PermissionMode::Auto);
        assert_eq!(
            PermissionMode::from_string("bypassPermissions"),
            PermissionMode::BypassPermissions
        );
        assert_eq!(PermissionMode::from_string("dontAsk"), PermissionMode::DontAsk);
        assert_eq!(PermissionMode::from_string("plan"), PermissionMode::Plan);
        assert_eq!(PermissionMode::from_string("bubble"), PermissionMode::Bubble);
        assert_eq!(PermissionMode::from_string("unknown"), PermissionMode::Default);
    }

    #[test]
    fn test_escape_unescape_roundtrip() {
        let original = r#"python -c "print(1)""#;
        let escaped = escape_rule_content(original);
        let unescaped = unescape_rule_content(&escaped);
        assert_eq!(original, unescaped);
    }
}
