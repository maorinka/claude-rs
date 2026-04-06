use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// ─── Skill registry ──────────────────────────────────────────────────────────
//
// Skills are registered at startup. Each skill has a name and content that
// is injected as a prompt when the skill is invoked.

#[derive(Clone, Debug)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub content: String,
}

static SKILL_STORE: Lazy<Mutex<HashMap<String, SkillEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Register a skill that can be invoked via the SkillTool.
pub fn register_skill(name: &str, description: &str, content: &str) {
    let mut store = SKILL_STORE.lock().unwrap();
    store.insert(
        name.to_string(),
        SkillEntry {
            name: name.to_string(),
            description: description.to_string(),
            content: content.to_string(),
        },
    );
}

/// Get all registered skills.
pub fn list_skills() -> Vec<SkillEntry> {
    let store = SKILL_STORE.lock().unwrap();
    store.values().cloned().collect()
}

/// Clear all registered skills (for testing).
#[cfg(test)]
pub fn clear_skills() {
    let mut store = SKILL_STORE.lock().unwrap();
    store.clear();
}

// ─── SkillTool ───────────────────────────────────────────────────────────────

pub struct SkillTool;

#[async_trait]
impl ToolExecutor for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }

    fn description(&self) -> String {
        r#"Execute a skill within the main conversation

When users ask you to perform tasks, check if any of the available skills match. Skills provide specialized capabilities and domain knowledge.

When users reference a "slash command" or "/<something>" (e.g., "/commit", "/review-pr"), they are referring to a skill. Use this tool to invoke it.

How to invoke:
- Use this tool with the skill name and optional arguments
- Examples:
  - `skill: "pdf"` - invoke the pdf skill
  - `skill: "commit", args: "-m 'Fix bug'"` - invoke with arguments
  - `skill: "review-pr", args: "123"` - invoke with arguments
  - `skill: "ms-office-suite:pdf"` - invoke using fully qualified name

Important:
- Available skills are listed in system-reminder messages in the conversation
- When a skill matches the user's request, this is a BLOCKING REQUIREMENT: invoke the relevant Skill tool BEFORE generating any other response about the task
- NEVER mention a skill without actually calling this tool
- Do not invoke a skill that is already running
- Do not use this tool for built-in CLI commands (like /help, /clear, etc.)
- If you see a <command-name> tag in the current conversation turn, the skill has ALREADY been loaded - follow the instructions directly instead of calling this tool again"#
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "The skill name. E.g., \"commit\", \"review-pr\", or \"pdf\""
                },
                "args": {
                    "type": "string",
                    "description": "Optional arguments for the skill"
                }
            },
            "required": ["skill"]
        })
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let skill_name = match input.get("skill").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: skill" }),
                    is_error: true,
                });
            }
        };

        let args = input.get("args").and_then(|v| v.as_str());

        // Look up the skill
        let store = SKILL_STORE.lock().unwrap();

        // Try exact match first, then try with plugin prefix matching
        let skill = store.get(skill_name).or_else(|| {
            // Try matching by the last segment for qualified names like "plugin:skill"
            store
                .values()
                .find(|s| s.name.ends_with(&format!(":{}", skill_name)) || s.name == skill_name)
        });

        match skill {
            Some(entry) => {
                let mut content = entry.content.clone();
                if let Some(a) = args {
                    content = format!("{}\n\nArguments: {}", content, a);
                }

                Ok(ToolResultData {
                    data: json!({
                        "skill": entry.name,
                        "content": content,
                        "message": format!("Skill '{}' loaded successfully.", entry.name)
                    }),
                    is_error: false,
                })
            }
            None => {
                let available: Vec<String> = store.keys().cloned().collect();
                let available_str = if available.is_empty() {
                    "No skills are currently registered.".to_string()
                } else {
                    format!("Available skills: {}", available.join(", "))
                };

                Ok(ToolResultData {
                    data: json!({
                        "error": format!("Skill '{}' not found. {}", skill_name, available_str)
                    }),
                    is_error: true,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ReadFileState;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext {
            working_directory: PathBuf::from("/tmp"),
            read_file_state: Arc::new(std::sync::Mutex::new(ReadFileState::new())),
            permission_mode: crate::registry::PermissionMode::Default,
        }
    }

    #[tokio::test]
    async fn skill_tool_missing_name() {
        let tool = SkillTool;
        let input = json!({});
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("missing required field: skill"));
    }

    #[tokio::test]
    async fn skill_tool_not_found() {
        clear_skills();
        let tool = SkillTool;
        let input = json!({ "skill": "nonexistent" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn skill_tool_found_and_invoked() {
        clear_skills();
        register_skill(
            "commit",
            "Create a git commit",
            "Run git add and git commit with a good message.",
        );

        let tool = SkillTool;
        let input = json!({ "skill": "commit", "args": "-m 'Fix bug'" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["skill"].as_str().unwrap(), "commit");
        assert!(result.data["content"].as_str().unwrap().contains("git add"));
        assert!(result.data["content"]
            .as_str()
            .unwrap()
            .contains("Arguments: -m 'Fix bug'"));

        clear_skills();
    }

    #[tokio::test]
    async fn skill_tool_invoked_without_args() {
        clear_skills();
        register_skill(
            "review-pr",
            "Review a PR",
            "Review the pull request for code quality.",
        );

        let tool = SkillTool;
        let input = json!({ "skill": "review-pr" });
        let ctx = make_ctx();
        let cancel = CancellationToken::new();

        let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["skill"].as_str().unwrap(), "review-pr");
        assert!(!result.data["content"]
            .as_str()
            .unwrap()
            .contains("Arguments:"));

        clear_skills();
    }
}
