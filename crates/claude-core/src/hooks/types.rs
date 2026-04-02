use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Hook Events — all 27 events from the TypeScript HOOK_EVENTS array
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Notification,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Stop,
    StopFailure,
    SubagentStart,
    SubagentStop,
    PreCompact,
    PostCompact,
    PermissionRequest,
    PermissionDenied,
    Setup,
    TeammateIdle,
    TaskCreated,
    TaskCompleted,
    Elicitation,
    ElicitationResult,
    ConfigChange,
    WorktreeCreate,
    WorktreeRemove,
    InstructionsLoaded,
    CwdChanged,
    FileChanged,
}

impl HookEvent {
    /// Parse a string into a HookEvent, returning None if invalid.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "PreToolUse" => Some(Self::PreToolUse),
            "PostToolUse" => Some(Self::PostToolUse),
            "PostToolUseFailure" => Some(Self::PostToolUseFailure),
            "Notification" => Some(Self::Notification),
            "UserPromptSubmit" => Some(Self::UserPromptSubmit),
            "SessionStart" => Some(Self::SessionStart),
            "SessionEnd" => Some(Self::SessionEnd),
            "Stop" => Some(Self::Stop),
            "StopFailure" => Some(Self::StopFailure),
            "SubagentStart" => Some(Self::SubagentStart),
            "SubagentStop" => Some(Self::SubagentStop),
            "PreCompact" => Some(Self::PreCompact),
            "PostCompact" => Some(Self::PostCompact),
            "PermissionRequest" => Some(Self::PermissionRequest),
            "PermissionDenied" => Some(Self::PermissionDenied),
            "Setup" => Some(Self::Setup),
            "TeammateIdle" => Some(Self::TeammateIdle),
            "TaskCreated" => Some(Self::TaskCreated),
            "TaskCompleted" => Some(Self::TaskCompleted),
            "Elicitation" => Some(Self::Elicitation),
            "ElicitationResult" => Some(Self::ElicitationResult),
            "ConfigChange" => Some(Self::ConfigChange),
            "WorktreeCreate" => Some(Self::WorktreeCreate),
            "WorktreeRemove" => Some(Self::WorktreeRemove),
            "InstructionsLoaded" => Some(Self::InstructionsLoaded),
            "CwdChanged" => Some(Self::CwdChanged),
            "FileChanged" => Some(Self::FileChanged),
            _ => None,
        }
    }

    /// Return the canonical string name for this event.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::Notification => "Notification",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
            Self::Stop => "Stop",
            Self::StopFailure => "StopFailure",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::PreCompact => "PreCompact",
            Self::PostCompact => "PostCompact",
            Self::PermissionRequest => "PermissionRequest",
            Self::PermissionDenied => "PermissionDenied",
            Self::Setup => "Setup",
            Self::TeammateIdle => "TeammateIdle",
            Self::TaskCreated => "TaskCreated",
            Self::TaskCompleted => "TaskCompleted",
            Self::Elicitation => "Elicitation",
            Self::ElicitationResult => "ElicitationResult",
            Self::ConfigChange => "ConfigChange",
            Self::WorktreeCreate => "WorktreeCreate",
            Self::WorktreeRemove => "WorktreeRemove",
            Self::InstructionsLoaded => "InstructionsLoaded",
            Self::CwdChanged => "CwdChanged",
            Self::FileChanged => "FileChanged",
        }
    }
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// Shell types
// ============================================================================

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShellType {
    Bash,
    #[serde(rename = "powershell")]
    PowerShell,
}

impl Default for ShellType {
    fn default() -> Self {
        Self::Bash
    }
}

// ============================================================================
// HookCommand — discriminated union of the 4 command types
// ============================================================================

/// A hook command that can be persisted in settings.json.
/// Corresponds to the TypeScript HookCommand discriminated union.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum HookCommand {
    /// Shell command hook — spawns a shell process.
    #[serde(rename = "command")]
    Command(CommandHook),

    /// LLM prompt hook — evaluates a prompt with a model.
    #[serde(rename = "prompt")]
    Prompt(PromptHook),

    /// HTTP hook — POSTs input to a URL and parses the JSON response.
    #[serde(rename = "http")]
    Http(HttpHook),

    /// Agent hook — runs a subagent to verify a condition.
    #[serde(rename = "agent")]
    Agent(AgentHook),
}

impl HookCommand {
    /// Returns a human-readable display string for the hook (used in progress/error messages).
    pub fn display_text(&self) -> String {
        match self {
            HookCommand::Command(h) => h.command.clone(),
            HookCommand::Prompt(h) => {
                let truncated = if h.prompt.len() > 60 {
                    format!("{}...", &h.prompt[..57])
                } else {
                    h.prompt.clone()
                };
                format!("prompt: {}", truncated)
            }
            HookCommand::Http(h) => format!("http: {}", h.url),
            HookCommand::Agent(h) => {
                let truncated = if h.prompt.len() > 60 {
                    format!("{}...", &h.prompt[..57])
                } else {
                    h.prompt.clone()
                };
                format!("agent: {}", truncated)
            }
        }
    }

    /// Returns the `if` condition if present on the hook.
    pub fn if_condition(&self) -> Option<&str> {
        match self {
            HookCommand::Command(h) => h.if_condition.as_deref(),
            HookCommand::Prompt(h) => h.if_condition.as_deref(),
            HookCommand::Http(h) => h.if_condition.as_deref(),
            HookCommand::Agent(h) => h.if_condition.as_deref(),
        }
    }

    /// Returns the per-hook timeout in seconds, if configured.
    pub fn timeout_secs(&self) -> Option<f64> {
        match self {
            HookCommand::Command(h) => h.timeout,
            HookCommand::Prompt(h) => h.timeout,
            HookCommand::Http(h) => h.timeout,
            HookCommand::Agent(h) => h.timeout,
        }
    }

    /// Returns the optional status message for spinner display.
    pub fn status_message(&self) -> Option<&str> {
        match self {
            HookCommand::Command(h) => h.status_message.as_deref(),
            HookCommand::Prompt(h) => h.status_message.as_deref(),
            HookCommand::Http(h) => h.status_message.as_deref(),
            HookCommand::Agent(h) => h.status_message.as_deref(),
        }
    }

    /// Whether this is a "once" hook (runs once then removed).
    pub fn is_once(&self) -> bool {
        match self {
            HookCommand::Command(h) => h.once.unwrap_or(false),
            HookCommand::Prompt(h) => h.once.unwrap_or(false),
            HookCommand::Http(h) => h.once.unwrap_or(false),
            HookCommand::Agent(h) => h.once.unwrap_or(false),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommandHook {
    /// Shell command to execute.
    pub command: String,

    /// Permission rule syntax filter (e.g., "Bash(git *)").
    #[serde(rename = "if")]
    pub if_condition: Option<String>,

    /// Shell interpreter: "bash" or "powershell". Defaults to bash.
    pub shell: Option<ShellType>,

    /// Per-hook timeout in seconds.
    pub timeout: Option<f64>,

    /// Custom spinner status message.
    #[serde(rename = "statusMessage")]
    pub status_message: Option<String>,

    /// If true, hook runs once and is removed.
    pub once: Option<bool>,

    /// If true, hook runs in background without blocking.
    #[serde(rename = "async")]
    pub is_async: Option<bool>,

    /// If true, runs in background; on exit code 2, wakes the model.
    #[serde(rename = "asyncRewake")]
    pub async_rewake: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptHook {
    /// Prompt to evaluate. Use $ARGUMENTS for hook input JSON.
    pub prompt: String,

    /// Permission rule syntax filter.
    #[serde(rename = "if")]
    pub if_condition: Option<String>,

    /// Per-hook timeout in seconds.
    pub timeout: Option<f64>,

    /// Model to use (e.g., "claude-sonnet-4-6").
    pub model: Option<String>,

    /// Custom spinner status message.
    #[serde(rename = "statusMessage")]
    pub status_message: Option<String>,

    /// If true, hook runs once and is removed.
    pub once: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HttpHook {
    /// URL to POST the hook input JSON to.
    pub url: String,

    /// Permission rule syntax filter.
    #[serde(rename = "if")]
    pub if_condition: Option<String>,

    /// Per-hook timeout in seconds.
    pub timeout: Option<f64>,

    /// Additional HTTP headers. Values may reference env vars ($VAR_NAME).
    pub headers: Option<HashMap<String, String>>,

    /// Env var names allowed for header interpolation.
    #[serde(rename = "allowedEnvVars")]
    pub allowed_env_vars: Option<Vec<String>>,

    /// Custom spinner status message.
    #[serde(rename = "statusMessage")]
    pub status_message: Option<String>,

    /// If true, hook runs once and is removed.
    pub once: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentHook {
    /// Prompt describing what to verify. Use $ARGUMENTS for hook input JSON.
    pub prompt: String,

    /// Permission rule syntax filter.
    #[serde(rename = "if")]
    pub if_condition: Option<String>,

    /// Per-hook timeout in seconds (default 60).
    pub timeout: Option<f64>,

    /// Model to use (e.g., "claude-sonnet-4-6").
    pub model: Option<String>,

    /// Custom spinner status message.
    #[serde(rename = "statusMessage")]
    pub status_message: Option<String>,

    /// If true, hook runs once and is removed.
    pub once: Option<bool>,
}

// ============================================================================
// HookMatcher — pairs a pattern with hooks to run
// ============================================================================

/// A matcher configuration with multiple hooks.
/// Corresponds to the TypeScript HookMatcher type.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HookMatcher {
    /// String pattern to match (e.g. tool names like "Write").
    /// If absent or empty, matches everything.
    pub matcher: Option<String>,

    /// List of hooks to execute when the matcher matches.
    pub hooks: Vec<HookCommand>,
}

// ============================================================================
// HooksSettings — the full hooks config (event -> matchers)
// ============================================================================

/// Hooks configuration: maps each event to an array of matchers.
/// Not all events need to be defined.
pub type HooksSettings = HashMap<HookEvent, Vec<HookMatcher>>;

// ============================================================================
// Hook execution results
// ============================================================================

/// The outcome of a single hook execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookOutcome {
    Success,
    Blocking,
    NonBlockingError,
    Cancelled,
}

impl std::fmt::Display for HookOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "success"),
            Self::Blocking => write!(f, "blocking"),
            Self::NonBlockingError => write!(f, "non_blocking_error"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// A blocking error produced by a hook.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookBlockingError {
    /// The error message text.
    pub blocking_error: String,
    /// The command string that produced the error.
    pub command: String,
}

/// Permission behavior that a hook can express.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
    Passthrough,
}

/// Result from a PermissionRequest hook decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "behavior")]
pub enum PermissionRequestResult {
    #[serde(rename = "allow")]
    Allow {
        #[serde(rename = "updatedInput")]
        updated_input: Option<HashMap<String, serde_json::Value>>,
        #[serde(rename = "updatedPermissions")]
        updated_permissions: Option<Vec<serde_json::Value>>,
    },
    #[serde(rename = "deny")]
    Deny {
        message: Option<String>,
        interrupt: Option<bool>,
    },
}

/// Result of a single hook execution.
#[derive(Clone, Debug)]
pub struct HookResult {
    /// Outcome of this hook execution.
    pub outcome: HookOutcome,

    /// Blocking error, if the hook produced one (exit code 2 or JSON decision:block).
    pub blocking_error: Option<HookBlockingError>,

    /// Whether to prevent the model from continuing.
    pub prevent_continuation: Option<bool>,

    /// Reason for stopping continuation.
    pub stop_reason: Option<String>,

    /// Permission behavior the hook expressed (allow/deny/ask/passthrough).
    pub permission_behavior: Option<PermissionBehavior>,

    /// Human-readable reason for the permission decision.
    pub hook_permission_decision_reason: Option<String>,

    /// Additional context to inject into the conversation.
    pub additional_context: Option<String>,

    /// Override for the initial user message (SessionStart).
    pub initial_user_message: Option<String>,

    /// Modified tool input (from PreToolUse hooks).
    pub updated_input: Option<HashMap<String, serde_json::Value>>,

    /// Modified MCP tool output (from PostToolUse hooks).
    pub updated_mcp_tool_output: Option<serde_json::Value>,

    /// PermissionRequest hook decision.
    pub permission_request_result: Option<PermissionRequestResult>,

    /// Whether to retry (from PermissionDenied hooks).
    pub retry: Option<bool>,

    /// System message from JSON output.
    pub system_message: Option<String>,

    /// Absolute paths to watch for FileChanged hooks.
    pub watch_paths: Option<Vec<String>>,

    /// Stdout from the hook (for display/logging).
    pub stdout: String,

    /// Stderr from the hook.
    pub stderr: String,

    /// Exit code.
    pub exit_code: Option<i32>,

    /// Duration of execution in milliseconds.
    pub duration_ms: Option<u64>,

    /// The command display text for this hook.
    pub command_display: String,
}

impl Default for HookResult {
    fn default() -> Self {
        Self {
            outcome: HookOutcome::Success,
            blocking_error: None,
            prevent_continuation: None,
            stop_reason: None,
            permission_behavior: None,
            hook_permission_decision_reason: None,
            additional_context: None,
            initial_user_message: None,
            updated_input: None,
            updated_mcp_tool_output: None,
            permission_request_result: None,
            retry: None,
            system_message: None,
            watch_paths: None,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            duration_ms: None,
            command_display: String::new(),
        }
    }
}

/// Aggregated result from all hooks for a given event.
#[derive(Clone, Debug, Default)]
pub struct AggregatedHookResult {
    /// Blocking errors from all hooks.
    pub blocking_errors: Vec<HookBlockingError>,

    /// Whether any hook requested to prevent continuation.
    pub prevent_continuation: bool,

    /// The stop reason, if any hook provided one.
    pub stop_reason: Option<String>,

    /// Aggregated permission behavior (deny > ask > allow > passthrough).
    pub permission_behavior: Option<PermissionBehavior>,

    /// Reason for the aggregated permission decision.
    pub hook_permission_decision_reason: Option<String>,

    /// Source of the hook that determined the permission behavior.
    pub hook_source: Option<String>,

    /// Additional contexts from all hooks (collected, not deduplicated).
    pub additional_contexts: Vec<String>,

    /// Override for the initial user message (last one wins).
    pub initial_user_message: Option<String>,

    /// Modified tool input (last one wins).
    pub updated_input: Option<HashMap<String, serde_json::Value>>,

    /// Modified MCP tool output (last one wins).
    pub updated_mcp_tool_output: Option<serde_json::Value>,

    /// PermissionRequest hook decision (last one wins).
    pub permission_request_result: Option<PermissionRequestResult>,

    /// Whether to retry (from PermissionDenied hooks).
    pub retry: Option<bool>,

    /// Absolute paths to watch for FileChanged hooks (accumulated).
    pub watch_paths: Vec<String>,

    /// Individual hook results (for detailed inspection).
    pub individual_results: Vec<HookResult>,
}

impl AggregatedHookResult {
    /// Returns true if any hook produced a blocking error.
    pub fn has_blocking_errors(&self) -> bool {
        !self.blocking_errors.is_empty()
    }
}

// ============================================================================
// Sync hook JSON output — the JSON a hook emits to stdout / HTTP body
// ============================================================================

/// The JSON output format from synchronous hooks.
/// Validated against a Zod schema in the TypeScript implementation.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SyncHookJsonOutput {
    /// Whether Claude should continue after hook (default: true).
    #[serde(rename = "continue")]
    pub should_continue: Option<bool>,

    /// Hide stdout from transcript (default: false).
    #[serde(rename = "suppressOutput")]
    pub suppress_output: Option<bool>,

    /// Message shown when continue is false.
    #[serde(rename = "stopReason")]
    pub stop_reason: Option<String>,

    /// "approve" or "block".
    pub decision: Option<String>,

    /// Explanation for the decision.
    pub reason: Option<String>,

    /// Warning message shown to the user.
    #[serde(rename = "systemMessage")]
    pub system_message: Option<String>,

    /// Event-specific output fields.
    #[serde(rename = "hookSpecificOutput")]
    pub hook_specific_output: Option<HookSpecificOutput>,
}

/// Async hook JSON output — tells the runner to background the process.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AsyncHookJsonOutput {
    /// Must be true.
    #[serde(rename = "async")]
    pub is_async: bool,

    /// Optional timeout override for the async hook.
    #[serde(rename = "asyncTimeout")]
    pub async_timeout: Option<u64>,
}

/// Parsed hook JSON output (either sync or async).
#[derive(Clone, Debug)]
pub enum HookJsonOutput {
    Sync(SyncHookJsonOutput),
    Async(AsyncHookJsonOutput),
}

/// Hook-specific output, keyed by the hookEventName field.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "hookEventName")]
pub enum HookSpecificOutput {
    PreToolUse {
        #[serde(rename = "permissionDecision")]
        permission_decision: Option<String>,
        #[serde(rename = "permissionDecisionReason")]
        permission_decision_reason: Option<String>,
        #[serde(rename = "updatedInput")]
        updated_input: Option<HashMap<String, serde_json::Value>>,
        #[serde(rename = "additionalContext")]
        additional_context: Option<String>,
    },
    UserPromptSubmit {
        #[serde(rename = "additionalContext")]
        additional_context: Option<String>,
    },
    SessionStart {
        #[serde(rename = "additionalContext")]
        additional_context: Option<String>,
        #[serde(rename = "initialUserMessage")]
        initial_user_message: Option<String>,
        #[serde(rename = "watchPaths")]
        watch_paths: Option<Vec<String>>,
    },
    Setup {
        #[serde(rename = "additionalContext")]
        additional_context: Option<String>,
    },
    SubagentStart {
        #[serde(rename = "additionalContext")]
        additional_context: Option<String>,
    },
    PostToolUse {
        #[serde(rename = "additionalContext")]
        additional_context: Option<String>,
        #[serde(rename = "updatedMCPToolOutput")]
        updated_mcp_tool_output: Option<serde_json::Value>,
    },
    PostToolUseFailure {
        #[serde(rename = "additionalContext")]
        additional_context: Option<String>,
    },
    PermissionDenied {
        retry: Option<bool>,
    },
    Notification {
        #[serde(rename = "additionalContext")]
        additional_context: Option<String>,
    },
    PermissionRequest {
        decision: Option<PermissionRequestDecision>,
    },
    Elicitation {
        action: Option<String>,
        content: Option<HashMap<String, serde_json::Value>>,
    },
    ElicitationResult {
        action: Option<String>,
        content: Option<HashMap<String, serde_json::Value>>,
    },
    CwdChanged {
        #[serde(rename = "watchPaths")]
        watch_paths: Option<Vec<String>>,
    },
    FileChanged {
        #[serde(rename = "watchPaths")]
        watch_paths: Option<Vec<String>>,
    },
    WorktreeCreate {
        #[serde(rename = "worktreePath")]
        worktree_path: String,
    },
}

/// Decision payload for PermissionRequest hookSpecificOutput.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "behavior")]
pub enum PermissionRequestDecision {
    #[serde(rename = "allow")]
    Allow {
        #[serde(rename = "updatedInput")]
        updated_input: Option<HashMap<String, serde_json::Value>>,
        #[serde(rename = "updatedPermissions")]
        updated_permissions: Option<Vec<serde_json::Value>>,
    },
    #[serde(rename = "deny")]
    Deny {
        message: Option<String>,
        interrupt: Option<bool>,
    },
}

// ============================================================================
// Hook execution context
// ============================================================================

/// Base hook input fields shared across all hook events.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseHookInput {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

/// Result of running hooks outside the REPL (SessionEnd, Notification, etc.).
#[derive(Clone, Debug)]
pub struct HookOutsideReplResult {
    pub command: String,
    pub succeeded: bool,
    pub output: String,
    pub blocked: bool,
    pub watch_paths: Option<Vec<String>>,
    pub system_message: Option<String>,
}

impl HookOutsideReplResult {
    /// Returns true if this result indicates a blocking error.
    pub fn is_blocked(&self) -> bool {
        self.blocked
    }
}

/// Returns true if any of the outside-REPL results are blocked.
pub fn has_blocking_result(results: &[HookOutsideReplResult]) -> bool {
    results.iter().any(|r| r.blocked)
}

// ============================================================================
// Deprecated compat type
// ============================================================================

/// Legacy hook config for backwards compatibility during migration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HookConfig {
    pub event: HookEvent,
    pub command: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}
