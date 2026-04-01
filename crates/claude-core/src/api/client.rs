use anyhow::Result;
use reqwest::Response;
use serde_json::{json, Value};

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
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-4-6".into(),
            max_tokens: 64000,
            thinking: ThinkingConfig::Adaptive,
            speed: None,
            api_version: "2023-06-01".into(),
        }
    }
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
    let thinking_obj = match &config.thinking {
        ThinkingConfig::Disabled => None,
        ThinkingConfig::Enabled { budget_tokens } => Some(json!({
            "type": "enabled",
            "budget_tokens": budget_tokens,
        })),
        ThinkingConfig::Adaptive => Some(json!({ "type": "adaptive" })),
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
    let session_id = uuid::Uuid::new_v4().to_string();
    let user_id_obj = json!({
        "device_id": device_id,
        "account_uuid": "",
        "session_id": session_id,
    });
    body["metadata"] = json!({
        "user_id": user_id_obj.to_string(),
    });

    // context_management: mirrors TS getAPIContextManagement().
    // For adaptive thinking, send clear_thinking strategy keeping all turns.
    if matches!(config.thinking, ThinkingConfig::Adaptive | ThinkingConfig::Enabled { .. }) {
        body["context_management"] = json!({
            "edits": [
                {
                    "type": "clear_thinking_20251015",
                    "keep": "all"
                }
            ]
        });
    }

    // output_config: empty object matches TS behaviour when no effort/budget is set.
    body["output_config"] = json!({});

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
    pub fn new(config: ApiConfig, auth: AuthMethod) -> Self {
        Self {
            config,
            auth,
            http: reqwest::Client::new(),
        }
    }

    /// POST a streaming request to `/v1/messages` and return the raw response.
    ///
    /// The caller is responsible for reading the SSE stream from the response body.
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

        let (header_name, header_value) = self.auth.to_header();

        let mut request = self
            .http
            .post(&url)
            .header("anthropic-version", &self.config.api_version)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("user-agent", "claude-cli/2.1.88 (external, cli)")
            .header("x-stainless-lang", "js")
            .header("x-stainless-package-version", "2.2.0")
            .header("x-stainless-runtime", "node")
            .header("x-stainless-retry-count", "0")
            .header(header_name, header_value);

        // Build anthropic-beta header (matches TS betas.ts constants)
        let mut betas = vec![
            "claude-code-20250219",
            "interleaved-thinking-2025-05-14",
            "context-management-2025-06-27",
            "prompt-caching-scope-2026-01-05",
            "effort-2025-11-24",
            "web-search-2025-03-05",
            "token-efficient-tools-2026-03-28",
        ];
        if self.auth.is_oauth() {
            betas.push("oauth-2025-04-20");
        }
        request = request
            .header("anthropic-beta", betas.join(","))
            .header("anthropic-dangerous-direct-browser-access", "true")
            .header("x-app", "cli");

        let response = request.json(&body).send().await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let err_body = response.text().await.unwrap_or_default();
            tracing::error!("API error {}: {}", status, &err_body[..err_body.len().min(500)]);
            anyhow::bail!("API error {}: {}", status, &err_body[..err_body.len().min(500)]);
        }

        Ok(response)
    }
}
