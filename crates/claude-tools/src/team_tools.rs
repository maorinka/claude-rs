use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::teams::coordinator::{AgentSpec, GLOBAL_COORDINATOR};
use claude_core::types::events::ToolResultData;

/// TeamCreateTool creates a new multi-agent team/swarm.
///
/// Matches TS `TeamCreateTool` — creates a team file, spawns agent processes,
/// and returns the team id and per-agent PIDs.
pub struct TeamCreateTool;

#[async_trait]
impl ToolExecutor for TeamCreateTool {
    fn name(&self) -> &str {
        "TeamCreate"
    }

    fn description(&self) -> String {
        "Create a new team for coordinating multiple agents. \
         Spawns each agent as a child process and persists the team state \
         under ~/.claude/teams/<team_id>/state.json."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["team_name"],
            "properties": {
                "team_name": {
                    "type": "string",
                    "description": "Name for the new team"
                },
                "description": {
                    "type": "string",
                    "description": "Optional team description/purpose"
                },
                "agent_type": {
                    "type": "string",
                    "description": "Type/role of the team lead (e.g. \"researcher\", \"test-runner\")"
                },
                "agents": {
                    "type": "array",
                    "description": "List of agent specs to spawn",
                    "items": {
                        "type": "object",
                        "required": ["name", "prompt"],
                        "properties": {
                            "name":   { "type": "string" },
                            "prompt": { "type": "string" },
                            "model":  { "type": "string" }
                        }
                    }
                }
            }
        })
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let team_name = match input["team_name"].as_str() {
            Some(n) if !n.trim().is_empty() => n.trim().to_string(),
            _ => {
                return Ok(ToolResultData {
                    data: json!({ "error": "team_name is required and must be non-empty" }),
                    is_error: true,
                });
            }
        };

        // Parse optional agent list; fall back to a single default agent.
        let agent_specs: Vec<AgentSpec> = if let Some(arr) = input["agents"].as_array() {
            arr.iter()
                .filter_map(|v| {
                    let name = v["name"].as_str()?;
                    let prompt = v["prompt"].as_str()?;
                    let model = v["model"].as_str().map(str::to_string);
                    let mut spec = AgentSpec::new(name, prompt);
                    if let Some(m) = model {
                        spec = spec.with_model(m);
                    }
                    Some(spec)
                })
                .collect()
        } else {
            vec![AgentSpec::new("default-agent", &team_name)]
        };

        match GLOBAL_COORDINATOR
            .create_team(team_name.clone(), agent_specs)
            .await
        {
            Ok(team) => {
                info!(team_id = %team.id, team_name = %team_name, "TeamCreateTool: created team");
                let agents_json: Vec<Value> = team
                    .agents
                    .iter()
                    .map(|a| {
                        json!({
                            "id":     a.id,
                            "name":   a.name,
                            "pid":    a.pid,
                            "status": format!("{:?}", a.status),
                        })
                    })
                    .collect();
                Ok(ToolResultData {
                    data: json!({
                        "team_id":   team.id,
                        "team_name": team.name,
                        "status":    format!("{:?}", team.status),
                        "agents":    agents_json,
                    }),
                    is_error: false,
                })
            }
            Err(e) => {
                warn!("TeamCreateTool: failed to create team: {}", e);
                Ok(ToolResultData {
                    data: json!({ "error": format!("Failed to create team: {}", e) }),
                    is_error: true,
                })
            }
        }
    }
}

/// TeamDeleteTool stops a team and kills all its agent processes.
///
/// Matches TS `TeamDeleteTool` — calls `coordinator.stop_team()`.
pub struct TeamDeleteTool;

#[async_trait]
impl ToolExecutor for TeamDeleteTool {
    fn name(&self) -> &str {
        "TeamDelete"
    }

    fn description(&self) -> String {
        "Stop a team: kill all agent processes and mark the team as stopped. \
         Accepts either a team_id (UUID) or a team_name."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "team_id": {
                    "type": "string",
                    "description": "UUID of the team to stop (preferred)"
                },
                "team_name": {
                    "type": "string",
                    "description": "Name of the team to stop (used if team_id is absent)"
                }
            }
        })
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        // Prefer explicit team_id; fall back to finding by name.
        let team_id_opt = input["team_id"].as_str().map(str::to_string);
        let team_name_opt = input["team_name"].as_str().map(str::to_string);

        let team_id = match team_id_opt {
            Some(id) => id,
            None => {
                // Look up by name in the in-memory list.
                let name = match team_name_opt {
                    Some(n) => n,
                    None => {
                        return Ok(ToolResultData {
                            data: json!({ "error": "Either team_id or team_name is required" }),
                            is_error: true,
                        });
                    }
                };
                match GLOBAL_COORDINATOR
                    .list_teams()
                    .into_iter()
                    .find(|t| t.name == name)
                {
                    Some(t) => t.id,
                    None => {
                        return Ok(ToolResultData {
                            data: json!({
                                "error": format!("No team named '{}' found", name)
                            }),
                            is_error: true,
                        });
                    }
                }
            }
        };

        match GLOBAL_COORDINATOR.stop_team(&team_id).await {
            Ok(()) => {
                info!(team_id = %team_id, "TeamDeleteTool: stopped team");
                Ok(ToolResultData {
                    data: json!({
                        "success":  true,
                        "team_id":  team_id,
                        "message":  "Team stopped and all agent processes killed",
                    }),
                    is_error: false,
                })
            }
            Err(e) => {
                warn!("TeamDeleteTool: failed to stop team {}: {}", team_id, e);
                Ok(ToolResultData {
                    data: json!({ "error": format!("Failed to stop team: {}", e) }),
                    is_error: true,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_team_create_tool_name() {
        let tool = TeamCreateTool;
        assert_eq!(tool.name(), "TeamCreate");
    }

    #[test]
    fn test_team_delete_tool_name() {
        let tool = TeamDeleteTool;
        assert_eq!(tool.name(), "TeamDelete");
    }

    #[test]
    fn test_team_create_schema() {
        let tool = TeamCreateTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "team_name"));
        assert!(schema["properties"]["team_name"].is_object());
        assert!(schema["properties"]["agents"].is_object());
    }

    #[test]
    fn test_team_delete_schema() {
        let tool = TeamDeleteTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["team_id"].is_object());
        assert!(schema["properties"]["team_name"].is_object());
    }
}
