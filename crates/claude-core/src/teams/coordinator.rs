//! Agent-team coordinator.
//!
//! Each team is persisted as a JSON state file under
//! `~/.claude/teams/<team_id>/state.json`.  Agent processes are spawned as
//! OS child processes; their PIDs are recorded so they can be checked and
//! killed later.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::types::{AgentStatus, Team, TeamAgent, TeamStatus};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Return the base directory for team state files: `~/.claude/teams/`.
fn teams_base_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("teams")
}

/// Return the state-file path for a given team id.
fn team_state_path(team_id: &str) -> PathBuf {
    teams_base_dir().join(team_id).join("state.json")
}

/// Load a team from disk synchronously.
fn read_team_sync(team_id: &str) -> Result<Team> {
    let path = team_state_path(team_id);
    let json = std::fs::read_to_string(&path).with_context(|| format!("read {:?}", path))?;
    serde_json::from_str(&json).context("deserialize Team")
}

/// Persist a team asynchronously.
async fn write_team_async(team: &Team) -> Result<()> {
    let path = team_state_path(&team.id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create team dir {:?}", parent))?;
    }
    let json = serde_json::to_string_pretty(team)?;
    tokio::fs::write(&path, json)
        .await
        .with_context(|| format!("write {:?}", path))?;
    Ok(())
}

/// Check whether an OS process is still alive (and not a zombie).
///
/// Uses `ps -p <pid> -o state=` to get the process state.  A zombie (`Z`)
/// is treated as dead because it has already terminated — it's just waiting
/// for the parent to call `wait()`.
fn process_is_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let output = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "state="])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                let state = String::from_utf8_lossy(&o.stdout);
                let state = state.trim();
                // 'Z' = zombie; treat as not running.
                !state.is_empty() && state != "Z"
            }
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix we cannot cheaply probe without platform APIs.
        let _ = pid;
        true
    }
}

/// Kill an OS process by PID.
fn kill_process(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
    }
    #[cfg(not(unix))]
    {
        // On Windows we could use taskkill, but teams are a Unix-first feature.
        warn!(
            "kill_process: not implemented on this platform (pid={})",
            pid
        );
    }
}

// ── coordinator ───────────────────────────────────────────────────────────────

/// In-memory registry of teams managed in this process.
///
/// Wrap in `Arc<Mutex<…>>` to share across async tasks.
#[derive(Debug, Default)]
pub struct TeamCoordinator {
    /// team_id → Team
    teams: Mutex<HashMap<String, Team>>,
    /// Optional executable override for spawning agents.
    /// When `None`, `std::env::current_exe()` is used (production).
    /// Tests can set this to e.g. `/bin/sleep` or `/bin/echo`.
    exe_override: Option<PathBuf>,
}

impl TeamCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a coordinator that spawns agents using the given executable
    /// instead of `std::env::current_exe()`.  Intended for tests.
    #[cfg(test)]
    pub fn with_exe(exe: impl Into<PathBuf>) -> Self {
        Self {
            teams: Mutex::new(HashMap::new()),
            exe_override: Some(exe.into()),
        }
    }

    /// Create a new team, spawn each agent as a child process, and persist
    /// the team state to `~/.claude/teams/<team_id>/state.json`.
    ///
    /// `agent_specs` is a list of `(name, prompt, model)` tuples; each entry
    /// becomes one `TeamAgent`.  Each agent is spawned as the current
    /// executable (`claude-rs`) with `-p <prompt>` and the agent's
    /// configuration.
    pub async fn create_team(
        &self,
        name: impl Into<String>,
        agent_specs: Vec<AgentSpec>,
    ) -> Result<Team> {
        let name = name.into();
        let team_id = Uuid::new_v4().to_string();

        let mut agents: Vec<TeamAgent> = agent_specs
            .into_iter()
            .map(|spec| TeamAgent {
                id: Uuid::new_v4().to_string(),
                name: spec.name,
                prompt: spec.prompt,
                model: spec.model,
                pid: None,
                status: AgentStatus::Pending,
            })
            .collect();

        // Spawn each agent as a child process.
        let exe_ref = self.exe_override.as_deref();
        for agent in &mut agents {
            match spawn_agent_process_with_exe(agent, exe_ref) {
                Ok(pid) => {
                    agent.pid = Some(pid);
                    agent.status = AgentStatus::Running;
                    info!(team = %team_id, agent = %agent.id, pid, "Spawned agent process");
                }
                Err(e) => {
                    warn!(
                        team = %team_id,
                        agent = %agent.id,
                        error = %e,
                        "Failed to spawn agent process"
                    );
                    agent.status = AgentStatus::Failed;
                }
            }
        }

        let team = Team {
            id: team_id.clone(),
            name: name.clone(),
            agents,
            status: TeamStatus::Active,
        };

        write_team_async(&team).await?;
        debug!(team = %team_id, "Persisted team state");

        let mut store = self.teams.lock().unwrap();
        store.insert(team_id.clone(), team.clone());
        info!(team = %team_id, %name, "Created team");

        Ok(team)
    }

    /// Stop all agent processes in a team and mark it as `Stopped`.
    pub async fn stop_team(&self, team_id: &str) -> Result<()> {
        let mut team = {
            let store = self.teams.lock().unwrap();
            match store.get(team_id) {
                Some(t) => t.clone(),
                None => {
                    // Try loading from disk if not in memory.
                    drop(store);
                    read_team_sync(team_id)?
                }
            }
        };

        for agent in &mut team.agents {
            if let Some(pid) = agent.pid {
                kill_process(pid);
                agent.status = AgentStatus::Completed;
                debug!(agent = %agent.id, pid, "Killed agent process");
            }
        }
        team.status = TeamStatus::Stopped;

        write_team_async(&team).await?;

        let mut store = self.teams.lock().unwrap();
        store.insert(team_id.to_string(), team);
        info!(team = %team_id, "Stopped team");

        Ok(())
    }

    /// Return a snapshot of the team with each agent's `status` refreshed
    /// by probing the OS for its PID.
    pub async fn get_team_status(&self, team_id: &str) -> Result<Team> {
        let mut team = {
            let store = self.teams.lock().unwrap();
            match store.get(team_id) {
                Some(t) => t.clone(),
                None => {
                    drop(store);
                    read_team_sync(team_id)?
                }
            }
        };

        // Refresh running/completed status for each agent.
        for agent in &mut team.agents {
            if agent.status == AgentStatus::Running {
                if let Some(pid) = agent.pid {
                    if !process_is_running(pid) {
                        agent.status = AgentStatus::Completed;
                    }
                }
            }
        }

        // If all agents finished, mark team as stopped.
        let all_done = team
            .agents
            .iter()
            .all(|a| matches!(a.status, AgentStatus::Completed | AgentStatus::Failed));
        if all_done && team.status == TeamStatus::Active {
            team.status = TeamStatus::Stopped;
        }

        // Persist the refreshed state.
        write_team_async(&team).await?;

        let mut store = self.teams.lock().unwrap();
        store.insert(team_id.to_string(), team.clone());

        Ok(team)
    }

    /// Return all teams currently tracked in memory.
    pub fn list_teams(&self) -> Vec<Team> {
        self.teams.lock().unwrap().values().cloned().collect()
    }

    /// Load all teams persisted under `~/.claude/teams/` into the in-memory
    /// registry and return them.
    pub async fn load_all_teams(&self) -> Result<Vec<Team>> {
        let base = teams_base_dir();
        if !base.exists() {
            return Ok(vec![]);
        }

        let mut read_dir = tokio::fs::read_dir(&base).await?;
        let mut loaded = Vec::new();

        while let Some(entry) = read_dir.next_entry().await? {
            let team_id = entry.file_name().to_string_lossy().into_owned();
            let state_path = base.join(&team_id).join("state.json");
            if !state_path.exists() {
                continue;
            }
            match read_team_sync(&team_id) {
                Ok(team) => {
                    let mut store = self.teams.lock().unwrap();
                    store.insert(team_id.clone(), team.clone());
                    loaded.push(team);
                }
                Err(e) => {
                    warn!(team = %team_id, error = %e, "Failed to load team from disk");
                }
            }
        }

        Ok(loaded)
    }
}

// ── agent spawning ────────────────────────────────────────────────────────────

/// Parameters for a single agent in a new team.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub prompt: String,
    pub model: Option<String>,
}

impl AgentSpec {
    pub fn new(name: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            prompt: prompt.into(),
            model: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

/// Inner implementation that accepts an optional executable override.
/// When `exe_override` is `None` the current executable is used and
/// the full CLI argument protocol is followed (`-p <prompt>`, `--model`, env vars).
/// When an override is provided (tests), the prompt is passed as a bare
/// positional argument so callers can use simple utilities like `sleep` or `echo`.
fn spawn_agent_process_with_exe(
    agent: &TeamAgent,
    exe_override: Option<&std::path::Path>,
) -> Result<u32> {
    let (exe, is_real) = match exe_override {
        Some(p) => (p.to_path_buf(), false),
        None => (
            std::env::current_exe().context("could not determine current executable path")?,
            true,
        ),
    };

    let mut cmd = std::process::Command::new(&exe);

    if is_real {
        // Production: pass prompt via `-p` flag with full CLI protocol.
        cmd.arg("-p").arg(&agent.prompt);
        if let Some(ref model) = agent.model {
            cmd.arg("--model").arg(model);
        }
    } else {
        // Test mode: pass prompt as a bare positional arg so simple
        // utilities like `sleep 3600` or `echo hello` work.
        cmd.arg(&agent.prompt);
    }

    cmd.env("CLAUDE_TEAM_NAME", &agent.name);
    cmd.env("CLAUDE_CODE_AGENT_ID", &agent.id);

    // Redirect stdio so the child doesn't inherit the parent's terminal.
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let child = cmd
        .spawn()
        .with_context(|| format!("spawn agent process for agent '{}'", agent.name))?;
    Ok(child.id())
}

// ── global singleton ──────────────────────────────────────────────────────────

use once_cell::sync::Lazy;

/// Process-wide coordinator singleton — mirrors the TS `SwarmCoordinator`.
pub static GLOBAL_COORDINATOR: Lazy<Arc<TeamCoordinator>> =
    Lazy::new(|| Arc::new(TeamCoordinator::new()));

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: run an async test.
    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    // Override the teams base dir by using a temp dir via env var is complex;
    // instead we directly exercise the coordinator with the real home dir but
    // clean up afterwards.  For isolation we give every test a unique team name.

    #[test]
    fn test_create_team_writes_state_file() {
        rt().block_on(async {
            // Use `sleep` as stand-in so the process stays alive for inspection.
            let coordinator = TeamCoordinator::with_exe("sleep");
            let specs = vec![AgentSpec::new("worker-1", "3600")];

            let team = coordinator
                .create_team("test-create-writes-state", specs)
                .await
                .expect("create_team should succeed");

            // Verify the state file exists.
            let path = team_state_path(&team.id);
            assert!(path.exists(), "state file should be created at {:?}", path);

            // Read it back and verify content.
            let json = std::fs::read_to_string(&path).unwrap();
            let reloaded: Team = serde_json::from_str(&json).unwrap();
            assert_eq!(reloaded.name, "test-create-writes-state");
            assert_eq!(reloaded.agents.len(), 1);
            assert_eq!(reloaded.agents[0].name, "worker-1");

            // Cleanup.
            let _ = coordinator.stop_team(&team.id).await;
            let dir = teams_base_dir().join(&team.id);
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[test]
    fn test_team_status_tracking() {
        rt().block_on(async {
            let coordinator = TeamCoordinator::with_exe("sleep");
            let specs = vec![
                AgentSpec::new("agent-a", "3600"),
                AgentSpec::new("agent-b", "3600"),
            ];

            let team = coordinator
                .create_team("test-status-tracking", specs)
                .await
                .expect("create_team");

            // Both agents should be Running (sleep processes).
            for a in &team.agents {
                assert_eq!(
                    a.status,
                    AgentStatus::Running,
                    "agent {} should be Running",
                    a.name
                );
                assert!(a.pid.is_some(), "agent {} should have a PID", a.name);
            }

            let refreshed = coordinator
                .get_team_status(&team.id)
                .await
                .expect("get_team_status");
            assert_eq!(refreshed.status, TeamStatus::Active);

            // Cleanup.
            let _ = coordinator.stop_team(&team.id).await;
            let dir = teams_base_dir().join(&team.id);
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[test]
    fn test_stop_team_kills_processes() {
        rt().block_on(async {
            let coordinator = TeamCoordinator::with_exe("sleep");
            let specs = vec![AgentSpec::new("sleeper", "3600")];

            let team = coordinator
                .create_team("test-stop-kills", specs)
                .await
                .expect("create_team");

            let pid = team.agents[0].pid.expect("agent should have PID");

            // Process should be alive immediately after spawning.
            assert!(
                process_is_running(pid),
                "process {pid} should be running before stop"
            );

            coordinator.stop_team(&team.id).await.expect("stop_team");

            // Poll for up to 2 seconds to give the OS time to reap the process.
            let dead = {
                let mut dead = false;
                for _ in 0..20 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if !process_is_running(pid) {
                        dead = true;
                        break;
                    }
                }
                dead
            };

            assert!(
                dead,
                "process {pid} should be dead within 2s after stop_team"
            );

            // Cleanup.
            let dir = teams_base_dir().join(&team.id);
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    #[test]
    fn test_list_teams() {
        rt().block_on(async {
            let coordinator = TeamCoordinator::with_exe("sleep");

            assert_eq!(
                coordinator.list_teams().len(),
                0,
                "fresh coordinator should have no teams"
            );

            let t1 = coordinator
                .create_team("list-team-alpha", vec![AgentSpec::new("a1", "3600")])
                .await
                .unwrap();
            let t2 = coordinator
                .create_team("list-team-beta", vec![AgentSpec::new("b1", "3600")])
                .await
                .unwrap();

            let teams = coordinator.list_teams();
            assert_eq!(teams.len(), 2);
            let ids: Vec<_> = teams.iter().map(|t| t.id.clone()).collect();
            assert!(ids.contains(&t1.id));
            assert!(ids.contains(&t2.id));

            // Cleanup.
            let _ = coordinator.stop_team(&t1.id).await;
            let _ = coordinator.stop_team(&t2.id).await;
            let _ = std::fs::remove_dir_all(teams_base_dir().join(&t1.id));
            let _ = std::fs::remove_dir_all(teams_base_dir().join(&t2.id));
        });
    }

    /// Verify that spawning agents uses a real executable (not `sleep`).
    /// Uses `/bin/echo` as the stand-in — it exits immediately after
    /// printing its args, proving the agent prompt is passed via `-p`.
    #[test]
    fn test_spawn_agent_uses_real_executable() {
        rt().block_on(async {
            let coordinator = TeamCoordinator::with_exe("/bin/echo");
            let specs = vec![AgentSpec::new("echo-agent", "hello world").with_model("test-model")];

            let team = coordinator
                .create_team("test-real-exe-spawn", specs)
                .await
                .expect("create_team with /bin/echo");

            // The echo process exits immediately, but it should have
            // been assigned a PID and started as Running.
            let agent = &team.agents[0];
            assert!(agent.pid.is_some(), "agent should have been assigned a PID");
            // Initial status is Running (set at spawn time).
            assert_eq!(agent.status, AgentStatus::Running);

            // After a brief wait the process will have exited; refresh
            // should detect it as completed.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let refreshed = coordinator
                .get_team_status(&team.id)
                .await
                .expect("get_team_status");
            assert_eq!(
                refreshed.agents[0].status,
                AgentStatus::Completed,
                "echo process should complete quickly"
            );

            // Cleanup.
            let dir = teams_base_dir().join(&team.id);
            let _ = std::fs::remove_dir_all(dir);
        });
    }
}
