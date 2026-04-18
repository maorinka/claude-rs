//! Optional secondary ("fast/small") model used by tools that need a cheap
//! LLM call for post-processing — e.g. WebFetchTool's `applyPromptToMarkdown`
//! which summarises fetched content against a user prompt.
//!
//! Mirrors TS `services/api/claude.ts::queryHaiku()` from the reference
//! implementation. Parked behind a trait + global OnceLock so tools in
//! `claude-tools/` can use it without plumbing an ApiClient through every
//! `ToolUseContext` construction site.
//!
//! The application entry point (CLI/TUI) is responsible for calling
//! `set_global()` once at startup to wire in a concrete implementation.
//! Tools that call `get_global()` when no model is registered gracefully
//! fall back to a no-op path.

use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::api::client::{ApiClient, ApiConfig, AuthMethod, ThinkingConfig};
use crate::api::sse::{parse_sse_event, ContentDelta, SseEvent};
use serde_json::json;

/// A minimal secondary-model interface: take a fully-formed user prompt,
/// return the model's text response.
#[async_trait]
pub trait SecondaryModel: Send + Sync {
    async fn summarize(&self, user_prompt: &str, cancel: CancellationToken) -> Result<String>;
}

static GLOBAL: OnceLock<Arc<dyn SecondaryModel>> = OnceLock::new();

/// Register the process-wide secondary model. Only the first call wins —
/// subsequent calls are silently ignored (matches `OnceLock` semantics).
pub fn set_global(model: Arc<dyn SecondaryModel>) {
    let _ = GLOBAL.set(model);
}

/// Fetch the registered secondary model if one has been installed.
pub fn get_global() -> Option<Arc<dyn SecondaryModel>> {
    GLOBAL.get().cloned()
}

// ── HaikuSecondaryModel: concrete impl backed by ApiClient ───────────────────

/// A SecondaryModel backed by an `ApiClient` configured for Haiku.
/// Mirrors TS `queryHaiku()` which targets claude-haiku-4-5 for cheap, fast
/// post-processing of tool output (e.g. WebFetch summarisation).
pub struct HaikuSecondaryModel {
    client: ApiClient,
}

impl HaikuSecondaryModel {
    /// Build a Haiku client by cloning auth + base_url from the given
    /// caller-provided auth and reusing the session id.
    pub fn new(auth: AuthMethod, base_url: String, session_id: String) -> Self {
        let config = ApiConfig {
            base_url,
            model: "claude-haiku-4-5".into(),
            max_tokens: 2048,
            thinking: ThinkingConfig::Disabled,
            speed: None,
            api_version: "2023-06-01".into(),
            session_id,
            account_uuid: String::new(),
        };
        Self {
            client: ApiClient::new(config, auth),
        }
    }
}

#[async_trait]
impl SecondaryModel for HaikuSecondaryModel {
    async fn summarize(&self, user_prompt: &str, cancel: CancellationToken) -> Result<String> {
        let messages = vec![json!({
            "role": "user",
            "content": [{"type": "text", "text": user_prompt}],
        })];

        let response = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(anyhow!("secondary model call cancelled"));
            }
            res = self.client.stream_request(&messages, &[], &[]) => res?,
        };

        // Accumulate text deltas from the streaming response. We ignore every
        // other block type (ToolUse, Thinking) — the secondary model is told
        // to respond with plain text.
        let mut byte_stream = response.bytes_stream();
        let mut buf = String::new();
        let mut current_event = None;
        let mut current_data = None;
        let mut out = String::new();

        while let Some(chunk) = byte_stream.next().await {
            if cancel.is_cancelled() {
                return Err(anyhow!("secondary model call cancelled"));
            }
            let chunk = chunk?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim_end_matches('\r').to_string();
                buf = buf[nl + 1..].to_string();

                if let Some(rest) = line.strip_prefix("event:") {
                    current_event = Some(rest.trim().to_string());
                } else if let Some(rest) = line.strip_prefix("data:") {
                    current_data = Some(rest.trim().to_string());
                } else if line.is_empty() {
                    if let (Some(ev), Some(data)) = (current_event.take(), current_data.take()) {
                        if let Ok(SseEvent::ContentBlockDelta {
                            delta: ContentDelta::TextDelta { text },
                            ..
                        }) = parse_sse_event(&ev, &data)
                        {
                            out.push_str(&text);
                        }
                    }
                }
            }
        }

        Ok(out)
    }
}
