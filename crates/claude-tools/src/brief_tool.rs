use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// ── Global brief-mode state ──────────────────────────────────────────────────

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

// ── Instruction text ─────────────────────────────────────────────────────────

/// Instructions returned to the model when brief mode is enabled.
/// Mirrors the TS `BRIEF_PROACTIVE_SECTION` behaviour: the model should use
/// short, direct replies and avoid unnecessary commentary.
const BRIEF_ENABLED_INSTRUCTIONS: &str = "\
Brief mode is now ON. You MUST follow these rules:
- Keep responses short and direct. Omit unnecessary explanation and commentary.
- Lead with the answer, decision, or action — not preamble.
- Use file:line references, command names, and PR numbers instead of prose descriptions.
- Use second person (\"your config\"), never third.
- If the user asks a yes/no question, start with yes or no.
- Skip filler like \"Sure!\", \"Great question!\", \"Let me help you with that.\".
- When showing code, show only the relevant lines, not entire files.";

const BRIEF_DISABLED_INSTRUCTIONS: &str = "\
Brief mode is now OFF. You may return to normal verbosity levels. \
Provide full explanations and context as appropriate.";

// ── Tool ─────────────────────────────────────────────────────────────────────

pub struct BriefTool;

#[async_trait]
impl ToolExecutor for BriefTool {
    fn name(&self) -> &str {
        "Brief"
    }

    fn description(&self) -> String {
        "Toggle brief mode for more concise responses with less explanation and commentary.".to_string()
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

        // Persist in global state so other subsystems can query it.
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

    #[test]
    fn test_global_brief_state_toggle() {
        set_brief_mode(false);
        assert!(!is_brief_enabled());
        set_brief_mode(true);
        assert!(is_brief_enabled());
        set_brief_mode(false);
        assert!(!is_brief_enabled());
    }

    #[test]
    fn test_brief_tool_response_includes_brevity_instruction() {
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
        });
    }
}
