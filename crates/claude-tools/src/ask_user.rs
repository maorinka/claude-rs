use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// -- Channel for user-input responses ----------------------------------------
//
// When `AskUserQuestionTool::call` is invoked it:
//   1. Creates a `oneshot` channel.
//   2. Stores the `Sender` here so the TUI can pick it up.
//   3. Blocks on the `Receiver` until the TUI sends an answer.
//
// The TUI detects the tool awaiting input, shows an input dialog, and calls
// `send_user_answer` with the response. That unblocks the tool.

static ASK_USER_TX: Lazy<Mutex<Option<oneshot::Sender<String>>>> = Lazy::new(|| Mutex::new(None));

/// Called by the TUI layer when the user has submitted an answer.
/// Returns `true` if a waiting sender was found, `false` otherwise.
pub fn send_user_answer(answer: String) -> bool {
    let mut guard = ASK_USER_TX.lock().unwrap();
    if let Some(tx) = guard.take() {
        // The receiver may already be gone (e.g. cancellation), ignore errors.
        let _ = tx.send(answer);
        true
    } else {
        false
    }
}

/// Check if there is currently a question awaiting an answer.
/// Used by the TUI to decide whether to show the AskUser dialog.
pub fn is_awaiting_answer() -> bool {
    let guard = ASK_USER_TX.lock().unwrap();
    guard.is_some()
}

/// Get the current pending question info (question, options) if available.
/// This is stored alongside the sender when a question is asked.
static PENDING_QUESTION: Lazy<Mutex<Option<PendingQuestion>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone)]
pub struct PendingQuestion {
    pub question: String,
    pub options: Vec<String>,
}

/// Get the pending question info. Returns None if no question is awaiting.
pub fn get_pending_question() -> Option<PendingQuestion> {
    let guard = PENDING_QUESTION.lock().unwrap();
    guard.clone()
}

pub struct AskUserQuestionTool;

/// Verbatim port of TS AskUserQuestionTool/prompt.ts
/// `ASK_USER_QUESTION_TOOL_PROMPT`. TS `${EXIT_PLAN_MODE_TOOL_NAME}`
/// resolves to `ExitPlanMode` at runtime; baked in as a literal.
pub const ASK_USER_PROMPT: &str = include_str!("prompts/ask_user.md");

#[async_trait]
impl ToolExecutor for AskUserQuestionTool {
    fn name(&self) -> &str {
        "AskUser"
    }

    fn description(&self) -> String {
        ASK_USER_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user. Should be clear and end with a question mark."
                },
                "options": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": {
                                "type": "string",
                                "description": "The display text for this option (1-5 words)."
                            },
                            "description": {
                                "type": "string",
                                "description": "Explanation of what this option means."
                            }
                        },
                        "required": ["label", "description"]
                    },
                    "description": "Optional list of choices to present to the user (2-4 options)."
                }
            },
            "required": ["question"]
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // Only one question can be asked at a time (single TUI dialog).
        false
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let question = match input["question"].as_str() {
            Some(q) if !q.trim().is_empty() => q.to_string(),
            _ => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: question" }),
                    is_error: true,
                });
            }
        };

        // Extract options -- support both simple string arrays and structured objects
        let options: Vec<String> = input
            .get("options")
            .and_then(|o| o.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        // Support { label, description } objects or plain strings
                        v.as_str().map(|s| s.to_string()).or_else(|| {
                            v.get("label")
                                .and_then(|l| l.as_str())
                                .map(|s| s.to_string())
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Set up a oneshot channel so the TUI can send the user's reply.
        let (tx, rx) = oneshot::channel::<String>();
        {
            let mut guard = ASK_USER_TX.lock().unwrap();
            *guard = Some(tx);
        }

        // Store the question info so the TUI can retrieve it
        {
            let mut guard = PENDING_QUESTION.lock().unwrap();
            *guard = Some(PendingQuestion {
                question: question.clone(),
                options: options.clone(),
            });
        }

        // Block until the TUI sends an answer or cancellation occurs.
        let answer = tokio::select! {
            result = rx => {
                match result {
                    Ok(ans) => ans,
                    Err(_) => {
                        // Sender dropped without sending (e.g. TUI exited).
                        Self::clear_pending();
                        return Ok(ToolResultData {
                            data: json!({ "error": "user input channel closed" }),
                            is_error: true,
                        });
                    }
                }
            }
            _ = cancel.cancelled() => {
                // Cancelled -- clean up the stored sender.
                Self::clear_pending();
                return Ok(ToolResultData {
                    data: json!({ "error": "cancelled" }),
                    is_error: true,
                });
            }
        };

        // Clean up pending state
        Self::clear_pending();

        // Build the response with questions and answers map (matching TS output schema)
        let mut answers = HashMap::new();
        answers.insert(question.clone(), answer.clone());

        Ok(ToolResultData {
            data: json!({
                "questions": [{ "question": question, "options": options }],
                "answers": answers,
                "answer": answer
            }),
            is_error: false,
        })
    }
}

impl AskUserQuestionTool {
    fn clear_pending() {
        {
            let mut guard = ASK_USER_TX.lock().unwrap();
            *guard = None;
        }
        {
            let mut guard = PENDING_QUESTION.lock().unwrap();
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;

    // Serialise channel-flow tests so they don't steal each other's global sender.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn make_ctx() -> ToolUseContext {
        ToolUseContext::for_test(PathBuf::from("/tmp"), std::sync::Arc::new(std::sync::Mutex::new(
                crate::registry::ReadFileState::new(),
            )), crate::registry::PermissionMode::Default)
    }

    #[tokio::test]
    async fn test_channel_flow_returns_answer() {
        let _guard = TEST_LOCK.lock().unwrap();

        let tool = AskUserQuestionTool;
        let input = json!({ "question": "What is your name?" });
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle =
            tokio::spawn(async move { tool.call(&input, &make_ctx(), cancel_clone, None).await });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Verify pending question state
        let pending = get_pending_question();
        assert!(pending.is_some(), "should have a pending question");
        assert_eq!(pending.unwrap().question, "What is your name?");

        send_user_answer("Alice".to_string());

        let result = handle.await.unwrap().unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["answer"], "Alice");
        // Check answers map
        assert_eq!(result.data["answers"]["What is your name?"], "Alice");
    }

    #[tokio::test]
    async fn test_channel_flow_with_structured_options() {
        let _guard = TEST_LOCK.lock().unwrap();

        let tool = AskUserQuestionTool;
        let input = json!({
            "question": "Pick a color",
            "options": [
                { "label": "Red", "description": "A warm color" },
                { "label": "Blue", "description": "A cool color" }
            ]
        });
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle =
            tokio::spawn(async move { tool.call(&input, &make_ctx(), cancel_clone, None).await });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        send_user_answer("Red".to_string());

        let result = handle.await.unwrap().unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["answer"], "Red");
        // Options should be stored in the response
        let questions = result.data["questions"].as_array().unwrap();
        assert_eq!(questions[0]["options"][0], "Red");
    }

    #[tokio::test]
    async fn test_cancellation_returns_error() {
        let _guard = TEST_LOCK.lock().unwrap();

        let tool = AskUserQuestionTool;
        let input = json!({ "question": "Will you wait?" });
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle =
            tokio::spawn(async move { tool.call(&input, &make_ctx(), cancel_clone, None).await });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        cancel.cancel();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let result = handle.await.unwrap().unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("cancel"));
        // Pending state should be cleared
        assert!(!is_awaiting_answer());
    }

    #[tokio::test]
    async fn test_missing_question_returns_error() {
        let _guard = TEST_LOCK.lock().unwrap();

        let tool = AskUserQuestionTool;
        let result = tool
            .call(&json!({}), &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("question"));
    }

    #[test]
    fn test_ask_user_tool_properties() {
        let tool = AskUserQuestionTool;
        assert_eq!(tool.name(), "AskUser");
        assert!(tool.is_read_only(&json!({})));
        assert!(!tool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn test_send_user_answer_returns_false_when_no_sender() {
        let _guard = TEST_LOCK.lock().unwrap();
        // No pending question
        let result = send_user_answer("test".to_string());
        assert!(!result, "should return false when no sender is waiting");
    }
}
