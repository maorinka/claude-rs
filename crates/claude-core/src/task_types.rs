//! Task state taxonomy.
//!
//! Port of `src/Task.ts` + `src/tasks/types.ts` + the per-task-type
//! `*State` shapes. TS ships polymorphic task registry machinery
//! (spawn/kill/render dispatch, AppState integration, forked-agent
//! invocation) across ~2K LOC. This module ports the data shapes —
//! enough for callers to write task-aware code that reads the state,
//! without pulling in the full orchestration layer.
//!
//! The actual task runtime (spawn, kill, progress updates) still lives
//! in claude_tools::task_tools today with a narrower shape. When that
//! integrates with this enum, remove the inline duplication there.

use std::time::Duration;

// ── Core taxonomy ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskType {
    LocalBash,
    LocalAgent,
    RemoteAgent,
    InProcessTeammate,
    LocalWorkflow,
    MonitorMcp,
    Dream,
}

impl TaskType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskType::LocalBash => "local_bash",
            TaskType::LocalAgent => "local_agent",
            TaskType::RemoteAgent => "remote_agent",
            TaskType::InProcessTeammate => "in_process_teammate",
            TaskType::LocalWorkflow => "local_workflow",
            TaskType::MonitorMcp => "monitor_mcp",
            TaskType::Dream => "dream",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Killed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Killed => "killed",
        }
    }

    /// True when the task won't transition further — used to guard
    /// orphan-cleanup and message-injection paths. Matches TS
    /// `isTerminalTaskStatus`.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Killed
        )
    }
}

/// Fields shared by every task state variant. Matches TS `TaskStateBase`.
#[derive(Debug, Clone)]
pub struct TaskStateBase {
    pub id: String,
    pub task_type: TaskType,
    pub status: TaskStatus,
    pub description: String,
    pub tool_use_id: Option<String>,
    pub start_time_ms: u64,
    pub end_time_ms: Option<u64>,
    pub total_paused_ms: Option<u64>,
    pub output_file: String,
    pub output_offset: u64,
    pub notified: bool,
}

// ── Per-task state ───────────────────────────────────────────────────────

/// Local `bash`-spawned command. Covers foreground bash invocations
/// that get backgrounded via Ctrl+B.
#[derive(Debug, Clone)]
pub struct LocalShellTaskState {
    pub base: TaskStateBase,
    pub command: String,
    pub pid: Option<u32>,
    pub is_backgrounded: bool,
    pub kind: LocalShellKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalShellKind {
    /// Plain bash command.
    Bash,
    /// MCP server monitor with its own UI affordances.
    Monitor,
}

/// A subprocess agent (`Agent` tool with `run_in_background=true`).
#[derive(Debug, Clone)]
pub struct LocalAgentTaskState {
    pub base: TaskStateBase,
    pub subagent_type: String,
    pub prompt: String,
    pub pid: Option<u32>,
    pub is_backgrounded: bool,
    pub worktree_path: Option<String>,
}

/// A remote agent dispatched via the `RemoteTrigger` tool.
#[derive(Debug, Clone)]
pub struct RemoteAgentTaskState {
    pub base: TaskStateBase,
    /// Remote task id returned by the remote API.
    pub remote_task_id: String,
    pub subagent_type: String,
    pub prompt: String,
    pub ingress_url: Option<String>,
}

/// An in-process teammate (coordinator mode worker).
#[derive(Debug, Clone)]
pub struct InProcessTeammateTaskState {
    pub base: TaskStateBase,
    pub agent_id: String,
    pub prompt: String,
}

/// A bundled workflow script (`WORKFLOW_SCRIPTS` feature).
#[derive(Debug, Clone)]
pub struct LocalWorkflowTaskState {
    pub base: TaskStateBase,
    pub workflow_name: String,
    pub arguments: Vec<String>,
    pub pid: Option<u32>,
}

/// A watcher listening to MCP server events via stdio/SSE.
#[derive(Debug, Clone)]
pub struct MonitorMcpTaskState {
    pub base: TaskStateBase,
    pub server_name: String,
    pub connection_id: String,
}

/// A background memory-consolidation (auto-dream) pass. Gets surfaced
/// in the task pill so the user can see it running.
#[derive(Debug, Clone)]
pub struct DreamTaskState {
    pub base: TaskStateBase,
    pub phase: DreamPhase,
    pub sessions_reviewing: u32,
    /// Paths observed in Edit/Write tool_use blocks. Incomplete —
    /// misses bash-mediated writes. Matches TS comment semantics.
    pub files_touched: Vec<String>,
    /// Assistant turns, tool-uses collapsed to a count.
    pub turns: Vec<DreamTurn>,
    /// Prior consolidation-lock mtime, stashed so kill can rewind.
    pub prior_mtime_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DreamPhase {
    Starting,
    Updating,
}

#[derive(Debug, Clone)]
pub struct DreamTurn {
    pub text: String,
    pub tool_use_count: u32,
}

/// Keep only the N most recent turns for live display. Matches TS.
pub const DREAM_MAX_TURNS: usize = 30;

// ── Union ────────────────────────────────────────────────────────────────

/// Discriminated union of every task-state variant, matching TS
/// `TaskState`. Callers use the embedded `base` for common fields.
#[derive(Debug, Clone)]
pub enum TaskState {
    LocalShell(LocalShellTaskState),
    LocalAgent(LocalAgentTaskState),
    RemoteAgent(RemoteAgentTaskState),
    InProcessTeammate(InProcessTeammateTaskState),
    LocalWorkflow(LocalWorkflowTaskState),
    MonitorMcp(MonitorMcpTaskState),
    Dream(DreamTaskState),
}

impl TaskState {
    pub fn base(&self) -> &TaskStateBase {
        match self {
            TaskState::LocalShell(t) => &t.base,
            TaskState::LocalAgent(t) => &t.base,
            TaskState::RemoteAgent(t) => &t.base,
            TaskState::InProcessTeammate(t) => &t.base,
            TaskState::LocalWorkflow(t) => &t.base,
            TaskState::MonitorMcp(t) => &t.base,
            TaskState::Dream(t) => &t.base,
        }
    }

    pub fn status(&self) -> TaskStatus {
        self.base().status
    }

    pub fn task_type(&self) -> TaskType {
        self.base().task_type
    }

    /// Is this task a foreground task? Matches TS's
    /// `isBackgrounded === false` check — only the two task types with
    /// a foreground/background distinction carry that bit.
    pub fn is_foreground(&self) -> bool {
        match self {
            TaskState::LocalShell(t) => !t.is_backgrounded,
            TaskState::LocalAgent(t) => !t.is_backgrounded,
            _ => false,
        }
    }

    /// Should this task show up in the background-tasks indicator?
    /// Matches TS `isBackgroundTask`: running or pending AND not
    /// explicitly foregrounded.
    pub fn is_background_task(&self) -> bool {
        if !matches!(self.status(), TaskStatus::Running | TaskStatus::Pending) {
            return false;
        }
        !self.is_foreground()
    }
}

/// Elapsed wall-clock duration excluding any paused intervals.
/// Convenience matching TS `taskDuration`.
pub fn task_duration(base: &TaskStateBase, now_ms: u64) -> Duration {
    let end = base.end_time_ms.unwrap_or(now_ms);
    let raw = end.saturating_sub(base.start_time_ms);
    let paused = base.total_paused_ms.unwrap_or(0);
    Duration::from_millis(raw.saturating_sub(paused))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(task_type: TaskType, status: TaskStatus) -> TaskStateBase {
        TaskStateBase {
            id: "t".into(),
            task_type,
            status,
            description: "desc".into(),
            tool_use_id: None,
            start_time_ms: 0,
            end_time_ms: None,
            total_paused_ms: None,
            output_file: String::new(),
            output_offset: 0,
            notified: false,
        }
    }

    #[test]
    fn task_status_terminal_matches_ts_set() {
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Killed.is_terminal());
        assert!(!TaskStatus::Pending.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
    }

    #[test]
    fn as_str_round_trips_taxonomy() {
        for t in &[
            TaskType::LocalBash,
            TaskType::LocalAgent,
            TaskType::RemoteAgent,
            TaskType::InProcessTeammate,
            TaskType::LocalWorkflow,
            TaskType::MonitorMcp,
            TaskType::Dream,
        ] {
            let s = t.as_str();
            assert!(!s.is_empty());
            assert!(s.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }

    #[test]
    fn foreground_local_shell_not_background() {
        let t = TaskState::LocalShell(LocalShellTaskState {
            base: base(TaskType::LocalBash, TaskStatus::Running),
            command: "sleep 5".into(),
            pid: Some(42),
            is_backgrounded: false,
            kind: LocalShellKind::Bash,
        });
        assert!(t.is_foreground());
        assert!(!t.is_background_task());
    }

    #[test]
    fn backgrounded_local_shell_is_background() {
        let t = TaskState::LocalShell(LocalShellTaskState {
            base: base(TaskType::LocalBash, TaskStatus::Running),
            command: "long-running".into(),
            pid: Some(42),
            is_backgrounded: true,
            kind: LocalShellKind::Bash,
        });
        assert!(!t.is_foreground());
        assert!(t.is_background_task());
    }

    #[test]
    fn completed_task_not_in_background_indicator() {
        let t = TaskState::LocalAgent(LocalAgentTaskState {
            base: base(TaskType::LocalAgent, TaskStatus::Completed),
            subagent_type: "worker".into(),
            prompt: "do thing".into(),
            pid: None,
            is_backgrounded: true,
            worktree_path: None,
        });
        assert!(!t.is_background_task());
    }

    #[test]
    fn dream_task_without_fg_bit_is_background() {
        let t = TaskState::Dream(DreamTaskState {
            base: base(TaskType::Dream, TaskStatus::Running),
            phase: DreamPhase::Starting,
            sessions_reviewing: 1,
            files_touched: Vec::new(),
            turns: Vec::new(),
            prior_mtime_ms: 0,
        });
        assert!(!t.is_foreground());
        assert!(t.is_background_task());
    }

    #[test]
    fn base_accessor_returns_inner() {
        let t = TaskState::RemoteAgent(RemoteAgentTaskState {
            base: base(TaskType::RemoteAgent, TaskStatus::Pending),
            remote_task_id: "r-1".into(),
            subagent_type: "worker".into(),
            prompt: "..".into(),
            ingress_url: None,
        });
        assert_eq!(t.base().task_type, TaskType::RemoteAgent);
        assert_eq!(t.status(), TaskStatus::Pending);
    }

    #[test]
    fn task_duration_subtracts_paused() {
        let mut b = base(TaskType::LocalBash, TaskStatus::Running);
        b.start_time_ms = 1_000;
        b.end_time_ms = Some(6_000);
        b.total_paused_ms = Some(500);
        let d = task_duration(&b, 6_000);
        assert_eq!(d, Duration::from_millis(4_500));
    }
}
