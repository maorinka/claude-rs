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
    /// Optional per-skill header under which user-supplied args
    /// are appended at invoke time. When `None`, the tool falls
    /// back to a generic `Arguments: {args}` line.
    ///
    /// Matches TS per-skill `getPromptForCommand(args)` shapes —
    /// e.g. `simplify.ts` uses `## Additional Focus`, `stuck.ts`
    /// uses `## User-provided context`, `remember.ts` uses
    /// `## Additional context from user`.
    pub argument_header: Option<String>,
    /// Optional message to return INSTEAD OF the main content when
    /// invoked with empty or missing args. Matches TS skills that
    /// short-circuit on empty input — e.g. `loop.ts` returns its
    /// USAGE_MESSAGE, `batch.ts` returns MISSING_INSTRUCTION_MESSAGE.
    /// When `None`, the tool always returns the main `content`.
    pub empty_args_message: Option<String>,
}

static SKILL_STORE: Lazy<Mutex<HashMap<String, SkillEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Register a skill that can be invoked via the SkillTool.
///
/// Args are appended as `\n\nArguments: {args}` at invoke time.
/// Use [`register_skill_with_arg_header`] for TS-equivalent
/// per-skill headers (`## Additional Focus`, etc.).
pub fn register_skill(name: &str, description: &str, content: &str) {
    register_skill_full(name, description, content, None, None);
}

/// Register a skill with a custom header for user-supplied args.
/// The header is inserted as `\n\n## {header}\n\n{args}` at
/// invoke time when args are present.
pub fn register_skill_with_arg_header(
    name: &str,
    description: &str,
    content: &str,
    argument_header: Option<&str>,
) {
    register_skill_full(name, description, content, argument_header, None);
}

/// Full registration form — content + optional per-skill arg
/// header + optional empty-args fallback.
///
/// `empty_args_message`: when set, the SkillTool returns this text
/// (instead of the main `content`) when the user invokes the skill
/// with no args. Matches TS skills that short-circuit on empty
/// input — see `loop.ts` USAGE_MESSAGE + `batch.ts`
/// MISSING_INSTRUCTION_MESSAGE.
pub fn register_skill_full(
    name: &str,
    description: &str,
    content: &str,
    argument_header: Option<&str>,
    empty_args_message: Option<&str>,
) {
    let mut store = SKILL_STORE.lock().unwrap();
    store.insert(
        name.to_string(),
        SkillEntry {
            name: name.to_string(),
            description: description.to_string(),
            content: content.to_string(),
            argument_header: argument_header.map(|s| s.to_string()),
            empty_args_message: empty_args_message.map(|s| s.to_string()),
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
                // Args are treated as "empty" when missing, or
                // present-but-trimmed-to-zero-length. TS normalises
                // via `args.trim()` before the empty branch —
                // e.g. `loop.ts` does `if (!args.trim()) return USAGE_MESSAGE`.
                let trimmed_args: Option<&str> = args.map(str::trim).filter(|s| !s.is_empty());

                // Empty-args short-circuit: if the skill declares a
                // dedicated empty_args_message, return it verbatim
                // instead of the main content.
                let content = match (&entry.empty_args_message, trimmed_args) {
                    (Some(msg), None) => msg.clone(),
                    _ => {
                        let mut body = entry.content.clone();
                        if let Some(a) = trimmed_args {
                            body = match &entry.argument_header {
                                Some(header) => {
                                    // Matches TS per-skill appenders —
                                    // e.g. simplify.ts appends
                                    // "## Additional Focus\n\n{args}".
                                    format!("{}\n\n## {}\n\n{}", body, header, a)
                                }
                                None => format!("{}\n\nArguments: {}", body, a),
                            };
                        }
                        body
                    }
                };

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
            ..Default::default()
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

    #[tokio::test]
    async fn skill_tool_empty_args_returns_empty_args_message() {
        clear_skills();
        register_skill_full(
            "loop-test",
            "Schedule",
            "MAIN PROMPT BODY",
            Some("Input"),
            Some("USAGE: /loop <prompt>"),
        );

        let tool = SkillTool;
        let ctx = make_ctx();

        // Missing args field entirely → usage message.
        let r1 = tool
            .call(&json!({ "skill": "loop-test" }), &ctx, CancellationToken::new(), None)
            .await
            .unwrap();
        assert_eq!(r1.data["content"].as_str().unwrap(), "USAGE: /loop <prompt>");

        // Empty string args → usage message (TS normalises via trim).
        let r2 = tool
            .call(
                &json!({ "skill": "loop-test", "args": "" }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(r2.data["content"].as_str().unwrap(), "USAGE: /loop <prompt>");

        // Whitespace-only args → usage message.
        let r3 = tool
            .call(
                &json!({ "skill": "loop-test", "args": "   " }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(r3.data["content"].as_str().unwrap(), "USAGE: /loop <prompt>");

        // Real args → main body + header + args (NOT the usage message).
        let r4 = tool
            .call(
                &json!({ "skill": "loop-test", "args": "5m /standup" }),
                &ctx,
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let out = r4.data["content"].as_str().unwrap();
        assert!(out.starts_with("MAIN PROMPT BODY"));
        assert!(out.contains("## Input"));
        assert!(out.contains("5m /standup"));

        clear_skills();
    }
}
