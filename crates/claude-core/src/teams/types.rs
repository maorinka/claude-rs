use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Backend types
// ---------------------------------------------------------------------------

/// Types of backends available for teammate execution.
///  - `Tmux`: Uses tmux for pane management (works in tmux or standalone).
///  - `ITerm2`: Uses iTerm2 native split panes via the it2 CLI.
///  - `InProcess`: Runs teammate in the same process with isolated context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendType {
    Tmux,
    #[serde(rename = "iterm2")]
    ITerm2,
    InProcess,
}

impl BackendType {
    /// Whether this backend type uses terminal panes.
    pub fn is_pane_backend(&self) -> bool {
        matches!(self, BackendType::Tmux | BackendType::ITerm2)
    }

    /// Human-readable display name for this backend.
    pub fn display_name(&self) -> &'static str {
        match self {
            BackendType::Tmux => "tmux",
            BackendType::ITerm2 => "iTerm2",
            BackendType::InProcess => "in-process",
        }
    }
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendType::Tmux => write!(f, "tmux"),
            BackendType::ITerm2 => write!(f, "iterm2"),
            BackendType::InProcess => write!(f, "in-process"),
        }
    }
}

// ---------------------------------------------------------------------------
// Spawn mode
// ---------------------------------------------------------------------------

/// How a teammate is displayed / laid out in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnMode {
    /// Split-pane view (leader on left, teammates on right).
    SplitPane,
    /// Each teammate in its own tmux window.
    SeparateWindow,
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

/// Agent color names used for pane borders and UI differentiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentColor {
    Red,
    Blue,
    Green,
    Yellow,
    Purple,
    Orange,
    Pink,
    Cyan,
}

impl AgentColor {
    /// Return the tmux-compatible color string for this agent color.
    pub fn tmux_color(&self) -> &'static str {
        match self {
            AgentColor::Red => "red",
            AgentColor::Blue => "blue",
            AgentColor::Green => "green",
            AgentColor::Yellow => "yellow",
            AgentColor::Purple => "magenta",
            AgentColor::Orange => "colour208",
            AgentColor::Pink => "colour205",
            AgentColor::Cyan => "cyan",
        }
    }

    /// Color palette for assignment rotation.
    pub const ALL: &'static [AgentColor] = &[
        AgentColor::Red,
        AgentColor::Blue,
        AgentColor::Green,
        AgentColor::Yellow,
        AgentColor::Purple,
        AgentColor::Orange,
        AgentColor::Pink,
        AgentColor::Cyan,
    ];
}

impl std::fmt::Display for AgentColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AgentColor::Red => "red",
            AgentColor::Blue => "blue",
            AgentColor::Green => "green",
            AgentColor::Yellow => "yellow",
            AgentColor::Purple => "purple",
            AgentColor::Orange => "orange",
            AgentColor::Pink => "pink",
            AgentColor::Cyan => "cyan",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for AgentColor {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "red" => Ok(AgentColor::Red),
            "blue" => Ok(AgentColor::Blue),
            "green" => Ok(AgentColor::Green),
            "yellow" => Ok(AgentColor::Yellow),
            "purple" | "magenta" => Ok(AgentColor::Purple),
            "orange" => Ok(AgentColor::Orange),
            "pink" => Ok(AgentColor::Pink),
            "cyan" => Ok(AgentColor::Cyan),
            _ => Err(format!("unknown agent color: {}", s)),
        }
    }
}

// ---------------------------------------------------------------------------
// Teammate identity
// ---------------------------------------------------------------------------

/// Identity fields for a teammate. Subset shared with `TeammateSpawnConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateIdentity {
    /// Agent name (e.g. "researcher", "tester").
    pub name: String,
    /// Team name this teammate belongs to.
    pub team_name: String,
    /// Assigned color for UI differentiation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<AgentColor>,
    /// Whether plan mode approval is required before implementation.
    #[serde(default)]
    pub plan_mode_required: bool,
}

// ---------------------------------------------------------------------------
// Teammate spawn configuration
// ---------------------------------------------------------------------------

/// Configuration for spawning a teammate (any execution mode).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateSpawnConfig {
    // -- identity fields --
    /// Agent name (e.g. "researcher", "tester").
    pub name: String,
    /// Team name this teammate belongs to.
    pub team_name: String,
    /// Assigned color for UI differentiation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<AgentColor>,
    /// Whether plan mode approval is required before implementation.
    #[serde(default)]
    pub plan_mode_required: bool,

    // -- spawn-specific fields --
    /// Initial prompt to send to the teammate.
    pub prompt: String,
    /// Working directory for the teammate.
    pub cwd: String,
    /// Model to use for this teammate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// System prompt for this teammate (resolved from workflow config).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// How to apply the system prompt: "default", "replace", or "append".
    #[serde(default = "default_system_prompt_mode")]
    pub system_prompt_mode: SystemPromptMode,
    /// Optional git worktree path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    /// Parent session ID (for context linking).
    pub parent_session_id: String,
    /// Tool permissions to grant this teammate.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Whether this teammate can show permission prompts for unlisted tools.
    /// When false (default), unlisted tools are auto-denied.
    #[serde(default)]
    pub allow_permission_prompts: bool,
    /// Optional agent type identifier (e.g. "worker", custom agent name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Optional description for the task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// System prompt application mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SystemPromptMode {
    Default,
    Replace,
    Append,
}

fn default_system_prompt_mode() -> SystemPromptMode {
    SystemPromptMode::Default
}

// ---------------------------------------------------------------------------
// Teammate spawn result
// ---------------------------------------------------------------------------

/// Opaque pane identifier (tmux pane ID such as "%1", or iTerm2 session ID).
pub type PaneId = String;

/// Result from spawning a teammate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateSpawnResult {
    /// Whether spawn was successful.
    pub success: bool,
    /// Unique agent ID (format: `agentName@teamName`).
    pub agent_id: String,
    /// Error message if spawn failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Task ID in state (in-process only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Pane ID (pane-based only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<PaneId>,
}

// ---------------------------------------------------------------------------
// Create pane result
// ---------------------------------------------------------------------------

/// Result of creating a new teammate pane.
#[derive(Debug, Clone)]
pub struct CreatePaneResult {
    /// The pane ID for the newly created pane.
    pub pane_id: PaneId,
    /// Whether this is the first teammate pane (affects layout strategy).
    pub is_first_teammate: bool,
}

// ---------------------------------------------------------------------------
// Spawn output (the full data returned to callers)
// ---------------------------------------------------------------------------

/// Full output from a teammate spawn operation, matching the TS `SpawnOutput`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnOutput {
    pub teammate_id: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub tmux_session_name: String,
    pub tmux_window_name: String,
    pub tmux_pane_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    #[serde(default)]
    pub is_splitpane: bool,
    #[serde(default)]
    pub plan_mode_required: bool,
}

// ---------------------------------------------------------------------------
// Spawn teammate configuration (input from the tool layer)
// ---------------------------------------------------------------------------

/// Input configuration for spawning a teammate from the tool layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnTeammateConfig {
    pub name: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub use_splitpane: bool,
    #[serde(default)]
    pub plan_mode_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Backend detection result
// ---------------------------------------------------------------------------

/// Result from backend detection.
#[derive(Debug, Clone)]
pub struct BackendDetectionResult {
    /// The backend that should be used.
    pub backend_type: BackendType,
    /// Whether we're running inside the backend's native environment.
    pub is_native: bool,
    /// If iTerm2 is detected but it2 not installed, this will be true.
    pub needs_it2_setup: bool,
}

// ---------------------------------------------------------------------------
// Teammate message (for executor interface)
// ---------------------------------------------------------------------------

/// Message to send to a teammate (used by `TeammateExecutor`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateExecMessage {
    /// Message content.
    pub text: String,
    /// Sender agent ID.
    pub from: String,
    /// Sender display color.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Message timestamp (ISO string).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// 5-10 word summary shown as preview in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

// ---------------------------------------------------------------------------
// PaneBackend trait
// ---------------------------------------------------------------------------

/// Interface for pane management backends. Abstracts operations for creating
/// and managing terminal panes for teammate visualization in swarm mode.
#[async_trait::async_trait]
pub trait PaneBackend: Send + Sync {
    /// The type identifier for this backend.
    fn backend_type(&self) -> BackendType;

    /// Human-readable display name.
    fn display_name(&self) -> &str;

    /// Whether this backend supports hiding and showing panes.
    fn supports_hide_show(&self) -> bool;

    /// Checks if this backend is available on the system.
    async fn is_available(&self) -> bool;

    /// Checks if we're currently running inside this backend's environment.
    async fn is_running_inside(&self) -> bool;

    /// Creates a new pane for a teammate in the swarm view.
    async fn create_teammate_pane_in_swarm_view(
        &self,
        name: &str,
        color: AgentColor,
    ) -> anyhow::Result<CreatePaneResult>;

    /// Sends a command to execute in a specific pane.
    async fn send_command_to_pane(
        &self,
        pane_id: &str,
        command: &str,
        use_external_session: bool,
    ) -> anyhow::Result<()>;

    /// Sets the border color for a pane.
    async fn set_pane_border_color(
        &self,
        pane_id: &str,
        color: AgentColor,
        use_external_session: bool,
    ) -> anyhow::Result<()>;

    /// Sets the title for a pane.
    async fn set_pane_title(
        &self,
        pane_id: &str,
        name: &str,
        color: AgentColor,
        use_external_session: bool,
    ) -> anyhow::Result<()>;

    /// Enables pane border status display.
    async fn enable_pane_border_status(
        &self,
        window_target: Option<&str>,
        use_external_session: bool,
    ) -> anyhow::Result<()>;

    /// Rebalances panes to achieve the desired layout.
    async fn rebalance_panes(
        &self,
        window_target: &str,
        has_leader: bool,
    ) -> anyhow::Result<()>;

    /// Kills/closes a specific pane.
    async fn kill_pane(
        &self,
        pane_id: &str,
        use_external_session: bool,
    ) -> bool;

    /// Hides a pane by breaking it out into a hidden window.
    async fn hide_pane(
        &self,
        pane_id: &str,
        use_external_session: bool,
    ) -> bool;

    /// Shows a previously hidden pane by joining it back into the main window.
    async fn show_pane(
        &self,
        pane_id: &str,
        target_window_or_pane: &str,
        use_external_session: bool,
    ) -> bool;
}

// ---------------------------------------------------------------------------
// TeammateExecutor trait
// ---------------------------------------------------------------------------

/// Common interface for teammate execution backends. Abstracts the differences
/// between pane-based (tmux/iTerm2) and in-process execution.
#[async_trait::async_trait]
pub trait TeammateExecutor: Send + Sync {
    /// Backend type identifier.
    fn backend_type(&self) -> BackendType;

    /// Check if this executor is available on the system.
    async fn is_available(&self) -> bool;

    /// Spawn a new teammate with the given configuration.
    async fn spawn(&self, config: &TeammateSpawnConfig) -> anyhow::Result<TeammateSpawnResult>;

    /// Send a message to a teammate.
    async fn send_message(&self, agent_id: &str, message: &TeammateExecMessage) -> anyhow::Result<()>;

    /// Terminate a teammate (graceful shutdown request).
    async fn terminate(&self, agent_id: &str, reason: Option<&str>) -> bool;

    /// Force kill a teammate (immediate termination).
    async fn kill(&self, agent_id: &str) -> bool;

    /// Check if a teammate is still active.
    async fn is_active(&self, agent_id: &str) -> bool;
}

// ---------------------------------------------------------------------------
// Legacy types kept for backward compat with existing coordinator.rs
// ---------------------------------------------------------------------------

/// Overall status of a team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamStatus {
    Active,
    Stopped,
}

/// Per-agent lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// A single agent that belongs to a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamAgent {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub model: Option<String>,
    /// OS process ID once the agent has been spawned.
    pub pid: Option<u32>,
    pub status: AgentStatus,
}

/// A named group of coordinated agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub agents: Vec<TeamAgent>,
    pub status: TeamStatus,
}

/// Team file member entry (matches the TS team file JSON schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamFileMember {
    pub agent_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub plan_mode_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joined_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tmux_pane_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub subscriptions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_type: Option<BackendType>,
}

/// Team file stored at `~/.claude/teams/{team}/team.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamFile {
    pub team_name: String,
    pub lead_agent_id: String,
    pub members: Vec<TeamFileMember>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Permission mode — re-exported from the canonical permissions module.
// ---------------------------------------------------------------------------

pub use crate::permissions::types::PermissionMode;
