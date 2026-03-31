use anyhow::Result;
use reqwest::Response;
use serde_json::{json, Value};

use crate::types::content::ContentBlock;

// ── Public types ──────────────────────────────────────────────────────────────

/// Authentication method for the Anthropic API.
#[derive(Clone, Debug)]
pub enum AuthMethod {
    /// A standard API key (x-api-key header).
    ApiKey(String),
    /// An OAuth bearer token (Authorization: Bearer header).
    OAuthToken(String),
}

impl AuthMethod {
    /// Return the `(header_name, header_value)` pair for this auth method.
    pub fn to_header(&self) -> (&'static str, String) {
        match self {
            AuthMethod::ApiKey(key) => ("x-api-key", key.clone()),
            AuthMethod::OAuthToken(token) => ("authorization", format!("Bearer {}", token)),
        }
    }

    /// Whether this auth method requires the OAuth beta header.
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

    // Required fields matching real Claude Code request format
    body["metadata"] = json!({});
    body["output_config"] = json!({});

    body
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
        // The Anthropic SDK sends to /v1/messages?beta=true for beta.messages.create()
        let url = format!("{}/v1/messages?beta=true", self.config.base_url);
        let body = build_request_body(&self.config, messages, system, tools);

        let (header_name, header_value) = self.auth.to_header();

        let mut request = self
            .http
            .post(&url)
            .header("anthropic-version", &self.config.api_version)
            .header("content-type", "application/json")
            .header(header_name, header_value);

        // Build anthropic-beta header (matches real Claude Code's request exactly)
        let mut betas = vec![
            "claude-code-20250219",
            "interleaved-thinking-2025-05-14",
            "context-management-2025-06-27",
            "effort-2025-11-24",
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
            let body = response.text().await.unwrap_or_default();
            tracing::error!("API error {}: {}", status, &body[..body.len().min(500)]);
            anyhow::bail!("API error {}: {}", status, &body[..body.len().min(500)]);
        }

        Ok(response)
    }
}
