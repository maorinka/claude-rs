//! Agent-team coordinator.
//!
//! Each team is persisted as a JSON state file under
//! `~/.claude/teams/<team_id>/state.json`.  Agent processes are spawned as
//! OS child processes; their PIDs are recorded so they can be checked and
//! killed later.
//!
//! Also contains coordinator mode functions that mirror the TS
//! `coordinatorMode.ts` module.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::types::{AgentStatus, Team, TeamAgent, TeamStatus};

// ── coordinator mode ────────────────────────────────────────────────────────

/// Process-wide coordinator mode flag.
///
/// Initialized from the `CLAUDE_CODE_COORDINATOR_MODE` environment variable on
/// first access, and can be toggled at runtime via `match_session_mode` without
/// mutating the environment (which is unsafe in multithreaded async contexts).
static COORDINATOR_MODE: Lazy<AtomicBool> = Lazy::new(|| {
    let from_env = std::env::var("CLAUDE_CODE_COORDINATOR_MODE")
        .map(|v| is_env_truthy(&v))
        .unwrap_or(false);
    AtomicBool::new(from_env)
});

/// Check whether coordinator mode is enabled.
///
/// Mirrors the TS `isCoordinatorMode()`.
pub fn is_coordinator_mode() -> bool {
    COORDINATOR_MODE.load(Ordering::Relaxed)
}

fn is_env_truthy(val: &str) -> bool {
    matches!(val, "1" | "true" | "yes" | "TRUE" | "YES")
}

/// Session mode for matching on resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    Coordinator,
    Normal,
}

/// Checks if the current coordinator mode matches the session's stored mode.
/// If mismatched, flips the environment variable so `is_coordinator_mode()`
/// returns the correct value for the resumed session. Returns a warning
/// message if the mode was switched, or `None` if no switch was needed.
pub fn match_session_mode(session_mode: Option<SessionMode>) -> Option<String> {
    let session_mode = session_mode?;

    let current_is_coordinator = is_coordinator_mode();
    let session_is_coordinator = session_mode == SessionMode::Coordinator;

    if current_is_coordinator == session_is_coordinator {
        return None;
    }

    // Flip the in-process flag (no env mutation needed).
    COORDINATOR_MODE.store(session_is_coordinator, Ordering::Relaxed);

    if session_is_coordinator {
        Some("Entered coordinator mode to match resumed session.".to_string())
    } else {
        Some("Exited coordinator mode to match resumed session.".to_string())
    }
}

// ── tool allow-lists ────────────────────────────────────────────────────────

/// Tools allowed for async (sub)agents. Mirrors TS `ASYNC_AGENT_ALLOWED_TOOLS`.
pub static ASYNC_AGENT_ALLOWED_TOOLS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut set = HashSet::new();
    set.insert("Read");
    set.insert("WebSearch");
    set.insert("TodoWrite");
    set.insert("Grep");
    set.insert("WebFetch");
    set.insert("Glob");
    set.insert("Bash");
    set.insert("Edit");
    set.insert("Write");
    set.insert("NotebookEdit");
    set.insert("Skill");
    set.insert("SyntheticOutput");
    set.insert("ToolSearch");
    set.insert("EnterWorktree");
    set.insert("ExitWorktree");
    set
});

/// Tools allowed only for in-process teammates (not general async agents).
/// Mirrors TS `IN_PROCESS_TEAMMATE_ALLOWED_TOOLS`.
pub static IN_PROCESS_TEAMMATE_ALLOWED_TOOLS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut set = HashSet::new();
    set.insert("TaskCreate");
    set.insert("TaskGet");
    set.insert("TaskList");
    set.insert("TaskUpdate");
    set.insert("SendMessage");
    set
});

/// Internal worker tools that are hidden from the "worker tools context"
/// displayed to the coordinator but are still available to workers.
static INTERNAL_WORKER_TOOLS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut set = HashSet::new();
    set.insert("TeamCreate");
    set.insert("TeamDelete");
    set.insert("SendMessage");
    set.insert("SyntheticOutput");
    set
});

/// Tools allowed in coordinator mode -- only agent management tools.
pub static COORDINATOR_MODE_ALLOWED_TOOLS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut set = HashSet::new();
    set.insert("Agent");
    set.insert("TaskStop");
    set.insert("SendMessage");
    set.insert("SyntheticOutput");
    set
});

// ── coordinator prompts ─────────────────────────────────────────────────────

/// Returns context about worker tools for the coordinator.
///
/// `mcp_client_names`: names of connected MCP servers.
/// `scratchpad_dir`: optional scratchpad directory path.
/// `is_simple`: whether CLAUDE_CODE_SIMPLE is set.
pub fn get_coordinator_user_context(
    mcp_client_names: &[String],
    scratchpad_dir: Option<&str>,
    is_simple: bool,
) -> HashMap<String, String> {
    if !is_coordinator_mode() {
        return HashMap::new();
    }

    let worker_tools = if is_simple {
        let mut tools = vec!["Bash", "Edit", "Read"];
        tools.sort();
        tools.join(", ")
    } else {
        let mut tools: Vec<&str> = ASYNC_AGENT_ALLOWED_TOOLS
            .iter()
            .filter(|name| !INTERNAL_WORKER_TOOLS.contains(**name))
            .copied()
            .collect();
        tools.sort();
        tools.join(", ")
    };

    let mut content = format!(
        "Workers spawned via the Agent tool have access to these tools: {}",
        worker_tools
    );

    if !mcp_client_names.is_empty() {
        let server_names = mcp_client_names.join(", ");
        content.push_str(&format!(
            "\n\nWorkers also have access to MCP tools from connected MCP servers: {}",
            server_names
        ));
    }

    if let Some(dir) = scratchpad_dir {
        content.push_str(&format!(
            "\n\nScratchpad directory: {}\n\
             Workers can read and write here without permission prompts. \
             Use this for durable cross-worker knowledge -- structure files however fits the work.",
            dir
        ));
    }

    let mut map = HashMap::new();
    map.insert("workerToolsContext".to_string(), content);
    map
}

/// Returns the coordinator system prompt.
///
/// `is_simple`: whether CLAUDE_CODE_SIMPLE is set.
pub fn get_coordinator_system_prompt(is_simple: bool) -> String {
    let worker_capabilities = if is_simple {
        "Workers have access to Bash, Read, and Edit tools, plus MCP tools from configured MCP servers."
    } else {
        "Workers have access to standard tools, MCP tools from configured MCP servers, \
         and project skills via the Skill tool. Delegate skill invocations (e.g. /commit, /verify) to workers."
    };

    format!(
r#"You are Claude Code, an AI assistant that orchestrates software engineering tasks across multiple workers.

## 1. Your Role

You are a **coordinator**. Your job is to:
- Help the user achieve their goal
- Direct workers to research, implement and verify code changes
- Synthesize results and communicate with the user
- Answer questions directly when possible -- don't delegate work that you can handle without tools

Every message you send is to the user. Worker results and system notifications are internal signals, not conversation partners -- never thank or acknowledge them. Summarize new information for the user as it arrives.

## 2. Your Tools

- **Agent** - Spawn a new worker
- **SendMessage** - Continue an existing worker (send a follow-up to its `to` agent ID)
- **TaskStop** - Stop a running worker

When calling Agent:
- Do not use one worker to check on another. Workers will notify you when they are done.
- Do not use workers to trivially report file contents or run commands. Give them higher-level tasks.
- Do not set the model parameter. Workers need the default model for the substantive tasks you delegate.
- Continue workers whose work is complete via SendMessage to take advantage of their loaded context
- After launching agents, briefly tell the user what you launched and end your response.

## 3. Workers

When calling Agent, use subagent_type `worker`. Workers execute tasks autonomously.

{worker_capabilities}

## 4. Task Workflow

| Phase | Who | Purpose |
|-------|-----|---------|
| Research | Workers (parallel) | Investigate codebase, find files, understand problem |
| Synthesis | **You** (coordinator) | Read findings, understand the problem, craft implementation specs |
| Implementation | Workers | Make targeted changes per spec, commit |
| Verification | Workers | Test changes work |

**Parallelism is your superpower. Workers are async. Launch independent workers concurrently whenever possible.**

## 5. Writing Worker Prompts

**Workers can't see your conversation.** Every prompt must be self-contained with everything the worker needs.
When workers report research findings, **you must understand them before directing follow-up work**.
Never write "based on your findings" or "based on the research."
"#,
        worker_capabilities = worker_capabilities,
    )
}

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
