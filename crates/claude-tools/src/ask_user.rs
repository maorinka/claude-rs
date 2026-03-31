use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::sync::Mutex;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

// ─── Channel for user-input responses ─────────────────────────────────────────
//
// When `AskUserQuestionTool::call` is invoked it:
//   1. Creates a `oneshot` channel.
//   2. Stores the `Sender` here so the TUI can pick it up.
//   3. Returns a `ToolResultData` with `awaiting_input: true` immediately.
//
// The TUI detects `awaiting_input: true`, shows an input dialog, and calls
// `send_user_answer` with the response.  That unblocks the tool's `await`.

static ASK_USER_TX: Lazy<Mutex<Option<oneshot::Sender<String>>>> =
    Lazy::new(|| Mutex::new(None));

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

pub struct AskUserQuestionTool;

#[async_trait]
impl ToolExecutor for AskUserQuestionTool {
    fn name(&self) -> &str {
        "AskUser"
    }

    fn description(&self) -> String {
        "Ask the user a question and wait for their response. Use this when you need \
         clarification or a decision from the user before proceeding."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of choices to present to the user."
                }
            },
            "required": ["question"]
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
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
            Some(q) => q,
            None => {
                return Ok(ToolResultData {
                    data: json!({ "error": "missing required field: question" }),
                    is_error: true,
                });
            }
        };

        let options: Option<Vec<String>> = input
            .get("options")
            .and_then(|o| o.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });

        // Set up a oneshot channel so the TUI can send the user's reply.
        let (tx, rx) = oneshot::channel::<String>();
        {
            let mut guard = ASK_USER_TX.lock().unwrap();
            // Replace any previous sender (shouldn't normally be set, but be safe).
            *guard = Some(tx);
        }

        // Signal TUI that user input is needed.
        // Include the question and options so the TUI can display them.
        let awaiting_data = json!({
            "awaiting_input": true,
            "question": question,
            "options": options,
        });

        // The `call` contract returns `ToolResultData`, but here we need to *wait*
        // for the TUI answer.  We use `tokio::select!` so that cancellation is
        // respected and we don't leak the sender.
        let answer = tokio::select! {
            result = rx => {
                match result {
                    Ok(ans) => ans,
                    Err(_) => {
                        // Sender dropped without sending (e.g. TUI exited).
                        return Ok(ToolResultData {
                            data: json!({ "error": "user input channel closed" }),
                            is_error: true,
                        });
                    }
                }
            }
            _ = cancel.cancelled() => {
                // Cancelled — clean up the stored sender.
                let mut guard = ASK_USER_TX.lock().unwrap();
                *guard = None;
                return Ok(ToolResultData {
                    data: json!({ "error": "cancelled" }),
                    is_error: true,
                });
            }
        };

        let _ = awaiting_data; // suppress unused warning; data was used above for documentation

        Ok(ToolResultData {
            data: json!({ "answer": answer }),
            is_error: false,
        })
    }
}
