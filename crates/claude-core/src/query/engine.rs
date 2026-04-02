use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::api::accumulator::ContentBlockAccumulator;
use crate::api::client::{ApiClient, ToolDefinition};
use crate::api::sse::{self, ContentDelta, SseEvent};
use crate::types::content::ContentBlock;
use crate::types::events::StreamEvent;
use crate::types::message::StopReason;

use super::state::{QueryState, TransitionReason};

const MAX_OUTPUT_TOKENS_RECOVERY_LIMIT: u32 = 3;
const ESCALATED_MAX_TOKENS: u32 = 64_000;

pub struct QueryEngine {
    api_client: ApiClient,
    messages: Vec<serde_json::Value>,
    system_prompt: Vec<ContentBlock>,
    tool_schemas: Vec<ToolDefinition>,
    state: QueryState,
    cancel: CancellationToken,
    // Recovery state
    max_output_tokens_override: Option<u32>,
    recovery_count: u32,
    turn_count: u32,
    max_turns: Option<u32>,
}

impl QueryEngine {
    pub fn new(
        api_client: ApiClient,
        system_prompt: Vec<ContentBlock>,
        tool_schemas: Vec<ToolDefinition>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            api_client,
            messages: Vec::new(),
            system_prompt,
            tool_schemas,
            state: QueryState::Querying,
            cancel,
            max_output_tokens_override: None,
            recovery_count: 0,
            turn_count: 0,
            max_turns: None,
        }
    }

    pub fn set_max_turns(&mut self, max: u32) {
        self.max_turns = Some(max);
    }

    /// Load messages from a previous session transcript to resume a conversation.
    /// Each value should be a JSON message object with "role" and "content" fields.
    pub fn load_messages(&mut self, messages: Vec<serde_json::Value>) {
        self.messages = messages;
    }

    /// Append a text block to the system prompt.
    pub fn append_system_prompt(&mut self, text: String) {
        self.system_prompt.push(ContentBlock::Text { text });
    }

    /// Add additional tool schemas (e.g. from MCP servers discovered at runtime).
    pub fn extend_tool_schemas(&mut self, extra: Vec<ToolDefinition>) {
        self.tool_schemas.extend(extra);
    }

    pub fn state(&self) -> &QueryState {
        &self.state
    }

    pub fn messages(&self) -> &[serde_json::Value] {
        &self.messages
    }

    /// Add a user message
    pub fn add_user_message(&mut self, text: &str) {
        self.messages.push(serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": text}]
        }));
    }

    /// Add a tool result message
    pub fn add_tool_result(&mut self, tool_use_id: &str, content: &str, is_error: bool) {
        self.messages.push(serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": [{"type": "text", "text": content}],
                "is_error": is_error,
            }]
        }));
    }

    /// Add the raw assistant message from the API response
    pub fn add_assistant_message(&mut self, content: Vec<serde_json::Value>) {
        self.messages.push(serde_json::json!({
            "role": "assistant",
            "content": content,
        }));
    }

    /// Run one turn of the query loop.
    /// Returns collected tool_use blocks (if any) and the stop reason.
    pub async fn run_turn(&mut self, event_tx: &mpsc::Sender<StreamEvent>) -> Result<TurnResult> {
        if self.cancel.is_cancelled() {
            self.state = QueryState::Terminal {
                stop_reason: StopReason::EndTurn,
                transition: TransitionReason::Aborted,
            };
            return Ok(TurnResult::Done(StopReason::EndTurn));
        }

        // Check max turns
        if let Some(max) = self.max_turns {
            if self.turn_count >= max {
                self.state = QueryState::Terminal {
                    stop_reason: StopReason::EndTurn,
                    transition: TransitionReason::MaxTurns,
                };
                return Ok(TurnResult::Done(StopReason::EndTurn));
            }
        }

        self.turn_count += 1;
        self.state = QueryState::Querying;

        // Check if compaction is needed before next API request (matches TS behavior:
        // autocompact runs BEFORE the API call, not after the response)
        if crate::compact::compactor::should_compact(
            &self.messages,
            crate::compact::compactor::default_context_window(),
        ) {
            let _ = event_tx
                .send(StreamEvent::Compacted {
                    summary: "Compacting conversation...".into(),
                })
                .await;
            match crate::compact::compactor::compact_conversation(
                &self.api_client,
                &self.messages,
                &self.system_prompt,
            )
            .await
            {
                Ok(compacted) => {
                    self.messages = compacted;
                }
                Err(e) => {
                    tracing::warn!("Compaction failed: {}", e);
                    // Continue without compaction — will eventually hit context limit
                }
            }
        }

        // Apply max_output_tokens override if set
        if let Some(override_tokens) = self.max_output_tokens_override {
            self.api_client.config.max_tokens = override_tokens as u64;
        }

        // Send request start event
        let _ = event_tx
            .send(StreamEvent::RequestStart {
                request_id: format!("turn_{}", self.turn_count),
            })
            .await;

        let response = self
            .api_client
            .stream_request_with_events(
                &self.messages,
                &self.system_prompt,
                &self.tool_schemas,
                Some(event_tx),
            )
            .await?;

        self.state = QueryState::Streaming;

        // Stream SSE events incrementally from the response body.
        // Instead of buffering the entire response, we read chunks as they
        // arrive, parse complete SSE events from them, and process each event
        // immediately.  This enables real-time text streaming to the TUI and
        // mid-response cancellation.
        let mut byte_stream = response.bytes_stream();
        let mut line_buffer = String::new();
        let mut current_event_type: Option<String> = None;
        let mut current_data: Option<String> = None;

        let mut accumulator = ContentBlockAccumulator::new();
        let mut tool_use_blocks: Vec<ToolUseInfo> = Vec::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut assistant_content: Vec<serde_json::Value> = Vec::new();

        while let Some(chunk) = byte_stream.next().await {
            if self.cancel.is_cancelled() {
                self.state = QueryState::Terminal {
                    stop_reason: StopReason::EndTurn,
                    transition: TransitionReason::Aborted,
                };
                return Ok(TurnResult::Done(StopReason::EndTurn));
            }

            let chunk = chunk?;
            line_buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process all complete lines accumulated so far
            while let Some(newline_pos) = line_buffer.find('\n') {
                let line = line_buffer[..newline_pos]
                    .trim_end_matches('\r')
                    .to_string();
                line_buffer = line_buffer[newline_pos + 1..].to_string();

                if let Some(rest) = line.strip_prefix("event:") {
                    current_event_type = Some(rest.trim().to_string());
                } else if let Some(rest) = line.strip_prefix("data:") {
                    current_data = Some(rest.trim().to_string());
                } else if line.is_empty() {
                    // Empty line marks the end of an SSE event
                    if let (Some(event_type), Some(data)) =
                        (current_event_type.take(), current_data.take())
                    {
                        if let Ok(event) = sse::parse_sse_event(&event_type, &data) {
                            match event {
                                SseEvent::ContentBlockStart { index, block } => {
                                    accumulator.on_start(index, block);
                                }
                                SseEvent::ContentBlockDelta { index, delta } => {
                                    // Emit streaming deltas
                                    match &delta {
                                        ContentDelta::TextDelta { text } => {
                                            let _ = event_tx
                                                .send(StreamEvent::TextDelta { text: text.clone() })
                                                .await;
                                        }
                                        ContentDelta::ThinkingDelta { thinking } => {
                                            let _ = event_tx
                                                .send(StreamEvent::ThinkingDelta {
                                                    text: thinking.clone(),
                                                })
                                                .await;
                                        }
                                        _ => {}
                                    }
                                    accumulator.on_delta(index, delta);
                                }
                                SseEvent::ContentBlockStop { index } => {
                                    if let Ok(block) = accumulator.on_stop(index) {
                                        match &block {
                                            ContentBlock::ToolUse { id, name, input } => {
                                                let _ = event_tx
                                                    .send(StreamEvent::ToolStart {
                                                        tool_use_id: id.clone(),
                                                        name: name.clone(),
                                                        input: input.clone(),
                                                    })
                                                    .await;
                                                tool_use_blocks.push(ToolUseInfo {
                                                    id: id.clone(),
                                                    name: name.clone(),
                                                    input: input.clone(),
                                                });
                                                assistant_content.push(serde_json::json!({
                                                    "type": "tool_use",
                                                    "id": id,
                                                    "name": name,
                                                    "input": input,
                                                }));
                                            }
                                            ContentBlock::Text { text } => {
                                                assistant_content.push(serde_json::json!({
                                                    "type": "text",
                                                    "text": text,
                                                }));
                                            }
                                            ContentBlock::Thinking {
                                                thinking,
                                                signature,
                                            } => {
                                                assistant_content.push(serde_json::json!({
                                                    "type": "thinking",
                                                    "thinking": thinking,
                                                    "signature": signature,
                                                }));
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                SseEvent::MessageDelta {
                                    stop_reason: sr,
                                    usage,
                                } => {
                                    if let Some(sr_str) = sr {
                                        stop_reason = match sr_str.as_str() {
                                            "end_turn" => StopReason::EndTurn,
                                            "tool_use" => StopReason::ToolUse,
                                            "max_tokens" => StopReason::MaxTokens,
                                            "stop_sequence" => StopReason::StopSequence,
                                            _ => StopReason::EndTurn,
                                        };
                                    }
                                    if let Some(u) = usage {
                                        let _ = event_tx
                                            .send(StreamEvent::UsageUpdate(
                                                crate::types::usage::Usage {
                                                    input_tokens: 0,
                                                    output_tokens: u.output_tokens,
                                                    cache_creation_input_tokens: None,
                                                    cache_read_input_tokens: None,
                                                },
                                            ))
                                            .await;
                                    }
                                }
                                SseEvent::MessageStart { message } => {
                                    let _ = event_tx
                                        .send(StreamEvent::UsageUpdate(message.usage.clone()))
                                        .await;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Add assistant message to history
        if !assistant_content.is_empty() {
            self.add_assistant_message(assistant_content);
        }

        // Handle stop reason
        match stop_reason {
            StopReason::ToolUse if !tool_use_blocks.is_empty() => {
                self.state = QueryState::ExecutingTools;
                Ok(TurnResult::ToolUse(tool_use_blocks))
            }
            StopReason::MaxTokens => self.handle_max_tokens(event_tx).await,
            _ => {
                self.state = QueryState::Terminal {
                    stop_reason: stop_reason.clone(),
                    transition: TransitionReason::Completed,
                };
                let _ = event_tx
                    .send(StreamEvent::Done {
                        stop_reason: stop_reason.clone(),
                    })
                    .await;
                Ok(TurnResult::Done(stop_reason))
            }
        }
    }

    async fn handle_max_tokens(
        &mut self,
        event_tx: &mpsc::Sender<StreamEvent>,
    ) -> Result<TurnResult> {
        // Stage 1: One-shot escalation (8k → 64k)
        if self.max_output_tokens_override.is_none() {
            self.max_output_tokens_override = Some(ESCALATED_MAX_TOKENS);
            self.state = QueryState::RecoveringMaxTokens {
                recovery_count: self.recovery_count,
                escalated: true,
            };
            return Ok(TurnResult::ContinueRecovery);
        }

        // Stage 2: Recovery loop (up to 3)
        // Clear the escalated override — go back to default max_tokens (matches TS:
        // maxOutputTokensOverride: undefined in the stage-2 state transition)
        if self.recovery_count < MAX_OUTPUT_TOKENS_RECOVERY_LIMIT {
            self.max_output_tokens_override = None;
            self.recovery_count += 1;
            self.messages.push(serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": "Output token limit hit. Resume directly \u{2014} no apology, no recap of what you were doing. Pick up mid-thought if that is where the cut happened. Break remaining work into smaller pieces."}]
            }));
            self.state = QueryState::RecoveringMaxTokens {
                recovery_count: self.recovery_count,
                escalated: true,
            };
            return Ok(TurnResult::ContinueRecovery);
        }

        // Stage 3: Exhausted
        self.state = QueryState::Terminal {
            stop_reason: StopReason::MaxTokens,
            transition: TransitionReason::Error(
                crate::types::error::QueryError::MaxTokensExhausted {
                    recovery_count: self.recovery_count,
                },
            ),
        };
        let _ = event_tx
            .send(StreamEvent::Done {
                stop_reason: StopReason::MaxTokens,
            })
            .await;
        Ok(TurnResult::Done(StopReason::MaxTokens))
    }
}

#[derive(Debug)]
pub struct ToolUseInfo {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug)]
pub enum TurnResult {
    /// Query complete
    Done(StopReason),
    /// Tools need to be executed, then continue
    ToolUse(Vec<ToolUseInfo>),
    /// Max tokens recovery — caller should call run_turn again
    ContinueRecovery,
}
