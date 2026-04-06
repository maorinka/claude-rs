use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// -- Global brief-mode state -------------------------------------------------

/// Process-wide flag for brief mode, matching the TS `userMsgOptIn` behaviour.
static BRIEF_MODE: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));

/// Check whether brief mode is currently active.
pub fn is_brief_enabled() -> bool {
    BRIEF_MODE.load(Ordering::Relaxed)
}

/// Programmatically set brief mode (used by `--brief` flag, `/brief` command).
pub fn set_brief_mode(enabled: bool) {
    BRIEF_MODE.store(enabled, Ordering::Relaxed);
}

// -- System prompt integration -----------------------------------------------

/// Returns the brief-mode section for the system prompt.
///
/// When brief mode is active, this returns instructions that MUST be included
/// in the system prompt to change model behavior. When inactive, returns None.
///
/// Call this from `build_system_prompt()` to wire brief mode into the prompt.
pub fn get_brief_system_prompt_section() -> Option<String> {
    if !is_brief_enabled() {
        return None;
    }

    Some(BRIEF_SYSTEM_PROMPT_SECTION.to_string())
}

/// Instructions injected into the system prompt when brief mode is ON.
/// This mirrors the TS BRIEF_PROACTIVE_SECTION / getBriefSection() behavior.
const BRIEF_SYSTEM_PROMPT_SECTION: &str = "\
# Brief Mode (ACTIVE)

You MUST follow these rules while brief mode is active:
- Keep responses short and direct. Omit unnecessary explanation and commentary.
- Lead with the answer, decision, or action -- not preamble.
- Use file:line references, command names, and PR numbers instead of prose descriptions.
- Use second person (\"your config\"), never third.
- If the user asks a yes/no question, start with yes or no.
- Skip filler like \"Sure!\", \"Great question!\", \"Let me help you with that.\".
- When showing code, show only the relevant lines, not entire files.
- Prefer bullet points over paragraphs.
- Do not repeat what the user said back to them.";

// -- Instruction text (returned to model after tool call) --------------------

/// Instructions returned to the model when brief mode is enabled.
const BRIEF_ENABLED_INSTRUCTIONS: &str = "\
Brief mode is now ON. You MUST follow these rules:
- Keep responses short and direct. Omit unnecessary explanation and commentary.
- Lead with the answer, decision, or action -- not preamble.
- Use file:line references, command names, and PR numbers instead of prose descriptions.
- Use second person (\"your config\"), never third.
- If the user asks a yes/no question, start with yes or no.
- Skip filler like \"Sure!\", \"Great question!\", \"Let me help you with that.\".
- When showing code, show only the relevant lines, not entire files.";

const BRIEF_DISABLED_INSTRUCTIONS: &str = "\
Brief mode is now OFF. You may return to normal verbosity levels. \
Provide full explanations and context as appropriate.";

// -- Tool --------------------------------------------------------------------

pub struct BriefTool;

#[async_trait]
impl ToolExecutor for BriefTool {
    fn name(&self) -> &str {
        "Brief"
    }

    fn aliases(&self) -> &[&str] {
        &["SendUserMessage"]
    }

    fn description(&self) -> String {
        "Toggle brief mode for more concise responses with less explanation \
         and commentary. When enabled, the model's system prompt is updated \
         to enforce brevity."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "enabled": {
                    "type": "boolean",
                    "description": "Whether to enable brief mode."
                }
            },
            "required": ["enabled"]
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let enabled = match input["enabled"].as_bool() {
            Some(b) => b,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: enabled (must be boolean)" }),
                    is_error: true,
                });
            }
        };

        // Persist in global state so the system prompt builder can query it.
        set_brief_mode(enabled);

        let instructions = if enabled {
            BRIEF_ENABLED_INSTRUCTIONS
        } else {
            BRIEF_DISABLED_INSTRUCTIONS
        };

        Ok(ToolResultData {
            data: json!({
                "brief_mode": enabled,
                "instructions": instructions,
            }),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // Serialise tests that touch the global BRIEF_MODE flag.
    static BRIEF_TEST_LOCK: StdMutex<()> = StdMutex::new(());

    #[test]
    fn test_global_brief_state_toggle() {
        let _guard = BRIEF_TEST_LOCK.lock().unwrap();
        set_brief_mode(false);
        assert!(!is_brief_enabled());
        set_brief_mode(true);
        assert!(is_brief_enabled());
        set_brief_mode(false);
        assert!(!is_brief_enabled());
    }

    #[test]
    fn test_brief_system_prompt_section_when_enabled() {
        let _guard = BRIEF_TEST_LOCK.lock().unwrap();
        set_brief_mode(true);
        let section = get_brief_system_prompt_section();
        assert!(
            section.is_some(),
            "should return a section when brief is enabled"
        );
        let text = section.unwrap();
        assert!(text.contains("Brief Mode (ACTIVE)"));
        assert!(text.contains("Keep responses short"));
        assert!(text.contains("file:line references"));
        set_brief_mode(false);
    }

    #[test]
    fn test_brief_system_prompt_section_when_disabled() {
        let _guard = BRIEF_TEST_LOCK.lock().unwrap();
        set_brief_mode(false);
        let section = get_brief_system_prompt_section();
        assert!(
            section.is_none(),
            "should return None when brief is disabled"
        );
    }

    #[test]
    fn test_brief_tool_response_includes_brevity_instruction() {
        let _guard = BRIEF_TEST_LOCK.lock().unwrap();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let tool = BriefTool;
            let ctx = crate::registry::ToolUseContext {
                working_directory: std::path::PathBuf::from("/tmp"),
                read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
                    crate::registry::ReadFileState::new(),
                )),
                permission_mode: crate::registry::PermissionMode::Default,
            };
            let cancel = CancellationToken::new();

            // Enable brief mode
            let input = json!({ "enabled": true });
            let result = tool.call(&input, &ctx, cancel.clone(), None).await.unwrap();
            assert!(!result.is_error);
            assert_eq!(result.data["brief_mode"], true);
            let instructions = result.data["instructions"].as_str().unwrap();
            assert!(
                instructions.contains("Brief mode is now ON"),
                "should contain brief mode ON instruction"
            );
            assert!(
                instructions.contains("Keep responses short"),
                "should contain brevity instruction"
            );
            assert!(is_brief_enabled(), "global state should be ON");

            // System prompt section should now be active
            let section = get_brief_system_prompt_section();
            assert!(section.is_some());

            // Disable brief mode
            let input = json!({ "enabled": false });
            let result = tool.call(&input, &ctx, cancel, None).await.unwrap();
            assert!(!result.is_error);
            assert_eq!(result.data["brief_mode"], false);
            let instructions = result.data["instructions"].as_str().unwrap();
            assert!(
                instructions.contains("Brief mode is now OFF"),
                "should contain brief mode OFF instruction"
            );
            assert!(!is_brief_enabled(), "global state should be OFF");

            // System prompt section should now be None
            let section = get_brief_system_prompt_section();
            assert!(section.is_none());
        });
    }

    #[test]
    fn test_brief_tool_missing_enabled_field() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let tool = BriefTool;
            let ctx = crate::registry::ToolUseContext {
                working_directory: std::path::PathBuf::from("/tmp"),
                read_file_state: std::sync::Arc::new(std::sync::Mutex::new(
                    crate::registry::ReadFileState::new(),
                )),
                permission_mode: crate::registry::PermissionMode::Default,
            };
            let cancel = CancellationToken::new();

            let result = tool.call(&json!({}), &ctx, cancel, None).await.unwrap();
            assert!(result.is_error);
            assert!(result.data["error"].as_str().unwrap().contains("enabled"));
        });
    }

    #[test]
    fn test_brief_tool_properties() {
        let tool = BriefTool;
        assert_eq!(tool.name(), "Brief");
        assert!(tool.aliases().contains(&"SendUserMessage"));
        assert!(!tool.is_read_only(&json!({})));
        assert!(!tool.is_concurrency_safe(&json!({})));
    }
}
