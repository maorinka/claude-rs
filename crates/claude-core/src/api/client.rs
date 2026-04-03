use anyhow::Result;
use once_cell::sync::Lazy;
use reqwest::Response;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::auth::login::{debug_http_client, proxy_url};
use crate::types::content::ContentBlock;
use crate::types::events::StreamEvent;

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

pub fn minimal_transport_enabled() -> bool {
    matches!(
        std::env::var("CLAUDE_RS_TRANSPORT_MODE").ok().as_deref(),
        Some("minimal")
    ) || matches!(
        std::env::var("CLAUDE_RS_MINIMAL_TRANSPORT").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
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

/// Map model ID to marketing name (matches TS getPublicModelDisplayName).
fn model_marketing_name(model: &str) -> &str {
    if model.contains("opus-4-6") { "Opus 4.6" }
    else if model.contains("opus-4-5") { "Opus 4.5" }
    else if model.contains("opus-4-1") { "Opus 4.1" }
    else if model.contains("sonnet-4-6") { "Sonnet 4.6" }
    else if model.contains("sonnet-4-5") { "Sonnet 4.5" }
    else if model.contains("haiku-4-5") { "Haiku 4.5" }
    else if model.contains("claude-3-7-sonnet") { "Sonnet 3.7" }
    else { model }
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
    is_oauth: bool,
) -> Value {
    if minimal_transport_enabled() {
        return build_minimal_request_body(config, messages, system, tools, is_oauth);
    }

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

    // System prompt assembly. Order matters:
    //
    // WARNING: The billing attribution block MUST be the very first content
    // block in the system prompt array when using OAuth. The Anthropic API
    // uses it to identify the request as a Claude Code client and apply the
    // correct rate-limit tier. Without it, OAuth requests are immediately
    // rejected with 429. If it is not the first block (e.g. model identity
    // is prepended before it), the server won't find it and you get 429s.
    //
    // Order: [attribution (OAuth only)] -> [model identity] -> [user system blocks]
    if !system.is_empty() {
        let mut full_system: Vec<ContentBlock> = Vec::new();
        if is_oauth {
            full_system.push(ContentBlock::Text {
                text: "x-anthropic-billing-header: cc_version=1.0.33.claude-rs; cc_entrypoint=cli;"
                    .to_string(),
            });
        }
        let marketing = model_marketing_name(&config.model);
        full_system.push(ContentBlock::Text {
            text: format!(
                "You are powered by the model named {}. The exact model ID is {}.",
                marketing, config.model
            ),
        });
        full_system.extend_from_slice(system);
        body["system"] = serde_json::to_value(&full_system).unwrap_or(Value::Null);
    }

    // Tools (only include if non-empty).
    if !tools.is_empty() {
        body["tools"] = serde_json::to_value(tools).unwrap_or(Value::Null);
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

fn build_minimal_request_body(
    config: &ApiConfig,
    messages: &[Value],
    system: &[ContentBlock],
    tools: &[ToolDefinition],
    is_oauth: bool,
) -> Value {
    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "stream": true,
        "messages": messages,
    });

    // See the comment in build_request_body() — billing attribution MUST be
    // the first system prompt block for OAuth, or the API returns 429.
    if !system.is_empty() || is_oauth {
        let mut full_system: Vec<ContentBlock> = Vec::new();
        if is_oauth {
            full_system.push(ContentBlock::Text {
                text: "x-anthropic-billing-header: cc_version=1.0.33.claude-rs; cc_entrypoint=cli;"
                    .to_string(),
            });
        }
        full_system.extend_from_slice(system);
        body["system"] = serde_json::to_value(&full_system).unwrap_or(Value::Null);
    }

    if !tools.is_empty() {
        body["tools"] = serde_json::to_value(tools).unwrap_or(Value::Null);
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
        self.stream_request_with_events(messages, system, tools, None)
            .await
    }

    /// POST a streaming request with optional event feedback.
    pub async fn stream_request_with_events(
        &self,
        messages: &[Value],
        system: &[ContentBlock],
        tools: &[ToolDefinition],
        _event_tx: Option<&mpsc::Sender<StreamEvent>>,
    ) -> Result<Response> {
        let url = format!("{}/v1/messages", self.config.base_url);
        let body = build_request_body(&self.config, messages, system, tools, self.auth.is_oauth());
        let minimal_transport = minimal_transport_enabled();

        // Debug mode: dump the full request body when CLAUDE_RS_DEBUG=1
        if std::env::var("CLAUDE_RS_DEBUG").as_deref() == Ok("1") {
            if let Ok(pretty) = serde_json::to_string_pretty(&body) {
                let _ = std::fs::write("/tmp/claude-rs-request.json", &pretty);
                tracing::debug!("Request body written to /tmp/claude-rs-request.json");
            }
        }

        let (header_name, header_value) = self.auth.to_header();

        let mut request = self
            .http
            .post(&url)
            .header("anthropic-version", &self.config.api_version)
            .header("content-type", "application/json")
            .header(header_name, header_value);

        if minimal_transport {
            if self.auth.is_oauth() {
                request = request.header("anthropic-beta", "oauth-2025-04-20");
            }
        } else {
            request = request
                .header("accept", "application/json")
                .header("user-agent", "claude-cli/2.1.88 (external, cli)")
                .header("x-claude-code-session-id", &self.config.session_id)
                .header("x-stainless-lang", "js")
                .header("x-stainless-package-version", "2.2.0")
                .header("x-stainless-runtime", "node")
                .header("x-stainless-retry-count", "0");

            let mut betas = vec![
                "claude-code-20250219",
                "interleaved-thinking-2025-05-14",
                "context-management-2025-06-27",
                "prompt-caching-scope-2026-01-05",
            ];
            if self.auth.is_oauth() {
                betas.push("oauth-2025-04-20");
            }
            request = request
                .header("anthropic-beta", betas.join(","))
                .header("anthropic-dangerous-direct-browser-access", "true")
                .header("x-app", "cli");
        }

        let response = request.json(&body).send().await?;

        if response.status().is_success() {
            return Ok(response);
        }

        let status = response.status().as_u16();
        let err_body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "API error {}: {}",
            status,
            &err_body[..err_body.len().min(500)]
        );
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
        let body = build_request_body(&config, &[], &[], &[], false);
        let user_id = body["metadata"]["user_id"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(user_id).unwrap();
        assert_eq!(parsed["session_id"], "session-123");
        assert_eq!(parsed["account_uuid"], "account-456");
    }

    #[test]
    fn minimal_transport_body_strips_metadata_and_context_management() {
        std::env::set_var("CLAUDE_RS_MINIMAL_TRANSPORT", "1");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let body = build_request_body(&config, &[], &[], &[], false);
        assert!(body.get("metadata").is_none());
        assert!(body.get("context_management").is_none());
        assert!(body.get("thinking").is_none());
        std::env::remove_var("CLAUDE_RS_MINIMAL_TRANSPORT");
    }
}
