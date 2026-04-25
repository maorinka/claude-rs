use anyhow::Result;
use once_cell::sync::Lazy;
use rand::RngCore;
use reqwest::Response;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::auth::login::{debug_http_client, proxy_url};
use crate::types::content::ContentBlock;
use crate::types::events::StreamEvent;

// ── Model normalization (mirrors TS utils/model/model.ts) ────────────────────

/// Strip `[1m]`/`[2m]` context window suffixes before sending to the API.
/// Mirrors TS `normalizeModelStringForAPI()`: `model.replace(/\[(1|2)m\]/gi, '')`
fn normalize_model_for_api(model: &str) -> String {
    model
        .replace("[1m]", "")
        .replace("[1M]", "")
        .replace("[2m]", "")
        .replace("[2M]", "")
}

/// Check if a model string has the `[1m]` context window suffix.
/// Mirrors TS `has1mContext()`: `/\[1m\]/i.test(model)`
fn has_1m_context(model: &str) -> bool {
    model.contains("[1m]") || model.contains("[1M]")
}

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
    if lower.contains("opus-4-6")
        || lower.contains("sonnet-4-6")
        || lower.contains("haiku-4-5")
    {
        64_000
    } else if lower.contains("opus-4-1") || lower.contains("opus-4") {
        32_000
    } else if lower.contains("claude-3-opus") {
        4_096
    } else if lower.contains("claude-3-sonnet") {
        8_192
    } else if lower.contains("claude-3-haiku") {
        4_096
    } else if lower.contains("3-5-sonnet") || lower.contains("3-5-haiku") {
        8_192
    } else {
        32_000
    }
}

/// Map model ID to marketing name (matches TS getPublicModelDisplayName).
fn model_marketing_name(model: &str) -> &str {
    if model.contains("opus-4-6") {
        "Opus 4.6"
    } else if model.contains("opus-4-5") {
        "Opus 4.5"
    } else if model.contains("opus-4-1") {
        "Opus 4.1"
    } else if model.contains("sonnet-4-6") {
        "Sonnet 4.6"
    } else if model.contains("sonnet-4-5") {
        "Sonnet 4.5"
    } else if model.contains("haiku-4-5") {
        "Haiku 4.5"
    } else if model.contains("claude-3-7-sonnet") {
        "Sonnet 3.7"
    } else {
        model
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
    is_oauth: bool,
) -> Value {
    if minimal_transport_enabled() {
        return build_minimal_request_body(config, messages, system, tools, is_oauth);
    }

    // Strip [1m]/[2m] context window suffix before sending to the API.
    // Mirrors TS normalizeModelStringForAPI(): model.replace(/\[(1|2)m\]/gi, '')
    let api_model = normalize_model_for_api(&config.model);

    let mut body = json!({
        "model": api_model,
        "max_tokens": config.max_tokens,
        "stream": true,
        "messages": messages,
    });

    // Prompt caching: mark the last user-turn boundary in conversation
    // messages. Matches TS addCacheControlBreakpoints on messages — the
    // cache covers system + tools + conversation up to the marked turn.
    add_message_cache_markers(&mut body);

    // Thinking configuration.
    // Haiku does not support adaptive thinking — only send for Sonnet/Opus.
    let supports_thinking = !config.model.contains("haiku");
    let thinking_obj = if supports_thinking {
        match &config.thinking {
            ThinkingConfig::Disabled => None,
            ThinkingConfig::Enabled { budget_tokens } => {
                // API constraint: thinking.budget_tokens must be < max_tokens.
                // Mirrors TS: Math.min(maxOutputTokens - 1, thinkingBudget).
                let clamped = (*budget_tokens).min(config.max_tokens.saturating_sub(1));
                Some(json!({
                    "type": "enabled",
                    "budget_tokens": clamped,
                }))
            }
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
    if !system.is_empty() || is_oauth {
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

        // Prompt caching: mark the last system block with cache_control.
        // Matches TS addCacheControlBreakpoints — the system prompt is the
        // largest stable prefix and benefits most from caching.
        if let Some(sys_arr) = body["system"].as_array_mut() {
            if let Some(last) = sys_arr.last_mut() {
                last["cache_control"] = json!({"type": "ephemeral"});
            }
        }
    }

    // Tools (only include if non-empty).
    if !tools.is_empty() {
        body["tools"] = serde_json::to_value(tools).unwrap_or(Value::Null);

        // Prompt caching: mark the last tool definition.
        // Tool schemas are the second-largest stable prefix.
        if let Some(tools_arr) = body["tools"].as_array_mut() {
            if let Some(last) = tools_arr.last_mut() {
                last["cache_control"] = json!({"type": "ephemeral"});
            }
        }
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

/// Apply cache_control breakpoints to conversation messages in the request
/// body. Marks the last content block of the last user turn so the prompt
/// cache covers the stable prefix (system + tools + prior conversation).
///
/// Matches TS `addCacheControlBreakpoints` which walks messages backward and
/// stamps `cache_control: {"type": "ephemeral"}` on the last non-thinking
/// block of the most recent user turn.
fn add_message_cache_markers(body: &mut Value) {
    let Some(messages) = body["messages"].as_array_mut() else {
        return;
    };
    // Walk backward to find the last user message, mark its last content block.
    for msg in messages.iter_mut().rev() {
        if msg["role"].as_str() != Some("user") {
            continue;
        }
        let Some(content) = msg["content"].as_array_mut() else {
            continue;
        };
        // Find the last non-thinking block.
        for block in content.iter_mut().rev() {
            let btype = block["type"].as_str().unwrap_or("");
            if btype != "thinking" && btype != "redacted_thinking" {
                block["cache_control"] = json!({"type": "ephemeral"});
                return;
            }
        }
    }
}

fn build_minimal_request_body(
    config: &ApiConfig,
    messages: &[Value],
    system: &[ContentBlock],
    tools: &[ToolDefinition],
    is_oauth: bool,
) -> Value {
    let api_model = normalize_model_for_api(&config.model);

    let mut body = json!({
        "model": api_model,
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

        if let Some(sys_arr) = body["system"].as_array_mut() {
            if let Some(last) = sys_arr.last_mut() {
                last["cache_control"] = json!({"type": "ephemeral"});
            }
        }
    }

    if !tools.is_empty() {
        body["tools"] = serde_json::to_value(tools).unwrap_or(Value::Null);

        if let Some(tools_arr) = body["tools"].as_array_mut() {
            if let Some(last) = tools_arr.last_mut() {
                last["cache_control"] = json!({"type": "ephemeral"});
            }
        }
    }

    add_message_cache_markers(&mut body);

    body
}

/// Get or create a persistent device ID (matches TS `getOrCreateUserID()`).
fn get_or_create_device_id() -> String {
    if let Ok(config) = crate::config::global::load_global_config() {
        if let Some(user_id) = config.user_id {
            if !user_id.is_empty() {
                return user_id;
            }
        }
    }

    let id = generate_user_id();
    let saved_id = id.clone();
    let _ = crate::config::global::save_global_config(|mut config| {
        config.user_id = Some(saved_id);
        config
    });
    id
}

fn generate_user_id() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
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
            if has_1m_context(&self.config.model) {
                betas.push("context-1m-2025-08-07");
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

        // Return a typed error for prompt-too-long so the engine can
        // attempt reactive compaction before surfacing the error.
        if status == 413 || err_body.contains("prompt_too_long") {
            return Err(anyhow::Error::new(
                crate::types::error::PromptTooLongError { body: err_body },
            ));
        }

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
    use std::sync::Mutex;

    /// Serialize tests that touch environment variables to avoid races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn max_output_tokens_match_current_ts_defaults() {
        assert_eq!(get_max_output_tokens_for_model("claude-sonnet-4-6"), 64_000);
        assert_eq!(get_max_output_tokens_for_model("claude-opus-4-6"), 64_000);
        assert_eq!(get_max_output_tokens_for_model("claude-haiku-4-5"), 64_000);
        assert_eq!(get_max_output_tokens_for_model("claude-opus-4-1"), 32_000);
        assert_eq!(
            get_max_output_tokens_for_model("claude-3-opus-20240229"),
            4_096
        );
    }

    #[test]
    fn request_metadata_uses_stable_session_and_account_uuid() {
        let _guard = ENV_LOCK.lock().unwrap();
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
    fn generated_user_id_matches_ts_shape() {
        let id = generate_user_id();
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(id.chars().all(|c| !c.is_ascii_uppercase()));
    }

    #[test]
    fn minimal_transport_body_strips_metadata_and_context_management() {
        let _guard = ENV_LOCK.lock().unwrap();
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

    #[test]
    fn cache_markers_on_system_prompt() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDE_RS_MINIMAL_TRANSPORT");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let system = vec![
            ContentBlock::Text {
                text: "block 1".into(),
            },
            ContentBlock::Text {
                text: "block 2".into(),
            },
        ];
        let body = build_request_body(&config, &[], &system, &[], false);
        let sys = body["system"].as_array().unwrap();
        // Last system block should have cache_control
        let last = sys.last().unwrap();
        assert_eq!(
            last["cache_control"],
            json!({"type": "ephemeral"}),
            "last system block must have cache_control"
        );
        // First user block should NOT have cache_control
        assert!(
            sys[0].get("cache_control").is_none(),
            "first system block should not have cache_control"
        );
    }

    #[test]
    fn cache_markers_on_tool_definitions() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDE_RS_MINIMAL_TRANSPORT");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let tools = vec![
            ToolDefinition {
                name: "Read".into(),
                description: "Read a file".into(),
                input_schema: json!({"type": "object"}),
            },
            ToolDefinition {
                name: "Write".into(),
                description: "Write a file".into(),
                input_schema: json!({"type": "object"}),
            },
        ];
        let body = build_request_body(&config, &[], &[], &tools, false);
        let tools_arr = body["tools"].as_array().unwrap();
        let last = tools_arr.last().unwrap();
        assert_eq!(
            last["cache_control"],
            json!({"type": "ephemeral"}),
            "last tool must have cache_control"
        );
    }

    #[test]
    fn cache_markers_on_last_user_message() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDE_RS_MINIMAL_TRANSPORT");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let messages = vec![
            json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
            json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
            json!({"role": "user", "content": [{"type": "text", "text": "world"}]}),
        ];
        let body = build_request_body(&config, &messages, &[], &[], false);
        let msgs = body["messages"].as_array().unwrap();
        // Last user message (index 2) should have cache_control on its content block
        let last_user = &msgs[2];
        let content = last_user["content"].as_array().unwrap();
        assert_eq!(
            content[0]["cache_control"],
            json!({"type": "ephemeral"}),
            "last user message content block must have cache_control"
        );
        // First user message should NOT have it
        let first_user = &msgs[0];
        let first_content = first_user["content"].as_array().unwrap();
        assert!(
            first_content[0].get("cache_control").is_none(),
            "first user message should not have cache_control"
        );
    }

    #[test]
    fn cache_markers_skip_thinking_blocks() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDE_RS_MINIMAL_TRANSPORT");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let messages = vec![json!({"role": "user", "content": [
            {"type": "text", "text": "hello"},
            {"type": "thinking", "thinking": "hmm", "signature": "sig"}
        ]})];
        let body = build_request_body(&config, &messages, &[], &[], false);
        let msgs = body["messages"].as_array().unwrap();
        let content = msgs[0]["content"].as_array().unwrap();
        // cache_control should be on the text block, not the thinking block
        assert_eq!(content[0]["cache_control"], json!({"type": "ephemeral"}));
        assert!(content[1].get("cache_control").is_none());
    }

    #[test]
    fn minimal_transport_also_gets_cache_markers() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CLAUDE_RS_MINIMAL_TRANSPORT", "1");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let system = vec![ContentBlock::Text { text: "sys".into() }];
        let tools = vec![ToolDefinition {
            name: "T".into(),
            description: "d".into(),
            input_schema: json!({"type": "object"}),
        }];
        let messages = vec![json!({"role": "user", "content": [{"type": "text", "text": "hi"}]})];
        let body = build_request_body(&config, &messages, &system, &tools, false);
        let sys = body["system"].as_array().unwrap();
        assert_eq!(
            sys.last().unwrap()["cache_control"],
            json!({"type": "ephemeral"})
        );
        let tools_arr = body["tools"].as_array().unwrap();
        assert_eq!(
            tools_arr.last().unwrap()["cache_control"],
            json!({"type": "ephemeral"})
        );
        std::env::remove_var("CLAUDE_RS_MINIMAL_TRANSPORT");
    }

    #[test]
    fn oauth_request_includes_billing_attribution_without_user_system_prompt() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDE_RS_MINIMAL_TRANSPORT");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };

        let body = build_request_body(&config, &[], &[], &[], true);
        let system = body["system"].as_array().unwrap();

        assert!(
            system[0]["text"]
                .as_str()
                .unwrap()
                .starts_with("x-anthropic-billing-header:"),
            "OAuth billing attribution must be the first system block"
        );
        assert_eq!(
            system.last().unwrap()["cache_control"],
            json!({"type": "ephemeral"})
        );
    }
}
