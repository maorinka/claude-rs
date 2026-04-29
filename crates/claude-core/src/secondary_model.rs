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
use crate::types::content::ContentBlock;
use serde_json::json;

/// A minimal secondary-model interface: take a fully-formed user prompt,
/// return the model's text response.
#[async_trait]
pub trait SecondaryModel: Send + Sync {
    async fn summarize(&self, user_prompt: &str, cancel: CancellationToken) -> Result<String>;

    async fn web_search(
        &self,
        query: &str,
        allowed_domains: Option<Vec<String>>,
        blocked_domains: Option<Vec<String>>,
        cancel: CancellationToken,
    ) -> Result<String> {
        let _ = (query, allowed_domains, blocked_domains, cancel);
        Err(anyhow!("web search secondary model is not available"))
    }
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
    web_search_client: ApiClient,
}

impl HaikuSecondaryModel {
    /// Build a Haiku client by cloning auth + base_url from the given
    /// caller-provided auth and reusing the session id.
    pub fn new(auth: AuthMethod, base_url: String, session_id: String, main_model: String) -> Self {
        let config = ApiConfig {
            base_url: base_url.clone(),
            model: "claude-haiku-4-5".into(),
            fallback_model: None,
            max_tokens: 2048,
            thinking: ThinkingConfig::Disabled,
            speed: None,
            api_version: "2023-06-01".into(),
            session_id: session_id.clone(),
            account_uuid: String::new(),
            effort: None,
            task_budget_total: None,
            workload: None,
            sdk_betas: Vec::new(),
        };
        let web_search_config = ApiConfig {
            base_url,
            model: main_model,
            fallback_model: None,
            max_tokens: 64_000,
            thinking: ThinkingConfig::Disabled,
            speed: None,
            api_version: "2023-06-01".into(),
            session_id,
            account_uuid: String::new(),
            effort: None,
            task_budget_total: None,
            workload: None,
            sdk_betas: Vec::new(),
        };
        Self {
            client: ApiClient::new(config, auth.clone()),
            web_search_client: ApiClient::new(web_search_config, auth),
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

        if out.is_empty() {
            Ok("No response from model".to_string())
        } else {
            Ok(out)
        }
    }

    async fn web_search(
        &self,
        query: &str,
        allowed_domains: Option<Vec<String>>,
        blocked_domains: Option<Vec<String>>,
        cancel: CancellationToken,
    ) -> Result<String> {
        let messages = vec![json!({
            "role": "user",
            "content": [{"type": "text", "text": format!("Perform a web search for the query: {}", query)}],
        })];
        let system = vec![ContentBlock::Text {
            text: "You are an assistant for performing a web search tool use".into(),
        }];
        let mut tool_schema = json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 8,
        });
        if let Some(domains) = allowed_domains.filter(|domains| !domains.is_empty()) {
            tool_schema["allowed_domains"] = json!(domains);
        }
        if let Some(domains) = blocked_domains.filter(|domains| !domains.is_empty()) {
            tool_schema["blocked_domains"] = json!(domains);
        }

        let raw_tools = vec![tool_schema];
        let response = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(anyhow!("web search cancelled"));
            }
            res = self.web_search_client.stream_request_with_raw_tools(&messages, &system, &raw_tools) => res?,
        };

        let mut byte_stream = response.bytes_stream();
        let mut buf = String::new();
        let mut current_event = None;
        let mut current_data = None;
        let mut text = String::new();
        let mut links: Vec<serde_json::Value> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        while let Some(chunk) = byte_stream.next().await {
            if cancel.is_cancelled() {
                return Err(anyhow!("web search cancelled"));
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
                        if ev == "content_block_start" {
                            collect_web_search_result_links(&data, &mut links, &mut errors);
                        }
                        if let Ok(SseEvent::ContentBlockDelta {
                            delta: ContentDelta::TextDelta { text: delta },
                            ..
                        }) = parse_sse_event(&ev, &data)
                        {
                            text.push_str(&delta);
                        }
                    }
                }
            }
        }

        Ok(format_web_search_result(query, &text, &links, &errors))
    }
}

fn collect_web_search_result_links(
    data: &str,
    links: &mut Vec<serde_json::Value>,
    errors: &mut Vec<String>,
) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
        return;
    };
    let Some(content_block) = value.get("content_block") else {
        return;
    };
    if content_block.get("type").and_then(|v| v.as_str()) != Some("web_search_tool_result") {
        return;
    }

    match content_block.get("content") {
        Some(serde_json::Value::Array(results)) => {
            for result in results {
                let title = result.get("title").and_then(|v| v.as_str());
                let url = result.get("url").and_then(|v| v.as_str());
                if let (Some(title), Some(url)) = (title, url) {
                    links.push(json!({ "title": title, "url": url }));
                }
            }
        }
        Some(serde_json::Value::Object(obj)) => {
            if let Some(code) = obj.get("error_code").and_then(|v| v.as_str()) {
                errors.push(format!("Web search error: {}", code));
            }
        }
        _ => {}
    }
}

fn format_web_search_result(
    query: &str,
    text: &str,
    links: &[serde_json::Value],
    errors: &[String],
) -> String {
    let mut out = format!("Web search results for query: \"{}\"\n\n", query);
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        out.push_str(trimmed);
        out.push_str("\n\n");
    }
    for err in errors {
        out.push_str(err);
        out.push_str("\n\n");
    }
    if links.is_empty() {
        out.push_str("No links found.\n\n");
    } else {
        out.push_str("Links: ");
        out.push_str(&serde_json::to_string(links).unwrap_or_else(|_| "[]".into()));
        out.push_str("\n\n");
    }
    out.push_str(
        "REMINDER: You MUST include the sources above in your response to the user using markdown hyperlinks.",
    );
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_web_search_result_links_from_content_block_start() {
        let data = r#"{
            "type": "content_block_start",
            "index": 1,
            "content_block": {
                "type": "web_search_tool_result",
                "tool_use_id": "srv_1",
                "content": [
                    {"title": "Example", "url": "https://example.com", "encrypted_content": "x"}
                ]
            }
        }"#;
        let mut links = Vec::new();
        let mut errors = Vec::new();
        collect_web_search_result_links(data, &mut links, &mut errors);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0]["title"], "Example");
        assert_eq!(links[0]["url"], "https://example.com");
        assert!(errors.is_empty());
    }

    #[test]
    fn formats_web_search_result_with_sources_reminder() {
        let links = vec![json!({"title": "Example", "url": "https://example.com"})];
        let out = format_web_search_result("rust", "Summary", &links, &[]);
        assert!(out.contains("Web search results for query: \"rust\""));
        assert!(out.contains("Summary"));
        assert!(out.contains("Links:"));
        assert!(out.contains("REMINDER: You MUST include the sources above"));
    }
}
