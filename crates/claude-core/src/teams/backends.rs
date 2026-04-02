//! Backend implementations for teammate execution.
//!
//! Provides `TmuxBackend` (pane-based via tmux) and `InProcessBackend`
//! (same-process execution). Also contains shared utilities like model
//! resolution, unique name generation, and CLI flag building.

use std::collections::HashSet;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use tokio::process::Command;
use tracing::{debug, warn};

use super::mailbox::{
    create_shutdown_request, read_team_file, write_to_mailbox,
    TeammateMessageInput, TEAM_LEAD_NAME,
};
use super::types::*;

// ---------------------------------------------------------------------------
// Constants (mirrors TS constants.ts)
// ---------------------------------------------------------------------------

pub const SWARM_SESSION_NAME: &str = "claude-swarm";
pub const SWARM_VIEW_WINDOW_NAME: &str = "swarm-view";
pub const TMUX_COMMAND: &str = "tmux";
pub const HIDDEN_SESSION_NAME: &str = "claude-hidden";
pub const TEAMMATE_COMMAND_ENV_VAR: &str = "CLAUDE_CODE_TEAMMATE_COMMAND";
pub const TEAMMATE_COLOR_ENV_VAR: &str = "CLAUDE_CODE_AGENT_COLOR";
pub const PLAN_MODE_REQUIRED_ENV_VAR: &str = "CLAUDE_CODE_PLAN_MODE_REQUIRED";

/// Delay after pane creation to allow shell initialization (loading rc files,
/// prompts, etc.). 200ms is enough for most shell configurations.
const PANE_SHELL_INIT_DELAY_MS: u64 = 200;

/// Default hardcoded teammate model fallback.
const DEFAULT_TEAMMATE_MODEL: &str = "claude-opus-4-6-20250415";

// ---------------------------------------------------------------------------
// Tmux command helpers
// ---------------------------------------------------------------------------

/// Get the socket name for external swarm sessions.
pub fn get_swarm_socket_name() -> String {
    format!("claude-swarm-{}", std::process::id())
}

/// Run a tmux command in the user's original tmux session (no socket override).
async fn run_tmux_in_user_session(args: &[&str]) -> (String, String, i32) {
    run_command(TMUX_COMMAND, args).await
}

/// Run a tmux command in the external swarm socket.
async fn run_tmux_in_swarm(args: &[&str]) -> (String, String, i32) {
    let socket = get_swarm_socket_name();
    let mut full_args: Vec<String> = vec!["-L".to_string(), socket];
    full_args.extend(args.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = full_args.iter().map(|s| s.as_str()).collect();
    run_command(TMUX_COMMAND, &refs).await
}

/// Dispatch tmux command to the correct session.
async fn run_tmux(args: &[&str], use_external: bool) -> (String, String, i32) {
    if use_external {
        run_tmux_in_swarm(args).await
    } else {
        run_tmux_in_user_session(args).await
    }
}

/// Execute a command and return (stdout, stderr, exit_code).
async fn run_command(cmd: &str, args: &[&str]) -> (String, String, i32) {
    match Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let code = output.status.code().unwrap_or(-1);
            (stdout, stderr, code)
        }
        Err(e) => (String::new(), e.to_string(), -1),
    }
}

// ---------------------------------------------------------------------------
// TmuxBackend
// ---------------------------------------------------------------------------

/// TmuxBackend implements `PaneBackend` using tmux for pane management.
///
/// When running INSIDE tmux (leader is in tmux):
/// - Splits the current window to add teammates alongside the leader.
/// - Leader stays on left (30%), teammates on right (70%).
///
/// When running OUTSIDE tmux (leader is in regular terminal):
/// - Creates a `claude-swarm` session with a `swarm-view` window.
/// - All teammates are equally distributed (no leader pane).
pub struct TmuxBackend {
    /// Tracks whether the first pane has been used for external swarm session.
    first_pane_used: AtomicBool,
}

impl TmuxBackend {
    pub fn new() -> Self {
        Self {
            first_pane_used: AtomicBool::new(false),
        }
    }

    /// Checks if tmux is installed and available.
    pub async fn is_available() -> bool {
        let (_, _, code) = run_command("which", &[TMUX_COMMAND]).await;
        code == 0
    }

    /// Checks if we're currently running inside a tmux session.
    pub fn is_inside_tmux() -> bool {
        std::env::var("TMUX").is_ok()
    }

    /// Get the leader's pane ID from TMUX_PANE env var.
    fn get_leader_pane_id() -> Option<String> {
        std::env::var("TMUX_PANE").ok()
    }

    /// Get the current window target (session:window format).
    async fn get_current_window_target() -> Option<String> {
        let leader_pane = Self::get_leader_pane_id();
        let mut args = vec!["display-message"];
        let pane_str;
        if let Some(ref pane) = leader_pane {
            pane_str = pane.clone();
            args.push("-t");
            args.push(&pane_str);
        }
        args.push("-p");
        args.push("#{session_name}:#{window_index}");

        let (stdout, _, code) = run_tmux_in_user_session(&args).await;
        if code != 0 {
            return None;
        }
        Some(stdout.trim().to_string())
    }

    /// Get number of panes in a window.
    async fn get_pane_count(window_target: &str, use_swarm_socket: bool) -> Option<usize> {
        let args = vec!["list-panes", "-t", window_target, "-F", "#{pane_id}"];
        let (stdout, _, code) = if use_swarm_socket {
            run_tmux_in_swarm(&args.iter().map(|s| *s).collect::<Vec<_>>()).await
        } else {
            run_tmux_in_user_session(&args.iter().map(|s| *s).collect::<Vec<_>>()).await
        };
        if code != 0 {
            return None;
        }
        Some(stdout.trim().lines().filter(|l| !l.is_empty()).count())
    }

    /// Check if a tmux session exists in the swarm socket.
    async fn has_session_in_swarm(session_name: &str) -> bool {
        let (_, _, code) = run_tmux_in_swarm(&["has-session", "-t", session_name]).await;
        code == 0
    }

    /// Create the external swarm session with a single window.
    async fn create_external_swarm_session() -> Result<(String, String)> {
        let session_exists = Self::has_session_in_swarm(SWARM_SESSION_NAME).await;

        if !session_exists {
            let (stdout, stderr, code) = run_tmux_in_swarm(&[
                "new-session",
                "-d",
                "-s",
                SWARM_SESSION_NAME,
                "-n",
                SWARM_VIEW_WINDOW_NAME,
                "-P",
                "-F",
                "#{pane_id}",
            ])
            .await;

            if code != 0 {
                return Err(anyhow::anyhow!(
                    "Failed to create swarm session: {}",
                    if stderr.is_empty() {
                        "Unknown error"
                    } else {
                        &stderr
                    }
                ));
            }

            let pane_id = stdout.trim().to_string();
            let window_target = format!("{}:{}", SWARM_SESSION_NAME, SWARM_VIEW_WINDOW_NAME);

            debug!(
                "[TmuxBackend] Created external swarm session with window {}, pane {}",
                window_target, pane_id
            );
            return Ok((window_target, pane_id));
        }

        // Session exists, check if swarm-view window exists.
        let (stdout, _, _) = run_tmux_in_swarm(&[
            "list-windows",
            "-t",
            SWARM_SESSION_NAME,
            "-F",
            "#{window_name}",
        ])
        .await;

        let windows: Vec<&str> = stdout.trim().lines().filter(|l| !l.is_empty()).collect();
        let window_target = format!("{}:{}", SWARM_SESSION_NAME, SWARM_VIEW_WINDOW_NAME);

        if windows.contains(&SWARM_VIEW_WINDOW_NAME) {
            let (pane_stdout, _, _) = run_tmux_in_swarm(&[
                "list-panes",
                "-t",
                &window_target,
                "-F",
                "#{pane_id}",
            ])
            .await;
            let panes: Vec<&str> = pane_stdout.trim().lines().filter(|l| !l.is_empty()).collect();
            return Ok((window_target, panes.first().unwrap_or(&"").to_string()));
        }

        // Create the swarm-view window.
        let (stdout, stderr, code) = run_tmux_in_swarm(&[
            "new-window",
            "-t",
            SWARM_SESSION_NAME,
            "-n",
            SWARM_VIEW_WINDOW_NAME,
            "-P",
            "-F",
            "#{pane_id}",
        ])
        .await;

        if code != 0 {
            return Err(anyhow::anyhow!(
                "Failed to create swarm-view window: {}",
                if stderr.is_empty() {
                    "Unknown error"
                } else {
                    &stderr
                }
            ));
        }

        Ok((window_target, stdout.trim().to_string()))
    }

    /// Create a teammate pane with leader (inside tmux).
    pub async fn create_teammate_pane_with_leader(
        name: &str,
        color: AgentColor,
    ) -> Result<CreatePaneResult> {
        let current_pane = Self::get_leader_pane_id()
            .ok_or_else(|| anyhow::anyhow!("Could not determine current tmux pane"))?;
        let window_target = Self::get_current_window_target()
            .await
            .ok_or_else(|| anyhow::anyhow!("Could not determine current window"))?;

        let pane_count = Self::get_pane_count(&window_target, false)
            .await
            .ok_or_else(|| anyhow::anyhow!("Could not determine pane count"))?;

        let is_first_teammate = pane_count == 1;

        let split_result = if is_first_teammate {
            run_tmux_in_user_session(&[
                "split-window",
                "-t",
                &current_pane,
                "-h",
                "-l",
                "70%",
                "-P",
                "-F",
                "#{pane_id}",
            ])
            .await
        } else {
            let (list_stdout, _, _) = run_tmux_in_user_session(&[
                "list-panes",
                "-t",
                &window_target,
                "-F",
                "#{pane_id}",
            ])
            .await;

            let panes: Vec<&str> = list_stdout.trim().lines().filter(|l| !l.is_empty()).collect();
            let teammate_panes = &panes[1..];
            let teammate_count = teammate_panes.len();

            let split_vertically = teammate_count % 2 == 1;
            let target_idx = ((teammate_count as isize) - 1).max(0) as usize / 2;
            let target_pane = teammate_panes
                .get(target_idx)
                .or(teammate_panes.last())
                .unwrap_or(&"");

            run_tmux_in_user_session(&[
                "split-window",
                "-t",
                target_pane,
                if split_vertically { "-v" } else { "-h" },
                "-P",
                "-F",
                "#{pane_id}",
            ])
            .await
        };

        if split_result.2 != 0 {
            return Err(anyhow::anyhow!(
                "Failed to create teammate pane: {}",
                split_result.1
            ));
        }

        let pane_id = split_result.0.trim().to_string();
        debug!(
            "[TmuxBackend] Created teammate pane for {}: {}",
            name, pane_id
        );

        // Set border color and title.
        Self::set_pane_border_color_impl(&pane_id, color, false).await;
        Self::set_pane_title_impl(&pane_id, name, color, false).await;
        Self::rebalance_with_leader(&window_target).await;

        tokio::time::sleep(Duration::from_millis(PANE_SHELL_INIT_DELAY_MS)).await;

        Ok(CreatePaneResult {
            pane_id,
            is_first_teammate,
        })
    }

    /// Create a teammate pane in external (non-tmux) mode.
    pub async fn create_teammate_pane_external(
        &self,
        name: &str,
        color: AgentColor,
    ) -> Result<CreatePaneResult> {
        let (window_target, first_pane_id) = Self::create_external_swarm_session().await?;

        let pane_count = Self::get_pane_count(&window_target, true)
            .await
            .ok_or_else(|| anyhow::anyhow!("Could not determine pane count"))?;

        let was_first = !self.first_pane_used.load(Ordering::SeqCst) && pane_count == 1;

        let pane_id = if was_first {
            self.first_pane_used.store(true, Ordering::SeqCst);
            debug!(
                "[TmuxBackend] Using initial pane for first teammate {}: {}",
                name, first_pane_id
            );
            Self::enable_pane_border_status_impl(Some(&window_target), true).await;
            first_pane_id
        } else {
            let (list_stdout, _, _) = run_tmux_in_swarm(&[
                "list-panes",
                "-t",
                &window_target,
                "-F",
                "#{pane_id}",
            ])
            .await;

            let panes: Vec<&str> = list_stdout.trim().lines().filter(|l| !l.is_empty()).collect();
            let teammate_count = panes.len();
            let split_vertically = teammate_count % 2 == 1;
            let target_idx = ((teammate_count as isize) - 1).max(0) as usize / 2;
            let target_pane = panes
                .get(target_idx)
                .or(panes.last())
                .unwrap_or(&"");

            let (stdout, stderr, code) = run_tmux_in_swarm(&[
                "split-window",
                "-t",
                target_pane,
                if split_vertically { "-v" } else { "-h" },
                "-P",
                "-F",
                "#{pane_id}",
            ])
            .await;

            if code != 0 {
                return Err(anyhow::anyhow!(
                    "Failed to create teammate pane: {}",
                    stderr
                ));
            }

            stdout.trim().to_string()
        };

        Self::set_pane_border_color_impl(&pane_id, color, true).await;
        Self::set_pane_title_impl(&pane_id, name, color, true).await;
        Self::rebalance_tiled(&window_target).await;

        tokio::time::sleep(Duration::from_millis(PANE_SHELL_INIT_DELAY_MS)).await;

        Ok(CreatePaneResult {
            pane_id,
            is_first_teammate: was_first,
        })
    }

    /// Send a command to a specific pane.
    pub async fn send_command_to_pane_impl(
        pane_id: &str,
        command: &str,
        use_external: bool,
    ) -> Result<()> {
        let (_, stderr, code) = run_tmux(&["send-keys", "-t", pane_id, command, "Enter"], use_external).await;
        if code != 0 {
            return Err(anyhow::anyhow!(
                "Failed to send command to pane {}: {}",
                pane_id,
                stderr
            ));
        }
        Ok(())
    }

    /// Set pane border color.
    async fn set_pane_border_color_impl(pane_id: &str, color: AgentColor, use_external: bool) {
        let tmux_color = color.tmux_color();
        let fg_style = format!("bg=default,fg={}", tmux_color);
        let border_style = format!("fg={}", tmux_color);

        let _ = run_tmux(&["select-pane", "-t", pane_id, "-P", &fg_style], use_external).await;
        let _ = run_tmux(&["set-option", "-p", "-t", pane_id, "pane-border-style", &border_style], use_external).await;
        let _ = run_tmux(&["set-option", "-p", "-t", pane_id, "pane-active-border-style", &border_style], use_external).await;
    }

    /// Set pane title.
    async fn set_pane_title_impl(pane_id: &str, name: &str, color: AgentColor, use_external: bool) {
        let tmux_color = color.tmux_color();
        let _ = run_tmux(&["select-pane", "-t", pane_id, "-T", name], use_external).await;
        let format_str = format!("#[fg={},bold] #{{pane_title}} #[default]", tmux_color);
        let _ = run_tmux(&["set-option", "-p", "-t", pane_id, "pane-border-format", &format_str], use_external).await;
    }

    /// Enable pane border status for a window.
    pub async fn enable_pane_border_status_impl(window_target: Option<&str>, use_external: bool) {
        let target = match window_target {
            Some(t) => t.to_string(),
            None => match Self::get_current_window_target().await {
                Some(t) => t,
                None => return,
            },
        };

        let _ = run_tmux(&["set-option", "-w", "-t", &target, "pane-border-status", "top"], use_external).await;
    }

    /// Rebalance panes with leader (main-vertical layout, leader at 30%).
    async fn rebalance_with_leader(window_target: &str) {
        let (list_stdout, _, _) = run_tmux_in_user_session(&[
            "list-panes",
            "-t",
            window_target,
            "-F",
            "#{pane_id}",
        ])
        .await;

        let panes: Vec<&str> = list_stdout.trim().lines().filter(|l| !l.is_empty()).collect();
        if panes.len() <= 2 {
            return;
        }

        let _ = run_tmux_in_user_session(&[
            "select-layout",
            "-t",
            window_target,
            "main-vertical",
        ])
        .await;

        if let Some(leader_pane) = panes.first() {
            let _ = run_tmux_in_user_session(&[
                "resize-pane",
                "-t",
                leader_pane,
                "-x",
                "30%",
            ])
            .await;
        }

        debug!(
            "[TmuxBackend] Rebalanced {} teammate panes with leader",
            panes.len() - 1
        );
    }

    /// Rebalance panes in tiled layout (no leader).
    async fn rebalance_tiled(window_target: &str) {
        let (list_stdout, _, _) = run_tmux_in_swarm(&[
            "list-panes",
            "-t",
            window_target,
            "-F",
            "#{pane_id}",
        ])
        .await;

        let panes: Vec<&str> = list_stdout.trim().lines().filter(|l| !l.is_empty()).collect();
        if panes.len() <= 1 {
            return;
        }

        let _ = run_tmux_in_swarm(&["select-layout", "-t", window_target, "tiled"]).await;

        debug!(
            "[TmuxBackend] Rebalanced {} teammate panes with tiled layout",
            panes.len()
        );
    }

    /// Kill a specific pane.
    pub async fn kill_pane_impl(pane_id: &str, use_external: bool) -> bool {
        let (_, _, code) = run_tmux(&["kill-pane", "-t", pane_id], use_external).await;
        code == 0
    }

    /// Hide a pane by moving it to a detached hidden session.
    pub async fn hide_pane_impl(pane_id: &str, use_external: bool) -> bool {
        // Create hidden session if needed.
        let _ = run_tmux(&["new-session", "-d", "-s", HIDDEN_SESSION_NAME], use_external).await;

        let target = format!("{}:", HIDDEN_SESSION_NAME);
        let (_, _, code) = run_tmux(&["break-pane", "-d", "-s", pane_id, "-t", &target], use_external).await;
        if code == 0 {
            debug!("[TmuxBackend] Hidden pane {}", pane_id);
        } else {
            debug!("[TmuxBackend] Failed to hide pane {}", pane_id);
        }
        code == 0
    }

    /// Show a previously hidden pane.
    pub async fn show_pane_impl(
        pane_id: &str,
        target_window_or_pane: &str,
        use_external: bool,
    ) -> bool {
        let (_, _, code) = run_tmux(&["join-pane", "-h", "-s", pane_id, "-t", target_window_or_pane], use_external).await;

        if code != 0 {
            debug!("[TmuxBackend] Failed to show pane {}", pane_id);
            return false;
        }

        // Reapply main-vertical layout.
        let _ = run_tmux(&["select-layout", "-t", target_window_or_pane, "main-vertical"], use_external).await;

        let (list_stdout, _, _) = run_tmux(&["list-panes", "-t", target_window_or_pane, "-F", "#{pane_id}"], use_external).await;

        let panes: Vec<&str> = list_stdout.trim().lines().filter(|l| !l.is_empty()).collect();
        if let Some(first) = panes.first() {
            let _ = run_tmux(&["resize-pane", "-t", first, "-x", "30%"], use_external).await;
        }

        true
    }
}

#[async_trait::async_trait]
impl PaneBackend for TmuxBackend {
    fn backend_type(&self) -> BackendType {
        BackendType::Tmux
    }

    fn display_name(&self) -> &str {
        "tmux"
    }

    fn supports_hide_show(&self) -> bool {
        true
    }

    async fn is_available(&self) -> bool {
        Self::is_available().await
    }

    async fn is_running_inside(&self) -> bool {
        Self::is_inside_tmux()
    }

    async fn create_teammate_pane_in_swarm_view(
        &self,
        name: &str,
        color: AgentColor,
    ) -> Result<CreatePaneResult> {
        if Self::is_inside_tmux() {
            Self::create_teammate_pane_with_leader(name, color).await
        } else {
            self.create_teammate_pane_external(name, color).await
        }
    }

    async fn send_command_to_pane(
        &self,
        pane_id: &str,
        command: &str,
        use_external_session: bool,
    ) -> Result<()> {
        Self::send_command_to_pane_impl(pane_id, command, use_external_session).await
    }

    async fn set_pane_border_color(
        &self,
        pane_id: &str,
        color: AgentColor,
        use_external_session: bool,
    ) -> Result<()> {
        Self::set_pane_border_color_impl(pane_id, color, use_external_session).await;
        Ok(())
    }

    async fn set_pane_title(
        &self,
        pane_id: &str,
        name: &str,
        color: AgentColor,
        use_external_session: bool,
    ) -> Result<()> {
        Self::set_pane_title_impl(pane_id, name, color, use_external_session).await;
        Ok(())
    }

    async fn enable_pane_border_status(
        &self,
        window_target: Option<&str>,
        use_external_session: bool,
    ) -> Result<()> {
        Self::enable_pane_border_status_impl(window_target, use_external_session).await;
        Ok(())
    }

    async fn rebalance_panes(&self, window_target: &str, has_leader: bool) -> Result<()> {
        if has_leader {
            Self::rebalance_with_leader(window_target).await;
        } else {
            Self::rebalance_tiled(window_target).await;
        }
        Ok(())
    }

    async fn kill_pane(&self, pane_id: &str, use_external_session: bool) -> bool {
        Self::kill_pane_impl(pane_id, use_external_session).await
    }

    async fn hide_pane(&self, pane_id: &str, use_external_session: bool) -> bool {
        Self::hide_pane_impl(pane_id, use_external_session).await
    }

    async fn show_pane(
        &self,
        pane_id: &str,
        target_window_or_pane: &str,
        use_external_session: bool,
    ) -> bool {
        Self::show_pane_impl(pane_id, target_window_or_pane, use_external_session).await
    }
}

// ---------------------------------------------------------------------------
// InProcessBackend
// ---------------------------------------------------------------------------

/// InProcessBackend implements `TeammateExecutor` for in-process teammates.
///
/// In-process teammates run in the same process with isolated context.
/// They communicate via file-based mailbox (same as pane-based teammates).
/// They are terminated via an abort/cancellation mechanism.
pub struct InProcessBackend {
    /// Tracks active in-process teammates by agent_id.
    active: std::sync::Mutex<std::collections::HashMap<String, InProcessTeammate>>,
}

/// State of an in-process teammate.
struct InProcessTeammate {
    #[allow(dead_code)]
    agent_id: String,
    task_id: String,
    team_name: String,
    agent_name: String,
    cancelled: AtomicBool,
}

impl InProcessBackend {
    pub fn new() -> Self {
        Self {
            active: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Register a teammate as active.
    fn register(&self, agent_id: &str, task_id: &str, team_name: &str, agent_name: &str) {
        let mut map = self.active.lock().unwrap();
        map.insert(
            agent_id.to_string(),
            InProcessTeammate {
                agent_id: agent_id.to_string(),
                task_id: task_id.to_string(),
                team_name: team_name.to_string(),
                agent_name: agent_name.to_string(),
                cancelled: AtomicBool::new(false),
            },
        );
    }

    /// Look up a teammate by agent ID.
    fn find(&self, agent_id: &str) -> Option<(String, String, String, bool)> {
        let map = self.active.lock().unwrap();
        map.get(agent_id).map(|t| {
            (
                t.task_id.clone(),
                t.team_name.clone(),
                t.agent_name.clone(),
                t.cancelled.load(Ordering::SeqCst),
            )
        })
    }

    /// Mark a teammate as cancelled.
    fn mark_cancelled(&self, agent_id: &str) -> bool {
        let map = self.active.lock().unwrap();
        if let Some(t) = map.get(agent_id) {
            t.cancelled.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Remove a teammate from the active set.
    fn remove(&self, agent_id: &str) {
        let mut map = self.active.lock().unwrap();
        map.remove(agent_id);
    }
}

#[async_trait::async_trait]
impl TeammateExecutor for InProcessBackend {
    fn backend_type(&self) -> BackendType {
        BackendType::InProcess
    }

    async fn is_available(&self) -> bool {
        // In-process backend is always available.
        true
    }

    async fn spawn(&self, config: &TeammateSpawnConfig) -> Result<TeammateSpawnResult> {
        let agent_id = format_agent_id(&config.name, &config.team_name);
        let task_id = format!("task-{}", uuid::Uuid::new_v4());

        debug!("[InProcessBackend] spawn() called for {}", config.name);

        self.register(&agent_id, &task_id, &config.team_name, &config.name);

        // Write the initial prompt to the teammate's mailbox so the inbox
        // poller picks it up on first poll.
        if let Err(e) = write_to_mailbox(
            &config.name,
            TeammateMessageInput {
                from: TEAM_LEAD_NAME.to_string(),
                text: config.prompt.clone(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                color: None,
                summary: None,
            },
            Some(&config.team_name),
        )
        .await
        {
            warn!(
                "[InProcessBackend] Failed to write initial prompt to mailbox: {}",
                e
            );
        }

        Ok(TeammateSpawnResult {
            success: true,
            agent_id,
            error: None,
            task_id: Some(task_id),
            pane_id: None,
        })
    }

    async fn send_message(&self, agent_id: &str, message: &TeammateExecMessage) -> Result<()> {
        debug!(
            "[InProcessBackend] sendMessage() to {}: {}...",
            agent_id,
            &message.text[..message.text.len().min(50)]
        );

        let (agent_name, team_name) = parse_agent_id(agent_id)
            .ok_or_else(|| anyhow::anyhow!("Invalid agentId format: {}", agent_id))?;

        write_to_mailbox(
            &agent_name,
            TeammateMessageInput {
                from: message.from.clone(),
                text: message.text.clone(),
                timestamp: message
                    .timestamp
                    .clone()
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                color: message.color.clone(),
                summary: message.summary.clone(),
            },
            Some(&team_name),
        )
        .await?;

        Ok(())
    }

    async fn terminate(&self, agent_id: &str, reason: Option<&str>) -> bool {
        debug!(
            "[InProcessBackend] terminate() called for {}: {:?}",
            agent_id, reason
        );

        let (_task_id, team_name, agent_name, already_cancelled) = match self.find(agent_id) {
            Some(t) => t,
            None => {
                debug!(
                    "[InProcessBackend] terminate() failed: not found for {}",
                    agent_id
                );
                return false;
            }
        };

        if already_cancelled {
            debug!(
                "[InProcessBackend] terminate(): already cancelled for {}",
                agent_id
            );
            return true;
        }

        let request_id = format!("shutdown-{}-{}", agent_id, chrono::Utc::now().timestamp_millis());
        let shutdown_msg = create_shutdown_request(&request_id, TEAM_LEAD_NAME, reason);

        let _ = write_to_mailbox(
            &agent_name,
            TeammateMessageInput {
                from: TEAM_LEAD_NAME.to_string(),
                text: serde_json::to_string(&shutdown_msg).unwrap_or_default(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                color: None,
                summary: None,
            },
            Some(&team_name),
        )
        .await;

        self.mark_cancelled(agent_id);
        debug!(
            "[InProcessBackend] terminate() sent shutdown request to {}",
            agent_id
        );
        true
    }

    async fn kill(&self, agent_id: &str) -> bool {
        debug!("[InProcessBackend] kill() called for {}", agent_id);

        if !self.mark_cancelled(agent_id) {
            debug!(
                "[InProcessBackend] kill() failed: not found for {}",
                agent_id
            );
            return false;
        }

        self.remove(agent_id);
        debug!("[InProcessBackend] kill() succeeded for {}", agent_id);
        true
    }

    async fn is_active(&self, agent_id: &str) -> bool {
        match self.find(agent_id) {
            Some((_, _, _, cancelled)) => !cancelled,
            None => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared utilities
// ---------------------------------------------------------------------------

/// Format an agent ID from name and team: `agentName@teamName`.
pub fn format_agent_id(name: &str, team_name: &str) -> String {
    format!("{}@{}", name, team_name)
}

/// Parse an agent ID back into (agentName, teamName).
pub fn parse_agent_id(agent_id: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = agent_id.splitn(2, '@').collect();
    if parts.len() == 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

/// Sanitize an agent name to prevent `@` in agent IDs.
pub fn sanitize_agent_name(name: &str) -> String {
    name.replace('@', "-")
}

/// Sanitize a name for use in tmux window names.
pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Resolve a teammate model value. Handles the 'inherit' alias by
/// substituting the leader's model. If `leader_model` is None, falls
/// through to the default.
pub fn resolve_teammate_model(
    input_model: Option<&str>,
    leader_model: Option<&str>,
    configured_default: Option<&str>,
) -> String {
    if input_model == Some("inherit") {
        return leader_model
            .map(|s| s.to_string())
            .unwrap_or_else(|| get_default_teammate_model(leader_model, configured_default));
    }
    input_model
        .map(|s| s.to_string())
        .unwrap_or_else(|| get_default_teammate_model(leader_model, configured_default))
}

/// Get the default teammate model.
fn get_default_teammate_model(
    leader_model: Option<&str>,
    configured_default: Option<&str>,
) -> String {
    if let Some(configured) = configured_default {
        if configured.is_empty() {
            // User picked "Default" in config -- follow leader.
            return leader_model
                .map(|s| s.to_string())
                .unwrap_or_else(|| DEFAULT_TEAMMATE_MODEL.to_string());
        }
        return configured.to_string();
    }
    DEFAULT_TEAMMATE_MODEL.to_string()
}

/// Generate a unique teammate name by checking existing team members.
/// If the name already exists, appends a numeric suffix (e.g. `tester-2`).
pub async fn generate_unique_teammate_name(
    base_name: &str,
    team_name: &str,
) -> String {
    let team_file = match read_team_file(team_name).await {
        Some(tf) => tf,
        None => return base_name.to_string(),
    };

    let existing: HashSet<String> = team_file
        .members
        .iter()
        .map(|m| m.name.to_lowercase())
        .collect();

    if !existing.contains(&base_name.to_lowercase()) {
        return base_name.to_string();
    }

    let mut suffix = 2u32;
    loop {
        let candidate = format!("{}-{}", base_name, suffix);
        if !existing.contains(&candidate.to_lowercase()) {
            return candidate;
        }
        suffix += 1;
    }
}

/// Build CLI flags to propagate from the current session to spawned teammates.
pub fn build_inherited_cli_flags(
    plan_mode_required: bool,
    permission_mode: Option<&PermissionMode>,
    model_override: Option<&str>,
    settings_path: Option<&str>,
    inline_plugins: &[String],
    chrome_flag_override: Option<bool>,
    teammate_mode: Option<&str>,
) -> String {
    let mut flags = Vec::new();

    // Permission mode propagation.
    if plan_mode_required {
        // Don't inherit bypass permissions when plan mode is required.
    } else if permission_mode == Some(&PermissionMode::BypassPermissions) {
        flags.push("--dangerously-skip-permissions".to_string());
    } else if permission_mode == Some(&PermissionMode::AcceptEdits) {
        flags.push("--permission-mode acceptEdits".to_string());
    } else if permission_mode == Some(&PermissionMode::Auto) {
        flags.push("--permission-mode auto".to_string());
    }

    // Model override.
    if let Some(model) = model_override {
        flags.push(format!("--model '{}'", shell_escape(model)));
    }

    // Settings path.
    if let Some(path) = settings_path {
        flags.push(format!("--settings '{}'", shell_escape(path)));
    }

    // Inline plugins.
    for plugin_dir in inline_plugins {
        flags.push(format!("--plugin-dir '{}'", shell_escape(plugin_dir)));
    }

    // Teammate mode.
    if let Some(mode) = teammate_mode {
        flags.push(format!("--teammate-mode {}", mode));
    }

    // Chrome flag.
    match chrome_flag_override {
        Some(true) => flags.push("--chrome".to_string()),
        Some(false) => flags.push("--no-chrome".to_string()),
        None => {}
    }

    flags.join(" ")
}

/// Simple shell escaping for single-quoted values.
fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Build the `env KEY=VALUE ...` string for teammate spawn commands.
pub fn build_inherited_env_vars() -> String {
    let mut env_vars = vec![
        "CLAUDECODE=1".to_string(),
        "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1".to_string(),
    ];

    let forward_vars = [
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CONFIG_DIR",
        "CLAUDE_CODE_REMOTE",
        "CLAUDE_CODE_REMOTE_MEMORY_DIR",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "NO_PROXY",
        "no_proxy",
        "SSL_CERT_FILE",
        "NODE_EXTRA_CA_CERTS",
        "REQUESTS_CA_BUNDLE",
        "CURL_CA_BUNDLE",
    ];

    for key in &forward_vars {
        if let Ok(val) = std::env::var(key) {
            if !val.is_empty() {
                env_vars.push(format!("{}='{}'", key, shell_escape(&val)));
            }
        }
    }

    env_vars.join(" ")
}

/// Get the teammate color for rotation based on assignment index.
pub fn assign_teammate_color(index: usize) -> AgentColor {
    AgentColor::ALL[index % AgentColor::ALL.len()]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_and_parse_agent_id() {
        let id = format_agent_id("worker-1", "my-team");
        assert_eq!(id, "worker-1@my-team");

        let parsed = parse_agent_id(&id);
        assert_eq!(
            parsed,
            Some(("worker-1".to_string(), "my-team".to_string()))
        );

        assert_eq!(parse_agent_id("no-at-sign"), None);
    }

    #[test]
    fn test_sanitize_agent_name() {
        assert_eq!(sanitize_agent_name("worker@team"), "worker-team");
        assert_eq!(sanitize_agent_name("normal-name"), "normal-name");
    }

    #[test]
    fn test_resolve_teammate_model() {
        // Inherit with leader.
        assert_eq!(
            resolve_teammate_model(Some("inherit"), Some("claude-3-opus"), None),
            "claude-3-opus"
        );

        // Inherit without leader.
        assert_eq!(
            resolve_teammate_model(Some("inherit"), None, None),
            DEFAULT_TEAMMATE_MODEL
        );

        // Explicit model.
        assert_eq!(
            resolve_teammate_model(Some("custom-model"), Some("leader-model"), None),
            "custom-model"
        );

        // No model, no configured default.
        assert_eq!(
            resolve_teammate_model(None, None, None),
            DEFAULT_TEAMMATE_MODEL
        );

        // Configured default.
        assert_eq!(
            resolve_teammate_model(None, None, Some("sonnet-4")),
            "sonnet-4"
        );
    }

    #[test]
    fn test_build_inherited_cli_flags() {
        let flags = build_inherited_cli_flags(
            false,
            Some(&PermissionMode::BypassPermissions),
            Some("opus"),
            None,
            &[],
            None,
            Some("plan"),
        );
        assert!(flags.contains("--dangerously-skip-permissions"));
        assert!(flags.contains("--model 'opus'"));
        assert!(flags.contains("--teammate-mode plan"));

        // Plan mode required should NOT include bypass.
        let flags = build_inherited_cli_flags(
            true,
            Some(&PermissionMode::BypassPermissions),
            None,
            None,
            &[],
            None,
            None,
        );
        assert!(!flags.contains("--dangerously-skip-permissions"));
    }

    #[test]
    fn test_build_inherited_env_vars() {
        let env = build_inherited_env_vars();
        assert!(env.contains("CLAUDECODE=1"));
        assert!(env.contains("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1"));
    }

    #[test]
    fn test_assign_teammate_color() {
        assert_eq!(assign_teammate_color(0), AgentColor::Red);
        assert_eq!(assign_teammate_color(1), AgentColor::Blue);
        assert_eq!(assign_teammate_color(8), AgentColor::Red); // wraps
    }

    #[tokio::test]
    async fn test_in_process_backend_lifecycle() {
        let backend = InProcessBackend::new();

        assert!(backend.is_available().await);

        let agent_id = "test-worker@test-team";
        assert!(!backend.is_active(agent_id).await);

        // Register manually for lifecycle test.
        backend.register(agent_id, "task-1", "test-team", "test-worker");
        assert!(backend.is_active(agent_id).await);

        // Kill.
        assert!(backend.kill(agent_id).await);
        assert!(!backend.is_active(agent_id).await);
    }
}
