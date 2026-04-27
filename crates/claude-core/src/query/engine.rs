use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::api::accumulator::ContentBlockAccumulator;
use crate::api::client::{ApiClient, ToolDefinition};
use crate::api::sse::{self, ContentDelta, SseEvent};
use crate::types::content::ContentBlock;
use crate::types::events::StreamEvent;
use crate::types::message::{ApiMessage, AssistantMessage, StopReason};
use crate::types::usage::Usage;

use super::state::{QueryState, TransitionReason};

const MAX_OUTPUT_TOKENS_RECOVERY_LIMIT: u32 = 3;
const ESCALATED_MAX_TOKENS: u32 = 64_000;

/// Default stream idle timeout: 90 seconds (matches TS CLAUDE_STREAM_IDLE_TIMEOUT_MS default).
const STREAM_IDLE_TIMEOUT_MS: u64 = 90_000;

/// Message emitted in synthetic tool_result blocks when a turn is cancelled mid-stream.
/// Matches TS CANCEL_MESSAGE in src/services/tools/toolExecution.ts.
const CANCEL_MSG: &str =
    "The user doesn't want to take this action right now. STOP what you are doing and wait for the user to tell you how to proceed.";

#[derive(Clone, Debug)]
struct UsageAnchor {
    message_count: usize,
    usage: Usage,
}

pub struct QueryEngine {
    api_client: ApiClient,
    messages: Vec<serde_json::Value>,
    system_prompt: Vec<ContentBlock>,
    user_context_blocks: Vec<serde_json::Value>,
    tool_schemas: Vec<ToolDefinition>,
    state: QueryState,
    cancel: CancellationToken,
    // Recovery state
    max_output_tokens_override: Option<u32>,
    recovery_count: u32,
    turn_count: u32,
    /// Turns since the last successful compaction. Used for max_turns gating
    /// (matches TS `tracking.turnCounter` which resets on compact).
    turns_since_compact: u32,
    max_turns: Option<u32>,
    /// True once we have already escalated to ESCALATED_MAX_TOKENS (Stage 1).
    /// Prevents re-triggering Stage 1 on each Stage 2 recovery iteration.
    has_escalated_max_tokens: bool,
    /// When true, auto-compact is skipped for the next turn.
    /// Set when a compaction has just completed to prevent re-entry.
    /// Mirrors TS: querySource !== 'compact' check in query.ts:630.
    skip_autocompact: bool,
    /// When true, we have already attempted a reactive compact on prompt-too-long.
    /// Prevents infinite retry loops.
    has_attempted_reactive_compact: bool,
    /// Last real API usage and the history length at that assistant response.
    /// TS uses the latest assistant usage as a token-count anchor, then only
    /// estimates messages appended after it. This keeps post-first-turn
    /// autocompact checks cheap and aligned with server accounting.
    usage_anchor: Option<UsageAnchor>,
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
            user_context_blocks: Vec::new(),
            tool_schemas,
            state: QueryState::Querying,
            cancel,
            max_output_tokens_override: None,
            recovery_count: 0,
            turn_count: 0,
            turns_since_compact: 0,
            max_turns: None,
            has_escalated_max_tokens: false,
            skip_autocompact: false,
            has_attempted_reactive_compact: false,
            usage_anchor: None,
        }
    }

    pub fn set_max_turns(&mut self, max: u32) {
        self.max_turns = Some(max);
    }

    /// Update the model used for API requests (e.g. after /model switch).
    pub fn set_model(&mut self, model: String) {
        self.api_client.config.model = model;
    }

    /// Replace the cancellation token (e.g. when a new cancel scope is opened for a new turn).
    pub fn set_cancel_token(&mut self, token: CancellationToken) {
        self.cancel = token;
    }

    /// Load messages from a previous session transcript to resume a conversation.
    /// Each value should be a JSON message object with "role" and "content" fields.
    /// Applies compact boundary filtering so resumed sessions start from the correct
    /// post-compaction point (matches TS `getMessagesAfterCompactBoundary`).
    pub fn load_messages(&mut self, messages: Vec<serde_json::Value>) {
        self.messages = filter_after_compact_boundary(messages);
        self.usage_anchor = None;
    }

    /// Run prefix-preserving partial compaction from the selected message onward.
    /// Returns the compacted active message list and installs it into the engine.
    pub async fn partial_compact_from(
        &mut self,
        pivot_index: usize,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let compacted = crate::compact::compactor::partial_compact_conversation_from(
            &self.api_client,
            &self.messages,
            pivot_index,
            &self.system_prompt,
        )
        .await?;
        self.messages = compacted.clone();
        self.usage_anchor = None;
        self.skip_autocompact = true;
        self.turns_since_compact = 0;
        Ok(compacted)
    }

    /// Append a text block to the system prompt.
    pub fn append_system_prompt(&mut self, text: String) {
        self.system_prompt.push(ContentBlock::Text { text });
    }

    /// Append a request-time user context block.
    pub fn append_user_context_block(&mut self, text: String) {
        self.user_context_blocks
            .push(serde_json::json!({"type": "text", "text": text}));
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
        let block = serde_json::json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": [{"type": "text", "text": content}],
            "is_error": is_error,
        });

        if let Some(last) = self.messages.last_mut() {
            let can_append = last.get("role").and_then(|role| role.as_str()) == Some("user")
                && last
                    .get("content")
                    .and_then(|content| content.as_array())
                    .map(|content| {
                        !content.is_empty()
                            && content.iter().all(|block| {
                                block.get("type").and_then(|ty| ty.as_str()) == Some("tool_result")
                            })
                    })
                    .unwrap_or(false);
            if can_append {
                if let Some(content) = last
                    .get_mut("content")
                    .and_then(|content| content.as_array_mut())
                {
                    content.push(block);
                    return;
                }
            }
        }

        self.messages.push(serde_json::json!({
            "role": "user",
            "content": [block]
        }));
    }

    /// Repair message history to satisfy API constraints:
    /// 1. Each tool_use must have exactly one tool_result (add missing, remove duplicates)
    /// 2. No orphaned tool_use blocks without results
    fn repair_tool_use_results(&mut self) {
        // Pass 1: collect all tool_use IDs and count tool_results per ID
        let mut tool_use_ids: Vec<String> = Vec::new();
        let mut result_count: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for msg in &self.messages {
            let role = msg["role"].as_str().unwrap_or("");
            if let Some(content) = msg["content"].as_array() {
                for block in content {
                    let block_type = block["type"].as_str().unwrap_or("");
                    match (role, block_type) {
                        ("assistant", "tool_use") => {
                            if let Some(id) = block["id"].as_str() {
                                tool_use_ids.push(id.to_string());
                            }
                        }
                        ("user", "tool_result") => {
                            if let Some(id) = block["tool_use_id"].as_str() {
                                *result_count.entry(id.to_string()).or_insert(0) += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Pass 2: remove duplicate tool_results (keep first, remove rest)
        let duplicates: Vec<String> = result_count
            .iter()
            .filter(|(_, count)| **count > 1)
            .map(|(id, _)| id.clone())
            .collect();

        if !duplicates.is_empty() {
            tracing::warn!(
                count = duplicates.len(),
                "Removing duplicate tool_result blocks"
            );
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for msg in &mut self.messages {
                if msg["role"].as_str() != Some("user") {
                    continue;
                }
                if let Some(content) = msg["content"].as_array_mut() {
                    content.retain(|block| {
                        if block["type"].as_str() == Some("tool_result") {
                            if let Some(id) = block["tool_use_id"].as_str() {
                                if duplicates.contains(&id.to_string()) {
                                    // Keep first occurrence, remove rest
                                    return seen.insert(id.to_string());
                                }
                            }
                        }
                        true // keep non-tool_result blocks
                    });
                }
                // Remove empty user messages left after dedup
                if msg["content"]
                    .as_array()
                    .map(|a| a.is_empty())
                    .unwrap_or(false)
                {
                    *msg = serde_json::json!(null);
                }
            }
            self.messages.retain(|m| !m.is_null());
        }

        // Pass 3: add placeholder results for orphaned tool_use blocks
        let result_ids: std::collections::HashSet<String> = result_count.keys().cloned().collect();
        let orphans: Vec<String> = tool_use_ids
            .into_iter()
            .filter(|id| !result_ids.contains(id))
            .collect();

        if !orphans.is_empty() {
            tracing::warn!(
                count = orphans.len(),
                "Adding placeholder tool_results for orphaned tool_use blocks"
            );
            let mut result_blocks: Vec<serde_json::Value> = Vec::new();
            for id in &orphans {
                result_blocks.push(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": [{"type": "text", "text": "Tool execution was interrupted or failed silently."}],
                    "is_error": true,
                }));
            }
            self.messages.push(serde_json::json!({
                "role": "user",
                "content": result_blocks,
            }));
        }
    }

    /// Add the raw assistant message from the API response
    pub fn add_assistant_message(&mut self, content: Vec<serde_json::Value>) {
        self.messages.push(serde_json::json!({
            "role": "assistant",
            "content": content,
        }));
    }

    fn should_compact_now(&self) -> bool {
        let context_window = crate::compact::compactor::default_context_window();
        let estimated = if let Some(anchor) = &self.usage_anchor {
            if anchor.message_count <= self.messages.len() {
                crate::compact::compactor::token_count_from_usage(&anchor.usage)
                    + crate::compact::compactor::estimate_tokens(
                        &self.messages[anchor.message_count..],
                    )
            } else {
                crate::compact::compactor::estimate_tokens(&self.messages)
            }
        } else {
            crate::compact::compactor::estimate_tokens(&self.messages)
        };
        crate::compact::compactor::should_compact_estimated(estimated, context_window)
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

        // Check max turns — uses turns_since_compact to match TS turnCounter behavior
        // (counter resets after each compaction so users don't burn their budget on
        // compaction turns).
        if let Some(max) = self.max_turns {
            if self.turns_since_compact >= max {
                self.state = QueryState::Terminal {
                    stop_reason: StopReason::EndTurn,
                    transition: TransitionReason::MaxTurns,
                };
                return Ok(TurnResult::Done(StopReason::EndTurn));
            }
        }

        self.turn_count += 1;
        self.turns_since_compact += 1;
        self.state = QueryState::Querying;

        // Check if compaction is needed before next API request (matches TS behavior:
        // autocompact runs BEFORE the API call, not after the response).
        // Auto-compact guard: skip if we just compacted (prevents re-entry).
        // Mirrors TS: querySource !== 'compact' check in query.ts:630.
        if !self.skip_autocompact && self.should_compact_now() {
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
                    self.usage_anchor = None;
                    // Block re-compaction for the immediately following turn (Issue 38).
                    self.skip_autocompact = true;
                    // Reset post-compact turn counter (Issue 39).
                    self.turns_since_compact = 0;
                }
                Err(e) => {
                    tracing::warn!("Compaction failed: {}", e);
                    // Continue without compaction — will eventually hit context limit
                }
            }
        } else {
            // Clear the guard — it's only valid for one turn after compaction.
            self.skip_autocompact = false;
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

        // Repair orphaned tool_use blocks: if the last assistant message has tool_use
        // blocks without corresponding tool_result messages, add placeholder results.
        // This can happen if a tool execution fails silently or the event flow is interrupted.
        self.repair_tool_use_results();

        // Make the API call, with reactive compaction on prompt-too-long (Issue 11).
        // We use a loop with at most 2 iterations: the first attempt, and optionally a
        // single retry after reactive compaction (avoids async recursion / Box::pin).
        let response = 'api_call: loop {
            // Build dynamic user context prepend (Issue 25).
            // Mirrors TS prependUserContext() — injects currentDate as a
            // separate meta user message at request time.
            let messages_for_query =
                build_messages_for_query(&self.messages, &self.user_context_blocks);
            match self
                .api_client
                .stream_request_with_events(
                    &messages_for_query,
                    &self.system_prompt,
                    &self.tool_schemas,
                    Some(event_tx),
                )
                .await
            {
                Ok(resp) => break 'api_call resp,
                Err(e) => {
                    // Issue 11: catch prompt-too-long errors and attempt reactive compaction once.
                    if e.downcast_ref::<crate::types::error::PromptTooLongError>()
                        .is_some()
                        && !self.has_attempted_reactive_compact
                    {
                        self.has_attempted_reactive_compact = true;
                        let _ = event_tx
                            .send(StreamEvent::Compacted {
                                summary: "Context too long — compacting and retrying...".into(),
                            })
                            .await;
                        if let Ok(compacted) = crate::compact::compactor::compact_conversation(
                            &self.api_client,
                            &self.messages,
                            &self.system_prompt,
                        )
                        .await
                        {
                            self.messages = compacted;
                            self.usage_anchor = None;
                            // Loop will retry the API call with the compacted messages.
                            continue 'api_call;
                        }
                    }
                    // Only treat prompt-too-long as a graceful Done.
                    // All other errors (auth, network, 429, 529, etc.) must
                    // propagate so callers can retry or surface the real error.
                    if e.downcast_ref::<crate::types::error::PromptTooLongError>()
                        .is_some()
                    {
                        self.state = QueryState::Terminal {
                            stop_reason: StopReason::EndTurn,
                            transition: TransitionReason::Error(
                                crate::types::error::QueryError::PromptTooLong,
                            ),
                        };
                        let _ = event_tx
                            .send(StreamEvent::Done {
                                stop_reason: StopReason::EndTurn,
                            })
                            .await;
                        return Ok(TurnResult::Done(StopReason::EndTurn));
                    }
                    // Non-prompt-too-long errors: fire StopFailure hook (if a
                    // runner is installed) so user-configured hooks see the
                    // failure, then propagate to the caller. Mirrors TS
                    // executeStopFailureHooks invocation. Fire-and-drop: the
                    // hook feedback is logged but does not change propagation.
                    let err_text = format!("{:#}", e);
                    let _ = crate::hooks::fire_stop_failure(&err_text).await;
                    return Err(e);
                }
            }
        };

        self.state = QueryState::Streaming;

        // Stream SSE events incrementally from the response body.
        // Instead of buffering the entire response, we read chunks as they
        // arrive, parse complete SSE events from them, and process each event
        // immediately.  This enables real-time text streaming to the TUI and
        // mid-response cancellation.
        //
        // Issue 27: wrap each chunk read in a tokio::time::timeout so that
        // a hung (but not closed) TCP connection doesn't block forever.
        let idle_timeout_ms: u64 = std::env::var("CLAUDE_STREAM_IDLE_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(STREAM_IDLE_TIMEOUT_MS);
        let idle_timeout = tokio::time::Duration::from_millis(idle_timeout_ms);

        let mut byte_stream = response.bytes_stream();
        let mut line_buffer = String::new();
        let mut current_event_type: Option<String> = None;
        let mut current_data: Option<String> = None;

        let mut accumulator = ContentBlockAccumulator::new();
        let mut tool_use_blocks: Vec<ToolUseInfo> = Vec::new();
        // Issue 6: mirrors TS needsFollowUp — set when any tool_use block is observed,
        // regardless of stop_reason (which is unreliable per API docs).
        let mut needs_follow_up = false;
        let mut stop_reason = StopReason::EndTurn;
        let mut assistant_content: Vec<serde_json::Value> = Vec::new();
        let mut assistant_blocks: Vec<ContentBlock> = Vec::new();
        let mut response_message: Option<ApiMessage> = None;
        let mut response_usage: Option<Usage> = None;
        'stream: loop {
            // Issue 27: apply idle timeout to each chunk read.
            let maybe_chunk = match tokio::time::timeout(idle_timeout, byte_stream.next()).await {
                Ok(maybe) => maybe,
                Err(_elapsed) => {
                    tracing::warn!(
                        "Streaming idle timeout: no chunks received for {}s, aborting stream",
                        idle_timeout_ms / 1000
                    );
                    self.state = QueryState::Terminal {
                        stop_reason: StopReason::EndTurn,
                        transition: TransitionReason::Error(
                            crate::types::error::QueryError::StreamIdleTimeout,
                        ),
                    };
                    let _ = event_tx
                        .send(StreamEvent::Done {
                            stop_reason: StopReason::EndTurn,
                        })
                        .await;
                    return Ok(TurnResult::Done(StopReason::EndTurn));
                }
            };

            let chunk = match maybe_chunk {
                Some(c) => c,
                None => break, // stream ended
            };

            // Issue 9: check cancellation after each chunk; if tool_use blocks are
            // already buffered, emit synthetic tool_result messages so the conversation
            // history stays well-formed (avoids API 400 on the next turn).
            if self.cancel.is_cancelled() {
                // Commit the partial assistant message so the API sees it.
                if !assistant_content.is_empty() {
                    self.add_assistant_message(assistant_content.clone());
                }
                // For every tool_use that was already announced, add a synthetic
                // tool_result so the conversation stays well-formed.
                if !tool_use_blocks.is_empty() {
                    let mut synthetic_results: Vec<serde_json::Value> = Vec::new();
                    for block in &tool_use_blocks {
                        synthetic_results.push(serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": block.id,
                            "content": [{"type": "text", "text": CANCEL_MSG}],
                            "is_error": true,
                        }));
                    }
                    self.messages.push(serde_json::json!({
                        "role": "user",
                        "content": synthetic_results,
                    }));
                }
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
                        match sse::parse_sse_event(&event_type, &data) {
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to parse SSE event (type={:?}): {}",
                                    event_type,
                                    e
                                );
                            }
                            Ok(event) => match event {
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
                                        assistant_blocks.push(block.clone());
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
                                                // Issue 6: set needs_follow_up regardless of
                                                // stop_reason (stop_reason == "tool_use" is
                                                // unreliable per API docs — mirrors TS needsFollowUp).
                                                needs_follow_up = true;
                                                assistant_content.push(serde_json::json!({
                                                    "type": "tool_use",
                                                    "id": id,
                                                    "name": name,
                                                    "input": input,
                                                }));
                                            }
                                            ContentBlock::ServerToolUse { id, name, input } => {
                                                assistant_content.push(serde_json::json!({
                                                    "type": "server_tool_use",
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
                                            // Issue 6: route model_context_window_exceeded
                                            // through same path as max_tokens.
                                            "model_context_window_exceeded" => {
                                                StopReason::ModelContextWindowExceeded
                                            }
                                            "pause_turn" => StopReason::PauseTurn,
                                            _ => StopReason::EndTurn,
                                        };
                                    }
                                    if let Some(u) = usage {
                                        if let Some(accumulated) = response_usage.as_mut() {
                                            accumulated.output_tokens = u.output_tokens;
                                        }
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
                                    break 'stream;
                                }
                                SseEvent::MessageStart { message } => {
                                    response_usage = Some(message.usage.clone());
                                    response_message = Some(message.clone());
                                    let _ = event_tx
                                        .send(StreamEvent::UsageUpdate(message.usage.clone()))
                                        .await;
                                }
                                SseEvent::Error { message } => {
                                    tracing::error!("Streaming API error mid-stream: {}", message);
                                    let error =
                                        crate::types::error::QueryError::Api { status: 0, message };
                                    let _ = event_tx.send(StreamEvent::Error(error.clone())).await;
                                    self.state = QueryState::Terminal {
                                        stop_reason: StopReason::EndTurn,
                                        transition: TransitionReason::Error(error.clone()),
                                    };
                                    return Err(anyhow::anyhow!(
                                        "Streaming API error received mid-stream: {}",
                                        error
                                    ));
                                }
                                SseEvent::MessageStop => {
                                    break 'stream;
                                }
                                _ => {}
                            },
                        }
                    }
                }
            }
        }

        // Add assistant message to history
        if !assistant_content.is_empty() {
            if let Some(mut message) = response_message {
                message.content = assistant_blocks;
                message.stop_reason = Some(stop_reason.clone());
                if let Some(usage) = response_usage.clone() {
                    message.usage = usage;
                }
                let _ = event_tx
                    .send(StreamEvent::AssistantMessage(AssistantMessage {
                        uuid: uuid::Uuid::new_v4(),
                        message,
                        request_id: None,
                        timestamp: chrono::Utc::now(),
                    }))
                    .await;
            }
            self.add_assistant_message(assistant_content);
            if let Some(usage) = response_usage {
                self.usage_anchor = Some(UsageAnchor {
                    message_count: self.messages.len(),
                    usage,
                });
            }
        }

        // Issue 6: Do NOT rely on stop_reason == ToolUse (unreliable per API docs).
        // Use needs_follow_up, which is set whenever any tool_use block arrived.
        // ModelContextWindowExceeded is treated like MaxTokens.
        if needs_follow_up {
            self.state = QueryState::ExecutingTools;
            Ok(TurnResult::ToolUse(tool_use_blocks))
        } else {
            match stop_reason {
                StopReason::MaxTokens | StopReason::ModelContextWindowExceeded => {
                    self.handle_max_tokens(event_tx).await
                }
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
    }

    async fn handle_max_tokens(
        &mut self,
        event_tx: &mpsc::Sender<StreamEvent>,
    ) -> Result<TurnResult> {
        // Stage 1: One-shot escalation (8k → 64k).
        // Issue 7: Guard with has_escalated_max_tokens (never cleared) to prevent
        // the escalation from re-firing after Stage 2 clears max_output_tokens_override.
        if !self.has_escalated_max_tokens {
            self.has_escalated_max_tokens = true; // never cleared
            self.max_output_tokens_override = Some(ESCALATED_MAX_TOKENS);
            self.state = QueryState::RecoveringMaxTokens {
                recovery_count: self.recovery_count,
                escalated: true,
            };
            return Ok(TurnResult::ContinueRecovery);
        }

        // Stage 2: Recovery loop (up to MAX_OUTPUT_TOKENS_RECOVERY_LIMIT).
        // Clear the escalated override — go back to default max_tokens (matches TS:
        // maxOutputTokensOverride: undefined in the stage-2 state transition).
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

/// Build a `<system-reminder>` prepend content block containing dynamic context.
/// Mirrors TS prependUserContext() in src/utils/api.ts.
/// Injects the current date so the model always has up-to-date temporal context.
/// Returns None if no context is available (currently always returns Some).
fn build_user_context_block() -> Option<serde_json::Value> {
    use chrono::Local;
    let current_date = format!("Today's date is {}.", Local::now().format("%a %b %d %Y"));

    let inner = format!("# currentDate\n{}", current_date);
    let content = format!(
        "<system-reminder>\nAs you answer the user's questions, you can use the following context:\n{}\n\nIMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</system-reminder>",
        inner
    );

    Some(serde_json::json!({"type": "text", "text": content}))
}

/// Build the request message list with dynamic user context.
///
/// TS `prependUserContext()` prepends system-reminder content blocks to the
/// first user message rather than creating a separate user turn.
fn build_messages_for_query(
    messages: &[serde_json::Value],
    extra_context_blocks: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut context_blocks = extra_context_blocks.to_vec();
    if context_blocks.is_empty() {
        if let Some(context_block) = build_user_context_block() {
            context_blocks.push(context_block);
        }
    }
    if context_blocks.is_empty() {
        return messages.to_vec();
    };

    let mut result = messages.to_vec();
    if let Some(first_user) = result
        .iter_mut()
        .find(|msg| msg.get("role").and_then(|role| role.as_str()) == Some("user"))
    {
        match first_user.get_mut("content") {
            Some(serde_json::Value::Array(content)) => {
                content.splice(0..0, context_blocks);
            }
            Some(content) => {
                let existing = content.take();
                context_blocks.push(existing);
                *content = serde_json::Value::Array(context_blocks);
            }
            None => {
                first_user["content"] = serde_json::Value::Array(context_blocks);
            }
        }
        return result;
    }

    let mut with_context = Vec::with_capacity(result.len() + 1);
    with_context.push(serde_json::json!({
        "role": "user",
        "content": context_blocks,
    }));
    with_context.extend(result);
    with_context
}

/// Filter `messages` to only those from the last compact boundary onward.
///
/// If no compact boundary exists, all messages are returned unchanged.
/// Matches TS `getMessagesAfterCompactBoundary` in `src/utils/messages.ts:4643`.
fn filter_after_compact_boundary(messages: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    // Scan backward to find the last compact_boundary marker.
    // A compact_boundary message may have:
    //   - type == "compact_boundary" (Rust session format)
    //   - subtype == "compact_boundary" (TS session format for SystemCompactBoundaryMessage)
    let boundary_idx = messages.iter().rposition(|msg| {
        msg.get("type")
            .and_then(|t| t.as_str())
            .map(|t| t == "compact_boundary")
            .unwrap_or(false)
            || msg
                .get("subtype")
                .and_then(|s| s.as_str())
                .map(|s| s == "compact_boundary")
                .unwrap_or(false)
    });

    match boundary_idx {
        Some(idx) => messages[idx..].to_vec(),
        None => messages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_context_is_prepended_to_first_user_message() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })];

        let with_context = build_messages_for_query(&messages, &[]);

        assert_eq!(with_context.len(), 1);
        assert_eq!(with_context[0]["role"], "user");
        let content = with_context[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert!(content[0]["text"]
            .as_str()
            .unwrap()
            .contains("<system-reminder>"));
        assert_eq!(content[1]["text"], "hi");
    }

    #[test]
    fn user_context_preserves_history_when_no_user_message_exists() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "hello"}]
        })];

        let with_context = build_messages_for_query(&messages, &[]);

        assert_eq!(with_context.len(), 2);
        assert_eq!(with_context[0]["role"], "user");
        assert_eq!(with_context[1]["role"], "assistant");
    }

    #[test]
    fn extra_user_context_blocks_are_prepended_in_order() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "hi"}]
        })];
        let extra = vec![
            serde_json::json!({"type": "text", "text": "first"}),
            serde_json::json!({"type": "text", "text": "second"}),
        ];

        let with_context = build_messages_for_query(&messages, &extra);
        let content = with_context[0]["content"].as_array().unwrap();

        assert_eq!(content[0]["text"], "first");
        assert_eq!(content[1]["text"], "second");
        assert_eq!(content[2]["text"], "hi");
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
