use anyhow::Result;
use once_cell::sync::Lazy;
use reqwest::Response;
use serde_json::{json, Value};

use crate::auth::login::{debug_http_client, proxy_url};
use crate::types::content::ContentBlock;

// ── Public types ──────────────────────────────────────────────────────────────

/// Authentication method for the Anthropic API.
///
/// Matches the TS `getAnthropicClient()` in `src/services/api/client.ts`:
/// - Console users: `apiKey` param (sent as `x-api-key` header)
/// - Claude.ai subscribers: `authToken` param (sent as `Authorization: Bearer` header)
///   and requires `anthropic-beta: oauth-2025-04-20` header
#[derive(Clone, Debug)]
pub enum AuthMethod {
    /// A standard API key (x-api-key header).
    /// Used by Console users and ANTHROPIC_API_KEY env var.
    ApiKey(String),
    /// An OAuth bearer token (Authorization: Bearer header).
    /// Used by Claude.ai subscribers (Pro/Max/Team/Enterprise).
    /// Corresponds to the Anthropic SDK's `authToken` parameter.
    OAuthToken(String),
}

impl AuthMethod {
    /// Return the `(header_name, header_value)` pair for this auth method.
    ///
    /// - ApiKey -> `x-api-key: <key>` (matches TS SDK's apiKey param)
    /// - OAuthToken -> `Authorization: Bearer <token>` (matches TS SDK's authToken param)
    pub fn to_header(&self) -> (&'static str, String) {
        match self {
            AuthMethod::ApiKey(key) => ("x-api-key", key.clone()),
            AuthMethod::OAuthToken(token) => ("authorization", format!("Bearer {}", token)),
        }
    }

    /// Whether this auth method requires the OAuth beta header.
    /// The TS code adds `oauth-2025-04-20` to anthropic-beta when `isClaudeAISubscriber()`.
    pub fn is_oauth(&self) -> bool {
        matches!(self, AuthMethod::OAuthToken(_))
    }
}

/// Thinking / extended reasoning configuration.
#[derive(Clone, Debug, Default)]
pub enum ThinkingConfig {
    /// Disable extended thinking (default).
    #[default]
    Disabled,
    /// Enable extended thinking with a given token budget.
    Enabled { budget_tokens: u64 },
    /// Let the model decide adaptively.
    Adaptive,
}

/// Speed hint for the request.
#[derive(Clone, Debug)]
pub enum Speed {
    /// Optimise for lower latency.
    Fast,
    /// Optimise for higher quality / throughput.
    Standard,
}

/// Configuration for a single API request / session.
#[derive(Clone, Debug)]
pub struct ApiConfig {
    /// Base URL for the Anthropic API.
    pub base_url: String,
    /// Model identifier.
    pub model: String,
    /// Maximum number of output tokens.
    pub max_tokens: u64,
    /// Thinking / extended-reasoning configuration.
    pub thinking: ThinkingConfig,
    /// Optional speed hint.
    pub speed: Option<Speed>,
    /// Anthropic API version header value.
    pub api_version: String,
    /// Stable process-scoped session id, matching TS bootstrap session behavior.
    pub session_id: String,
    /// OAuth account UUID when available.
    pub account_uuid: String,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.anthropic.com".into(),
            model: "claude-opus-4-6".into(),
            max_tokens: get_max_output_tokens_for_model("claude-opus-4-6"),
            thinking: ThinkingConfig::Adaptive,
            speed: None,
            api_version: "2023-06-01".into(),
            session_id: get_session_id().clone(),
            account_uuid: String::new(),
        }
    }
}

const CAPPED_DEFAULT_MAX_TOKENS: u64 = 8_192;

static PROCESS_SESSION_ID: Lazy<String> = Lazy::new(|| uuid::Uuid::new_v4().to_string());

pub fn get_session_id() -> &'static String {
    &PROCESS_SESSION_ID
}

pub fn get_max_output_tokens_for_model(model: &str) -> u64 {
    let lower = model.to_ascii_lowercase();
    let default_tokens = if lower.contains("opus-4-1") || lower.contains("opus-4") {
        32_000
    } else if lower.contains("claude-3-opus") {
        4_096
    } else if lower.contains("claude-3-sonnet") {
        8_192
    } else if lower.contains("claude-3-haiku") {
        4_096
    } else if lower.contains("3-5-sonnet") || lower.contains("3-5-haiku") {
        8_192
    } else if lower.contains("3-7-sonnet") {
        32_000
    } else {
        32_000
    };

    // Match TS slot-reservation cap: normal requests default to 8k unless the
    // model's native default is lower.
    std::cmp::min(default_tokens, CAPPED_DEFAULT_MAX_TOKENS)
}

// ── Tool definition (for the request body) ───────────────────────────────────

/// A tool definition sent to the API.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

// ── Request body builder ──────────────────────────────────────────────────────

/// Build the JSON body for a `/v1/messages` streaming request.
///
/// Parameters:
/// - `config`   – request configuration.
/// - `messages` – conversation history as `(role, content_blocks)` pairs.
/// - `system`   – system prompt content blocks (may be empty).
/// - `tools`    – tool definitions (may be empty).
pub fn build_request_body(
    config: &ApiConfig,
    messages: &[Value],
    system: &[ContentBlock],
    tools: &[ToolDefinition],
) -> Value {
    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "stream": true,
        "messages": messages,
    });

    // Thinking configuration.
    // Haiku does not support adaptive thinking — only send for Sonnet/Opus.
    let supports_thinking = !config.model.contains("haiku");
    let thinking_obj = if supports_thinking {
        match &config.thinking {
            ThinkingConfig::Disabled => None,
            ThinkingConfig::Enabled { budget_tokens } => Some(json!({
                "type": "enabled",
                "budget_tokens": budget_tokens,
            })),
            ThinkingConfig::Adaptive => Some(json!({ "type": "adaptive" })),
        }
    } else {
        None
    };
    if let Some(thinking) = thinking_obj {
        body["thinking"] = thinking;
    }

    // Optional speed hint.
    if let Some(speed) = &config.speed {
        body["speed"] = match speed {
            Speed::Fast => json!("fast"),
            Speed::Standard => json!("standard"),
        };
    }

    // System prompt (only include if non-empty).
    if !system.is_empty() {
        body["system"] = serde_json::to_value(system).unwrap_or(Value::Null);
    }

    // Tools (only include if non-empty).
    if !tools.is_empty() {
        body["tools"] = serde_json::to_value(tools).unwrap_or(Value::Null);
    }

    // Add web_search server tool (matches TS WebSearchTool).
    // This is handled server-side by the API — not a regular tool_use/tool_result flow.
    body.as_object_mut()
        .unwrap()
        .entry("tools")
        .or_insert(json!([]));
    if let Some(tools_arr) = body["tools"].as_array_mut() {
        tools_arr.push(json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 8
        }));
    }

    // metadata: mirrors TS getAPIMetadata() — user_id is a JSON-encoded string
    // containing device_id, account_uuid, and session_id.
    let device_id = get_or_create_device_id();
    let user_id_obj = json!({
        "device_id": device_id,
        "account_uuid": config.account_uuid,
        "session_id": config.session_id,
    });
    body["metadata"] = json!({
        "user_id": user_id_obj.to_string(),
    });

    // context_management: mirrors TS getAPIContextManagement().
    // For adaptive thinking, send clear_thinking strategy keeping all turns.
    if supports_thinking
        && matches!(
            config.thinking,
            ThinkingConfig::Adaptive | ThinkingConfig::Enabled { .. }
        )
    {
        body["context_management"] = json!({
            "edits": [
                {
                    "type": "clear_thinking_20251015",
                    "keep": "all"
                }
            ]
        });
    }

    body
}

/// Get or create a persistent device ID (matches TS `getOrCreateUserID()`).
fn get_or_create_device_id() -> String {
    let config_dir = std::env::var("HOME")
        .map(|h| format!("{}/.claude", h))
        .unwrap_or_else(|_| "/tmp/.claude".to_string());
    let id_path = format!("{}/device_id", config_dir);

    // Try to read existing ID
    if let Ok(id) = std::fs::read_to_string(&id_path) {
        let id = id.trim().to_string();
        if !id.is_empty() {
            return id;
        }
    }

    // Generate and persist a new one
    let id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::create_dir_all(&config_dir);
    let _ = std::fs::write(&id_path, &id);
    id
}

// ── API client ────────────────────────────────────────────────────────────────

/// A thin HTTP client for the Anthropic Messages API.
pub struct ApiClient {
    pub config: ApiConfig,
    auth: AuthMethod,
    http: reqwest::Client,
}

impl ApiClient {
    /// Create a new `ApiClient` with the given configuration and auth method.
    pub fn new(mut config: ApiConfig, auth: AuthMethod) -> Self {
        config.base_url = proxy_url(&config.base_url);
        Self {
            config,
            auth,
            http: debug_http_client(),
        }
    }

    /// POST a streaming request to `/v1/messages` and return the raw response.
    ///
    /// Includes retry logic matching the TS `withRetry()` in
    /// `src/services/api/withRetry.ts`:
    /// - 429 (rate limit) and 529 (overloaded): retry with exponential backoff
    /// - 401 (unauthorized): refresh OAuth token and retry once
    /// - Other errors: fail immediately
    pub async fn stream_request(
        &self,
        messages: &[Value],
        system: &[ContentBlock],
        tools: &[ToolDefinition],
    ) -> Result<Response> {
        let url = format!("{}/v1/messages", self.config.base_url);
        let body = build_request_body(&self.config, messages, system, tools);

        // Debug mode: dump the full request body when CLAUDE_RS_DEBUG=1
        if std::env::var("CLAUDE_RS_DEBUG").as_deref() == Ok("1") {
            if let Ok(pretty) = serde_json::to_string_pretty(&body) {
                let _ = std::fs::write("/tmp/claude-rs-request.json", &pretty);
                tracing::debug!("Request body written to /tmp/claude-rs-request.json");
            }
        }

        // Retry configuration matching TS withRetry defaults:
        // - Max 8 attempts for transient errors (429/529)
        // - Exponential backoff starting at 1s, max 60s
        // - One 401 refresh retry
        const MAX_RETRIES: u32 = 8;
        const BASE_DELAY_MS: u64 = 1000;
        const MAX_DELAY_MS: u64 = 60_000;

        let mut has_retried_after_401 = false;
        let mut retry_count: u32 = 0;

        loop {
            let auth = if has_retried_after_401 {
                if let AuthMethod::OAuthToken(_) = &self.auth {
                    crate::auth::resolve::resolve_stored_oauth_token(false)
                        .await?
                        .map(AuthMethod::OAuthToken)
                        .unwrap_or_else(|| self.auth.clone())
                } else {
                    self.auth.clone()
                }
            } else {
                self.auth.clone()
            };
            let (header_name, header_value) = auth.to_header();

            let mut request = self
                .http
                .post(&url)
                .header("anthropic-version", &self.config.api_version)
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .header("user-agent", "claude-cli/2.1.88 (external, cli)")
                .header("x-claude-code-session-id", &self.config.session_id)
                .header("x-stainless-lang", "js")
                .header("x-stainless-package-version", "2.2.0")
                .header("x-stainless-runtime", "node")
                .header("x-stainless-retry-count", retry_count.to_string())
                .header(header_name, header_value);

            // Beta headers matching TS getAllModelBetas() for firstParty + OAuth:
            let mut betas = vec![
                "claude-code-20250219",
                "interleaved-thinking-2025-05-14",
                "context-management-2025-06-27",
                "prompt-caching-scope-2026-01-05",
            ];
            if auth.is_oauth() {
                betas.push("oauth-2025-04-20");
            }
            request = request
                .header("anthropic-beta", betas.join(","))
                .header("anthropic-dangerous-direct-browser-access", "true")
                .header("x-app", "cli");

            let response = request.json(&body).send().await?;

            if response.status().is_success() {
                return Ok(response);
            }

            let status = response.status().as_u16();

            // 401: try refreshing OAuth token once (matching TS handleOAuth401Error)
            if status == 401 && !has_retried_after_401 {
                has_retried_after_401 = true;
                if let AuthMethod::OAuthToken(failed_token) = &auth {
                    if crate::auth::resolve::handle_oauth_401_error(failed_token).await? {
                        tracing::info!("OAuth token refreshed after 401, retrying");
                        continue;
                    }
                }
            }

            // 429 or 529: retry with exponential backoff (matching TS withRetry)
            if (status == 429 || status == 529) && retry_count < MAX_RETRIES {
                retry_count += 1;

                // Parse retry-after header if present, otherwise use exponential backoff
                let delay_ms = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|secs| secs * 1000)
                    .unwrap_or_else(|| {
                        let exp = BASE_DELAY_MS * 2u64.pow(retry_count - 1);
                        exp.min(MAX_DELAY_MS)
                    });

                tracing::warn!(
                    "Rate limited ({}), retry {}/{} after {}ms",
                    status,
                    retry_count,
                    MAX_RETRIES,
                    delay_ms
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                continue;
            }

            let err_body = response.text().await.unwrap_or_default();
            tracing::error!(
                "API error {}: {}",
                status,
                &err_body[..err_body.len().min(500)]
            );
            anyhow::bail!(
                "API error {}: {}",
                status,
                &err_body[..err_body.len().min(500)]
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_output_tokens_match_ts_slot_reservation_cap() {
        assert_eq!(get_max_output_tokens_for_model("claude-sonnet-4-6"), 8_192);
        assert_eq!(get_max_output_tokens_for_model("claude-opus-4-6"), 8_192);
        assert_eq!(get_max_output_tokens_for_model("claude-haiku-4-5"), 8_192);
        assert_eq!(
            get_max_output_tokens_for_model("claude-3-opus-20240229"),
            4_096
        );
    }

    #[test]
    fn request_metadata_uses_stable_session_and_account_uuid() {
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            session_id: "session-123".into(),
            account_uuid: "account-456".into(),
            ..Default::default()
        };
        let body = build_request_body(&config, &[], &[], &[]);
        let user_id = body["metadata"]["user_id"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(user_id).unwrap();
        assert_eq!(parsed["session_id"], "session-123");
        assert_eq!(parsed["account_uuid"], "account-456");
    }
}
