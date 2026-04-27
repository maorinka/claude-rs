use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::sync::Mutex;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// -- Channel for user-input responses ----------------------------------------
//
// When `AskUserQuestionTool::call` needs to collect an answer locally it:
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
        "AskUserQuestion"
    }

    fn aliases(&self) -> &[&str] {
        &["AskUser"]
    }

    fn description(&self) -> String {
        ASK_USER_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 4,
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "The complete question to ask the user. Should be clear, specific, and end with a question mark."
                            },
                            "header": {
                                "type": "string",
                                "description": "Very short label displayed as a chip/tag."
                            },
                            "options": {
                                "type": "array",
                                "minItems": 2,
                                "maxItems": 4,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {
                                            "type": "string",
                                            "description": "The display text for this option that the user will see and select."
                                        },
                                        "description": {
                                            "type": "string",
                                            "description": "Explanation of what this option means or what will happen if chosen."
                                        },
                                        "preview": {
                                            "type": "string",
                                            "description": "Optional preview content rendered when this option is focused."
                                        }
                                    },
                                    "required": ["label", "description"]
                                }
                            },
                            "multiSelect": {
                                "type": "boolean",
                                "description": "Set to true to allow the user to select multiple options instead of just one."
                            }
                        },
                        "required": ["question", "header", "options", "multiSelect"]
                    },
                    "description": "Questions to ask the user (1-4 questions)"
                },
                "answers": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "User answers collected by the permission component"
                },
                "annotations": {
                    "type": "object",
                    "description": "Optional per-question annotations from the user."
                },
                "metadata": {
                    "type": "object",
                    "description": "Optional metadata for tracking and analytics purposes. Not displayed to user."
                }
            },
            "required": ["questions"]
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let questions = match input.get("questions").and_then(|value| value.as_array()) {
            Some(questions) if !questions.is_empty() => questions.clone(),
            _ => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: questions" }),
                    is_error: true,
                });
            }
        };

        if let Some(answers) = input.get("answers").and_then(|value| value.as_object()) {
            let mut data = json!({
                "questions": questions,
                "answers": answers,
            });
            if let Some(annotations) = input.get("annotations") {
                data["annotations"] = annotations.clone();
            }
            return Ok(ToolResultData {
                data,
                is_error: false,
            });
        }

        let mut answers = serde_json::Map::new();
        for question_def in &questions {
            let question = match question_def
                .get("question")
                .and_then(|value| value.as_str())
            {
                Some(question) if !question.trim().is_empty() => question.to_string(),
                _ => {
                    return Ok(ToolResultData {
                        data: json!({ "error": "missing required field: question" }),
                        is_error: true,
                    });
                }
            };
            let options = option_labels(question_def);
            let answer = match Self::ask_one(question.clone(), options, cancel.clone()).await {
                Ok(answer) => answer,
                Err(error) => {
                    return Ok(ToolResultData {
                        data: json!({ "error": error.to_string() }),
                        is_error: true,
                    });
                }
            };
            answers.insert(question, Value::String(answer));
        }

        Ok(ToolResultData {
            data: json!({
                "questions": questions,
                "answers": answers,
            }),
            is_error: false,
        })
    }
}

fn option_labels(question_def: &Value) -> Vec<String> {
    question_def
        .get("options")
        .and_then(|options| options.as_array())
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    option.as_str().map(str::to_string).or_else(|| {
                        option
                            .get("label")
                            .and_then(|label| label.as_str())
                            .map(str::to_string)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl AskUserQuestionTool {
    async fn ask_one(
        question: String,
        options: Vec<String>,
        cancel: CancellationToken,
    ) -> Result<String> {
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
                        anyhow::bail!("user input channel closed");
                    }
                }
            }
            _ = cancel.cancelled() => {
                // Cancelled -- clean up the stored sender.
                Self::clear_pending();
                anyhow::bail!("cancelled");
            }
        };

        // Clean up pending state
        Self::clear_pending();
        Ok(answer)
    }

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
#[allow(clippy::await_holding_lock)] // test-only global-state serialization via std::sync::Mutex
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;

    // Serialise channel-flow tests so they don't steal each other's global sender.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn make_ctx() -> ToolUseContext {
        ToolUseContext::for_test(
            PathBuf::from("/tmp"),
            std::sync::Arc::new(std::sync::Mutex::new(crate::registry::ReadFileState::new())),
            crate::registry::PermissionMode::Default,
        )
    }

    fn question_input(question: &str) -> Value {
        json!({
            "questions": [{
                "question": question,
                "header": "Choice",
                "options": [
                    { "label": "Yes", "description": "Use this option" },
                    { "label": "No", "description": "Use the other option" }
                ],
                "multiSelect": false
            }]
        })
    }

    #[tokio::test]
    async fn test_channel_flow_returns_answer() {
        let _guard = TEST_LOCK.lock().unwrap();

        let tool = AskUserQuestionTool;
        let input = question_input("What is your name?");
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
        assert_eq!(result.data["answers"]["What is your name?"], "Alice");
    }

    #[tokio::test]
    async fn test_channel_flow_with_structured_options() {
        let _guard = TEST_LOCK.lock().unwrap();

        let tool = AskUserQuestionTool;
        let input = json!({
            "questions": [{
                "question": "Pick a color",
                "header": "Color",
                "options": [
                    { "label": "Red", "description": "A warm color" },
                    { "label": "Blue", "description": "A cool color" }
                ],
                "multiSelect": false
            }]
        });
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle =
            tokio::spawn(async move { tool.call(&input, &make_ctx(), cancel_clone, None).await });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        send_user_answer("Red".to_string());

        let result = handle.await.unwrap().unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["answers"]["Pick a color"], "Red");
        // Options should be stored in the response
        let questions = result.data["questions"].as_array().unwrap();
        assert_eq!(questions[0]["options"][0]["label"], "Red");
    }

    #[tokio::test]
    async fn test_call_returns_permission_collected_answers() {
        let _guard = TEST_LOCK.lock().unwrap();

        let tool = AskUserQuestionTool;
        let result = tool
            .call(
                &json!({
                    "questions": [{
                        "question": "Pick one?",
                        "header": "Pick",
                        "options": [
                            { "label": "A", "description": "First" },
                            { "label": "B", "description": "Second" }
                        ],
                        "multiSelect": false
                    }],
                    "answers": { "Pick one?": "A" },
                    "annotations": { "Pick one?": { "notes": "fast path" } }
                }),
                &make_ctx(),
                CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.data["answers"]["Pick one?"], "A");
        assert_eq!(
            result.data["annotations"]["Pick one?"]["notes"],
            "fast path"
        );
        assert!(!is_awaiting_answer());
    }

    #[tokio::test]
    async fn test_cancellation_returns_error() {
        let _guard = TEST_LOCK.lock().unwrap();

        let tool = AskUserQuestionTool;
        let input = question_input("Will you wait?");
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
        assert_eq!(tool.name(), "AskUserQuestion");
        assert_eq!(tool.aliases(), &["AskUser"]);
        assert!(tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn test_send_user_answer_returns_false_when_no_sender() {
        let _guard = TEST_LOCK.lock().unwrap();
        // No pending question
        let result = send_user_answer("test".to_string());
        assert!(!result, "should return false when no sender is waiting");
    }
}
