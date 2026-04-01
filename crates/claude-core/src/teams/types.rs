use serde::{Deserialize, Serialize};

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
