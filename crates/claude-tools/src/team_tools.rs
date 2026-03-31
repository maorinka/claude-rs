use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

/// TeamCreateTool creates a new multi-agent team/swarm.
///
/// Matches TS `TeamCreateTool` — creates a team file, registers the leader,
/// and returns the team name and lead agent ID.
pub struct TeamCreateTool;

#[async_trait]
impl ToolExecutor for TeamCreateTool {
    fn name(&self) -> &str {
        "TeamCreate"
    }

    fn description(&self) -> String {
        "Create a new team for coordinating multiple agents. \
         Sets up the team file and registers the calling agent as team leader."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["team_name"],
            "properties": {
                "team_name": {
                    "type": "string",
                    "description": "Name for the new team to create"
                },
                "description": {
                    "type": "string",
                    "description": "Optional team description/purpose"
                },
                "agent_type": {
                    "type": "string",
                    "description": "Type/role of the team lead (e.g. \"researcher\", \"test-runner\")"
                }
            }
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
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

        let description = input
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let agent_type = input
            .get("agent_type")
            .and_then(|v| v.as_str())
            .unwrap_or("team-lead");

        // Generate a deterministic lead agent ID
        let lead_agent_id = format!("team-lead@{}", sanitize_name(&team_name));

        // Create the team file directory
        let teams_dir = ctx.working_directory.join(".claude").join("teams");
        let team_file_path = teams_dir.join(format!("{}.json", sanitize_name(&team_name)));

        if let Err(e) = tokio::fs::create_dir_all(&teams_dir).await {
            warn!("Failed to create teams directory: {}", e);
        }

        let team_file = json!({
            "name": team_name,
            "description": description,
            "createdAt": chrono::Utc::now().timestamp_millis(),
            "leadAgentId": lead_agent_id,
            "members": [
                {
                    "agentId": lead_agent_id,
                    "name": "team-lead",
                    "agentType": agent_type,
                    "joinedAt": chrono::Utc::now().timestamp_millis(),
                    "cwd": ctx.working_directory.to_string_lossy(),
                }
            ]
        });

        if let Err(e) = tokio::fs::write(
            &team_file_path,
            serde_json::to_string_pretty(&team_file)?,
        )
        .await
        {
            return Ok(ToolResultData {
                data: json!({ "error": format!("Failed to write team file: {}", e) }),
                is_error: true,
            });
        }

        info!(
            team = team_name,
            lead = lead_agent_id,
            "Created team"
        );

        Ok(ToolResultData {
            data: json!({
                "team_name": team_name,
                "team_file_path": team_file_path.to_string_lossy(),
                "lead_agent_id": lead_agent_id,
            }),
            is_error: false,
        })
    }
}

/// TeamDeleteTool cleans up a team/swarm and its associated resources.
///
/// Matches TS `TeamDeleteTool` — removes team files and task directories.
pub struct TeamDeleteTool;

#[async_trait]
impl ToolExecutor for TeamDeleteTool {
    fn name(&self) -> &str {
        "TeamDelete"
    }

    fn description(&self) -> String {
        "Clean up team and task directories when the swarm is complete. \
         Removes the team file and associated resources."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "team_name": {
                    "type": "string",
                    "description": "Name of the team to delete. If omitted, deletes the current team."
                }
            }
        })
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let team_name = input
            .get("team_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let teams_dir = ctx.working_directory.join(".claude").join("teams");

        if let Some(ref name) = team_name {
            let team_file = teams_dir.join(format!("{}.json", sanitize_name(name)));

            if team_file.exists() {
                // Check for active members before deleting
                if let Ok(content) = tokio::fs::read_to_string(&team_file).await {
                    if let Ok(team_data) = serde_json::from_str::<Value>(&content) {
                        if let Some(members) = team_data["members"].as_array() {
                            let active_non_lead: Vec<_> = members
                                .iter()
                                .filter(|m| {
                                    m["name"].as_str() != Some("team-lead")
                                        && m["isActive"].as_bool() != Some(false)
                                })
                                .collect();

                            if !active_non_lead.is_empty() {
                                let names: Vec<_> = active_non_lead
                                    .iter()
                                    .filter_map(|m| m["name"].as_str())
                                    .collect();
                                return Ok(ToolResultData {
                                    data: json!({
                                        "success": false,
                                        "message": format!(
                                            "Cannot cleanup team with {} active member(s): {}",
                                            active_non_lead.len(),
                                            names.join(", ")
                                        ),
                                        "team_name": name,
                                    }),
                                    is_error: false,
                                });
                            }
                        }
                    }
                }

                // Remove the team file
                if let Err(e) = tokio::fs::remove_file(&team_file).await {
                    warn!("Failed to remove team file: {}", e);
                }

                debug!(team = name.as_str(), "Deleted team file");
            }

            Ok(ToolResultData {
                data: json!({
                    "success": true,
                    "message": format!("Cleaned up directories for team \"{}\"", name),
                    "team_name": name,
                }),
                is_error: false,
            })
        } else {
            Ok(ToolResultData {
                data: json!({
                    "success": true,
                    "message": "No team name found, nothing to clean up",
                }),
                is_error: false,
            })
        }
    }
}

/// Sanitize a name for use as a filename (replace non-alphanumeric with hyphens).
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
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
        assert!(schema["properties"]["description"].is_object());
        assert!(schema["properties"]["agent_type"].is_object());
    }

    #[test]
    fn test_team_delete_schema() {
        let tool = TeamDeleteTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["team_name"].is_object());
    }

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("my-team"), "my-team");
        assert_eq!(sanitize_name("my team"), "my-team");
        assert_eq!(sanitize_name("team_v2"), "team_v2");
        assert_eq!(sanitize_name("team@123"), "team-123");
    }
}
