use anyhow::Result;
use once_cell::sync::Lazy;
use rand::RngCore;
use reqwest::Response;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::api::retry::RetryPolicy;
use crate::auth::login::{debug_http_client, proxy_url};
use crate::auth::resolve::resolve_stored_oauth_token;
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

fn anthropic_beta_header_value(is_oauth: bool, model: &str) -> String {
    let mut betas = vec![
        crate::constants::betas::CLAUDE_CODE_20250219,
        crate::constants::oauth::OAUTH_BETA_HEADER,
        crate::constants::betas::CONTEXT_1M,
        crate::constants::betas::INTERLEAVED_THINKING,
        crate::constants::betas::CONTEXT_MANAGEMENT,
        crate::constants::betas::PROMPT_CACHING_SCOPE,
        crate::constants::betas::ADVISOR,
        crate::constants::betas::EFFORT,
    ];
    if !is_oauth {
        betas.retain(|beta| *beta != crate::constants::oauth::OAUTH_BETA_HEADER);
    }
    if !has_1m_context(model) {
        betas.retain(|beta| *beta != crate::constants::betas::CONTEXT_1M);
    }
    betas.join(",")
}

fn add_tool_search_beta_header(header: &str) -> String {
    let beta = crate::constants::betas::TOOL_SEARCH_1P;
    if header.split(',').any(|part| part.trim() == beta) {
        header.to_string()
    } else if header.is_empty() {
        beta.to_string()
    } else {
        format!("{header},{beta}")
    }
}

fn add_beta_header(header: &str, beta: &str) -> String {
    if header.split(',').any(|part| part.trim() == beta) {
        header.to_string()
    } else if header.is_empty() {
        beta.to_string()
    } else {
        format!("{header},{beta}")
    }
}

fn oauth_billing_header(workload: Option<&str>) -> String {
    let cch = rand::thread_rng().next_u32() & 0x000f_ffff;
    let workload_pair = workload
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!(" cc_workload={value};"))
        .unwrap_or_default();
    format!(
        "x-anthropic-billing-header: cc_version=2.1.121.b32; cc_entrypoint=sdk-cli; cch={cch:05x};{workload_pair}"
    )
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
    /// Optional fallback model used after repeated overload responses.
    pub fallback_model: Option<String>,
    /// Maximum number of output tokens.
    pub max_tokens: u64,
    /// Thinking / extended-reasoning configuration.
    pub thinking: ThinkingConfig,
    /// Optional speed hint.
    pub speed: Option<Speed>,
    /// Optional model effort level for `output_config.effort`.
    pub effort: Option<String>,
    /// Optional API-side task budget for `output_config.task_budget`.
    pub task_budget_total: Option<u64>,
    /// Optional workload attribution included in the OAuth billing header.
    pub workload: Option<String>,
    /// SDK-provided beta headers after TS allowlist filtering.
    pub sdk_betas: Vec<String>,
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
            fallback_model: None,
            max_tokens: get_max_output_tokens_for_model("claude-opus-4-6"),
            thinking: ThinkingConfig::Adaptive,
            speed: None,
            effort: None,
            task_budget_total: None,
            workload: None,
            sdk_betas: Vec::new(),
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
    if lower.contains("opus-4-7")
        || lower.contains("opus-4-6")
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

fn model_supports_effort(model: &str) -> bool {
    if std::env::var("CLAUDE_CODE_ALWAYS_ENABLE_EFFORT")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
    {
        return true;
    }
    let lower = normalize_model_for_api(model).to_ascii_lowercase();
    if lower.contains("opus-4-6") || lower.contains("sonnet-4-6") {
        return true;
    }
    if lower.contains("haiku") || lower.contains("sonnet") || lower.contains("opus") {
        return false;
    }
    true
}

fn model_supports_max_effort(model: &str) -> bool {
    normalize_model_for_api(model)
        .to_ascii_lowercase()
        .contains("opus-4-6")
}

fn resolve_output_effort(model: &str, effort: Option<&str>) -> Option<String> {
    if !model_supports_effort(model) {
        return None;
    }

    match effort.map(|value| value.trim().to_ascii_lowercase()) {
        Some(value) if matches!(value.as_str(), "low" | "medium" | "high") => Some(value),
        Some(value) if value == "max" && model_supports_max_effort(model) => Some(value),
        Some(value) if value == "max" => Some("high".into()),
        Some(value) if value == "auto" || value == "unset" || value.is_empty() => {
            Some("high".into())
        }
        Some(_) => Some("high".into()),
        None => Some("high".into()),
    }
}

// ── Tool definition (for the request body) ───────────────────────────────────

/// A tool definition sent to the API.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default, skip)]
    pub defer_loading: bool,
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
    let output_effort = resolve_output_effort(&api_model, config.effort.as_deref());
    if output_effort.is_some() || config.task_budget_total.is_some() {
        let mut output_config = serde_json::Map::new();
        if let Some(effort) = output_effort {
            output_config.insert("effort".to_string(), json!(effort));
        }
        if let Some(total) = config.task_budget_total {
            output_config.insert(
                "task_budget".to_string(),
                json!({
                    "type": "tokens",
                    "total": total,
                }),
            );
        }
        body["output_config"] = Value::Object(output_config);
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
    // Order: [attribution (OAuth only)] -> [user system blocks]
    if !system.is_empty() || is_oauth {
        let mut full_system: Vec<ContentBlock> = Vec::new();
        if is_oauth {
            full_system.push(ContentBlock::Text {
                text: oauth_billing_header(config.workload.as_deref()),
            });
        }
        full_system.extend_from_slice(system);
        body["system"] = serde_json::to_value(&full_system).unwrap_or(Value::Null);

        add_system_cache_markers(&mut body, is_oauth);
    }

    // Tools (only include if non-empty). TS decides ToolSearch/defer_loading
    // at request time, after model/env/discovered-tool state is known.
    let tools_for_request = prepare_tool_definitions_for_request(config, messages, tools);
    if !tools_for_request.is_empty() {
        body["tools"] = Value::Array(tools_for_request);
    }

    // metadata: mirrors TS getAPIMetadata() — user_id is a JSON-encoded string
    // containing device_id, account_uuid, and session_id.
    let device_id = get_or_create_device_id();
    let user_id = api_metadata_user_id(
        extra_metadata(),
        &device_id,
        &config.account_uuid,
        &config.session_id,
    );
    body["metadata"] = json!({
        "user_id": user_id,
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

/// Build a request body with raw tool schema values.
///
/// This is used for Anthropic server-side tools such as `web_search_20250305`,
/// whose schema shape is not the normal client tool `{name, description,
/// input_schema}` form.
pub fn build_request_body_with_raw_tools(
    config: &ApiConfig,
    messages: &[Value],
    system: &[ContentBlock],
    tools: &[Value],
    is_oauth: bool,
) -> Value {
    let mut body = build_request_body(config, messages, system, &[], is_oauth);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }
    body
}

/// Apply cache_control breakpoints to cacheable system prompt blocks.
///
/// TS leaves the OAuth billing attribution block unmarked and marks the
/// following system blocks, using a 1h ephemeral cache.
fn add_system_cache_markers(body: &mut Value, is_oauth: bool) {
    let Some(system) = body["system"].as_array_mut() else {
        return;
    };
    let start = usize::from(is_oauth);
    for block in system.iter_mut().skip(start) {
        block["cache_control"] = cache_control_json();
    }
}

/// Apply cache_control breakpoints to conversation messages in the request body.
/// Marks the last non-thinking content block of the last message so the prompt
/// cache covers the stable prefix (system + tools + conversation).
///
/// Matches TS `addCacheBreakpoints` marker placement: exactly one message-level
/// marker at `messages.length - 1`, skipping thinking/redacted_thinking blocks.
fn add_message_cache_markers(body: &mut Value) {
    let Some(messages) = body["messages"].as_array_mut() else {
        return;
    };
    let Some(msg) = messages.last_mut() else {
        return;
    };
    let Some(content) = msg["content"].as_array_mut() else {
        return;
    };
    for block in content.iter_mut().rev() {
        let btype = block["type"].as_str().unwrap_or("");
        if btype != "thinking" && btype != "redacted_thinking" {
            block["cache_control"] = cache_control_json();
            return;
        }
    }
}

fn cache_control_json() -> Value {
    let ttl = std::env::var("CLAUDE_RS_CACHE_TTL").unwrap_or_else(|_| "1h".to_string());
    let mut value = json!({"type": "ephemeral", "ttl": ttl});
    if value["ttl"].as_str() == Some("") {
        value.as_object_mut().unwrap().remove("ttl");
    }
    if let Ok(scope) = std::env::var("CLAUDE_RS_CACHE_SCOPE") {
        if !scope.is_empty() {
            value["scope"] = json!(scope);
        }
    }
    value
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolSearchRequestMode {
    ToolSearch,
    ToolSearchAuto,
    Standard,
}

fn tool_search_request_mode() -> ToolSearchRequestMode {
    if crate::errors_util::is_env_truthy("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS") {
        return ToolSearchRequestMode::Standard;
    }

    let value = std::env::var("ENABLE_TOOL_SEARCH").ok();
    if let Some(value) = value.as_deref() {
        if let Some(percent) = parse_auto_tool_search_percentage(value) {
            return match percent {
                0 => ToolSearchRequestMode::ToolSearch,
                100 => ToolSearchRequestMode::Standard,
                _ => ToolSearchRequestMode::ToolSearchAuto,
            };
        }
        if value == "auto" || value.starts_with("auto:") {
            return ToolSearchRequestMode::ToolSearchAuto;
        }
        if is_truthy_value(value) {
            return ToolSearchRequestMode::ToolSearch;
        }
    }

    if crate::errors_util::is_env_definitely_falsy("ENABLE_TOOL_SEARCH") {
        return ToolSearchRequestMode::Standard;
    }

    ToolSearchRequestMode::ToolSearch
}

fn is_truthy_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn parse_auto_tool_search_percentage(value: &str) -> Option<u8> {
    let percent = value.strip_prefix("auto:")?.parse::<i32>().ok()?;
    Some(percent.clamp(0, 100) as u8)
}

fn model_supports_tool_reference(model: &str) -> bool {
    !normalize_model_for_api(model)
        .to_ascii_lowercase()
        .contains("haiku")
}

fn is_deferred_tool(tool: &ToolDefinition) -> bool {
    tool.defer_loading || tool.name.starts_with("mcp__")
}

fn prepare_tool_definitions_for_request(
    config: &ApiConfig,
    messages: &[Value],
    tools: &[ToolDefinition],
) -> Vec<Value> {
    prepare_tool_definitions_for_request_with_count(config, messages, tools, None)
}

fn prepare_tool_definitions_for_request_with_count(
    config: &ApiConfig,
    messages: &[Value],
    tools: &[ToolDefinition],
    deferred_tool_tokens: Option<usize>,
) -> Vec<Value> {
    if tools.is_empty() {
        return Vec::new();
    }

    let deferred_names = tools
        .iter()
        .filter(|tool| is_deferred_tool(tool))
        .map(|tool| tool.name.as_str())
        .collect::<std::collections::HashSet<_>>();

    let use_tool_search = tool_search_allowed_for_request(config)
        && model_supports_tool_reference(&config.model)
        && tools.iter().any(|tool| tool.name == "ToolSearch")
        && !deferred_names.is_empty()
        && match tool_search_request_mode() {
            ToolSearchRequestMode::ToolSearch => true,
            ToolSearchRequestMode::ToolSearchAuto => {
                if let Some(tokens) = deferred_tool_tokens {
                    tokens >= auto_tool_search_token_threshold(&config.model)
                } else {
                    deferred_tool_description_chars(tools)
                        >= auto_tool_search_char_threshold(&config.model)
                }
            }
            ToolSearchRequestMode::Standard => false,
        };

    let discovered = if use_tool_search {
        extract_discovered_tool_names(messages)
    } else {
        std::collections::HashSet::new()
    };

    tools
        .iter()
        .filter_map(|tool| {
            if !use_tool_search && tool.name == "ToolSearch" {
                return None;
            }

            let is_deferred = deferred_names.contains(tool.name.as_str());
            if use_tool_search && is_deferred && !discovered.contains(tool.name.as_str()) {
                return None;
            }

            let mut value = serde_json::to_value(tool).unwrap_or(Value::Null);
            if use_tool_search && is_deferred {
                value["defer_loading"] = Value::Bool(true);
            }
            Some(value)
        })
        .collect()
}

fn tool_search_allowed_for_request(config: &ApiConfig) -> bool {
    if std::env::var("ENABLE_TOOL_SEARCH")
        .ok()
        .is_some_and(|value| !value.is_empty())
    {
        return true;
    }

    if crate::privacy_level::get_api_provider() == crate::privacy_level::ApiProvider::FirstParty {
        return is_first_party_anthropic_base_url_value(&config.base_url);
    }

    true
}

fn is_first_party_anthropic_base_url_value(raw: &str) -> bool {
    crate::privacy_level::is_first_party_anthropic_url(raw)
}

fn deferred_tool_description_chars(tools: &[ToolDefinition]) -> usize {
    tools
        .iter()
        .filter(|tool| is_deferred_tool(tool))
        .map(|tool| {
            tool.name.len()
                + tool.description.len()
                + serde_json::to_string(&tool.input_schema)
                    .map(|schema| schema.len())
                    .unwrap_or_default()
        })
        .sum()
}

fn auto_tool_search_token_threshold(model: &str) -> usize {
    let percentage = auto_tool_search_percentage() as f64 / 100.0;
    let context_window_tokens = if has_1m_context(model) {
        1_000_000
    } else {
        200_000
    };
    (context_window_tokens as f64 * percentage).floor() as usize
}

fn auto_tool_search_percentage() -> u8 {
    match std::env::var("ENABLE_TOOL_SEARCH").ok().as_deref() {
        Some("auto") | None => 10,
        Some(value) => parse_auto_tool_search_percentage(value).unwrap_or(10),
    }
}

fn auto_tool_search_char_threshold(model: &str) -> usize {
    (auto_tool_search_token_threshold(model) as f64 * 2.5).floor() as usize
}

fn extract_discovered_tool_names(messages: &[Value]) -> std::collections::HashSet<&str> {
    let mut names = std::collections::HashSet::new();
    for msg in messages {
        if msg.get("type").and_then(Value::as_str) == Some("compact_boundary")
            || msg.get("subtype").and_then(Value::as_str) == Some("compact_boundary")
        {
            for metadata in [
                msg.get("compactMetadata"),
                msg.get("message")
                    .and_then(|message| message.get("compactMetadata")),
            ]
            .into_iter()
            .flatten()
            {
                if let Some(carried) = metadata
                    .get("preCompactDiscoveredTools")
                    .and_then(Value::as_array)
                {
                    for name in carried.iter().filter_map(Value::as_str) {
                        names.insert(name);
                    }
                }
            }
        }

        let Some(content) = msg.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(items) = block.get("content").and_then(Value::as_array) else {
                continue;
            };
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("tool_reference") {
                    if let Some(name) = item.get("tool_name").and_then(Value::as_str) {
                        names.insert(name);
                    }
                }
            }
        }
    }
    names
}

fn body_uses_tool_search(body: &Value) -> bool {
    body.get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool.get("name").and_then(Value::as_str) == Some("ToolSearch"))
                || tools
                    .iter()
                    .any(|tool| tool.get("defer_loading").and_then(Value::as_bool) == Some(true))
        })
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
                text: oauth_billing_header(config.workload.as_deref()),
            });
        }
        full_system.extend_from_slice(system);
        body["system"] = serde_json::to_value(&full_system).unwrap_or(Value::Null);

        add_system_cache_markers(&mut body, is_oauth);
    }

    let tools_for_request = prepare_tool_definitions_for_request(config, messages, tools);
    if !tools_for_request.is_empty() {
        body["tools"] = Value::Array(tools_for_request);
    }

    add_message_cache_markers(&mut body);

    body
}

fn build_request_body_with_tool_search_count(
    config: &ApiConfig,
    messages: &[Value],
    system: &[ContentBlock],
    tools: &[ToolDefinition],
    is_oauth: bool,
    deferred_tool_tokens: Option<usize>,
) -> Value {
    let mut body = build_request_body(config, messages, system, &[], is_oauth);
    let tools_for_request = prepare_tool_definitions_for_request_with_count(
        config,
        messages,
        tools,
        deferred_tool_tokens,
    );
    if !tools_for_request.is_empty() {
        body["tools"] = Value::Array(tools_for_request);
    }
    body
}

const TOOL_TOKEN_COUNT_OVERHEAD: usize = 500;

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

fn extra_metadata() -> serde_json::Map<String, Value> {
    let Ok(raw) = std::env::var("CLAUDE_CODE_EXTRA_METADATA") else {
        return serde_json::Map::new();
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

fn api_metadata_user_id(
    mut extra: serde_json::Map<String, Value>,
    device_id: &str,
    account_uuid: &str,
    session_id: &str,
) -> String {
    // Match TS getAPIMetadata() insertion order for the stable Claude Code
    // fields. Extra metadata is preserved after those fields, except callers
    // cannot override the canonical ids.
    extra.remove("device_id");
    extra.remove("account_uuid");
    extra.remove("session_id");

    let mut parts = vec![
        json_string_pair("device_id", device_id),
        json_string_pair("account_uuid", account_uuid),
        json_string_pair("session_id", session_id),
    ];
    for (key, value) in extra {
        if let (Ok(key_json), Ok(value_json)) =
            (serde_json::to_string(&key), serde_json::to_string(&value))
        {
            parts.push(format!("{key_json}:{value_json}"));
        }
    }
    format!("{{{}}}", parts.join(","))
}

fn json_string_pair(key: &str, value: &str) -> String {
    let key_json = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
    let value_json = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string());
    format!("{key_json}:{value_json}")
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

    /// POST a streaming request with raw tool schema values.
    pub async fn stream_request_with_raw_tools(
        &self,
        messages: &[Value],
        system: &[ContentBlock],
        tools: &[Value],
    ) -> Result<Response> {
        let body = build_request_body_with_raw_tools(
            &self.config,
            messages,
            system,
            tools,
            self.auth.is_oauth(),
        );
        self.send_streaming_body(body).await
    }

    /// POST a streaming request with optional event feedback.
    pub async fn stream_request_with_events(
        &self,
        messages: &[Value],
        system: &[ContentBlock],
        tools: &[ToolDefinition],
        _event_tx: Option<&mpsc::Sender<StreamEvent>>,
    ) -> Result<Response> {
        let deferred_tool_tokens = self
            .count_deferred_tool_tokens_for_auto_tool_search(tools)
            .await;
        let body = build_request_body_with_tool_search_count(
            &self.config,
            messages,
            system,
            tools,
            self.auth.is_oauth(),
            deferred_tool_tokens,
        );
        self.send_streaming_body(body).await
    }

    async fn count_deferred_tool_tokens_for_auto_tool_search(
        &self,
        tools: &[ToolDefinition],
    ) -> Option<usize> {
        if tool_search_request_mode() != ToolSearchRequestMode::ToolSearchAuto {
            return None;
        }
        if !tool_search_allowed_for_request(&self.config)
            || !model_supports_tool_reference(&self.config.model)
            || !tools.iter().any(|tool| tool.name == "ToolSearch")
        {
            return None;
        }

        let deferred_tools = tools
            .iter()
            .filter(|tool| is_deferred_tool(tool))
            .cloned()
            .collect::<Vec<_>>();
        if deferred_tools.is_empty() {
            return Some(0);
        }

        match self.count_tool_definition_tokens(&deferred_tools).await {
            Ok(Some(tokens)) if tokens > 0 => {
                Some(tokens.saturating_sub(TOOL_TOKEN_COUNT_OVERHEAD))
            }
            _ => None,
        }
    }

    async fn count_tool_definition_tokens(
        &self,
        tools: &[ToolDefinition],
    ) -> Result<Option<usize>> {
        let auth = self.current_request_auth().await?;
        let (header_name, header_value) = auth.to_header();
        let url = format!("{}/v1/messages/count_tokens", self.config.base_url);
        let body = json!({
            "model": normalize_model_for_api(&self.config.model),
            "messages": [{"role": "user", "content": "foo"}],
            "tools": tools,
        });

        let mut request = self
            .http
            .post(url)
            .header("anthropic-version", &self.config.api_version)
            .header("content-type", "application/json")
            .header(header_name, header_value);

        if !minimal_transport_enabled() {
            let beta_header = anthropic_beta_header_value(auth.is_oauth(), &self.config.model);
            request = request
                .header("accept", "application/json")
                .header("user-agent", crate::user_agent::get_user_agent(None))
                .header("x-claude-code-session-id", &self.config.session_id)
                .header("x-stainless-lang", "js")
                .header("x-stainless-package-version", "2.2.0")
                .header("x-stainless-runtime", "node")
                .header("x-stainless-retry-count", "0")
                .header("anthropic-beta", beta_header)
                .header("anthropic-dangerous-direct-browser-access", "true")
                .header("x-app", "cli");
        } else if auth.is_oauth() {
            request = request.header("anthropic-beta", "oauth-2025-04-20");
        }

        let response = request.json(&body).send().await?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let value = response.json::<Value>().await?;
        Ok(value
            .get("input_tokens")
            .and_then(Value::as_u64)
            .map(|tokens| tokens as usize))
    }

    async fn send_streaming_body(&self, mut body: Value) -> Result<Response> {
        let url = format!("{}/v1/messages", self.config.base_url);
        let minimal_transport = minimal_transport_enabled();
        let retry_policy = RetryPolicy::default();
        let mut retry_attempt = 0_u32;
        let mut consecutive_529 = 0_u32;
        let mut switched_to_fallback = false;

        loop {
            // Debug mode: dump the full request body when CLAUDE_RS_DEBUG=1
            if std::env::var("CLAUDE_RS_DEBUG").as_deref() == Ok("1") {
                if let Ok(pretty) = serde_json::to_string_pretty(&body) {
                    let _ = std::fs::write("/tmp/claude-rs-request.json", &pretty);
                    tracing::debug!("Request body written to /tmp/claude-rs-request.json");
                }
            }

            let auth = self.current_request_auth().await?;
            let (header_name, header_value) = auth.to_header();

            let mut request = self
                .http
                .post(&url)
                .header("anthropic-version", &self.config.api_version)
                .header("content-type", "application/json")
                .header(header_name, header_value);

            if minimal_transport {
                if auth.is_oauth() {
                    request = request.header("anthropic-beta", "oauth-2025-04-20");
                }
            } else {
                request = request
                    .header("accept", "application/json")
                    .header("user-agent", crate::user_agent::get_user_agent(None))
                    .header("x-claude-code-session-id", &self.config.session_id)
                    .header("x-stainless-lang", "js")
                    .header("x-stainless-package-version", "2.2.0")
                    .header("x-stainless-runtime", "node")
                    .header("x-stainless-retry-count", retry_attempt.to_string());

                let mut beta_header =
                    anthropic_beta_header_value(auth.is_oauth(), &self.config.model);
                if !auth.is_oauth() {
                    for beta in &self.config.sdk_betas {
                        beta_header = add_beta_header(&beta_header, beta);
                    }
                }
                if body_uses_tool_search(&body) {
                    beta_header = add_tool_search_beta_header(&beta_header);
                }
                if body
                    .get("output_config")
                    .and_then(|value| value.get("task_budget"))
                    .is_some()
                    && !beta_header
                        .split(',')
                        .any(|part| part.trim() == crate::constants::betas::TASK_BUDGETS)
                {
                    beta_header =
                        add_beta_header(&beta_header, crate::constants::betas::TASK_BUDGETS);
                }

                request = request
                    .header("anthropic-beta", beta_header)
                    .header("anthropic-dangerous-direct-browser-access", "true")
                    .header("x-app", "cli");
            }

            let response = request.json(&body).send().await?;

            if response.status().is_success() {
                crate::rate_limits::extract_quota_status_from_headers(
                    response.headers(),
                    auth.is_oauth(),
                );
                return Ok(response);
            }

            let status = response.status().as_u16();
            crate::rate_limits::extract_quota_status_from_error(
                Some(response.headers()),
                status,
                auth.is_oauth(),
            );
            let err_body = response.text().await.unwrap_or_default();

            // Return a typed error for prompt-too-long so the engine can
            // attempt reactive compaction before surfacing the error.
            if status == 413 || err_body.contains("prompt_too_long") {
                return Err(anyhow::Error::new(
                    crate::types::error::PromptTooLongError { body: err_body },
                ));
            }

            if status == 529 {
                consecutive_529 += 1;
                if consecutive_529 >= retry_policy.max_529_retries && !switched_to_fallback {
                    if let Some(fallback_model) = &self.config.fallback_model {
                        body["model"] = Value::String(normalize_model_for_api(fallback_model));
                        switched_to_fallback = true;
                        consecutive_529 = 0;
                        retry_attempt += 1;
                        continue;
                    }
                }
            } else {
                consecutive_529 = 0;
            }

            if matches!(status, 429 | 500 | 502 | 503 | 504 | 529)
                && retry_attempt < retry_policy.max_retries
            {
                retry_attempt += 1;
                tokio::time::sleep(retry_policy.backoff_delay(retry_attempt)).await;
                continue;
            }

            anyhow::bail!(
                "API error {}: {}",
                status,
                &err_body[..err_body.len().min(500)]
            );
        }
    }

    async fn current_request_auth(&self) -> Result<AuthMethod> {
        match &self.auth {
            AuthMethod::ApiKey(_) => Ok(self.auth.clone()),
            AuthMethod::OAuthToken(_) => {
                let token = resolve_stored_oauth_token(false)
                    .await?
                    .unwrap_or_else(|| match &self.auth {
                        AuthMethod::OAuthToken(token) => token.clone(),
                        AuthMethod::ApiKey(_) => unreachable!(),
                    });
                Ok(AuthMethod::OAuthToken(token))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize tests that touch environment variables to avoid races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_cache_env() {
        std::env::remove_var("CLAUDE_RS_MINIMAL_TRANSPORT");
        std::env::remove_var("CLAUDE_RS_CACHE_TTL");
        std::env::remove_var("CLAUDE_RS_CACHE_SCOPE");
    }

    fn clear_tool_search_env() {
        std::env::remove_var("ENABLE_TOOL_SEARCH");
        std::env::remove_var("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS");
        std::env::remove_var("CLAUDE_CODE_USE_BEDROCK");
        std::env::remove_var("CLAUDE_CODE_USE_VERTEX");
        std::env::remove_var("CLAUDE_CODE_USE_FOUNDRY");
        std::env::remove_var("USER_TYPE");
    }

    fn tool_def(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: format!("{name} description"),
            input_schema: json!({"type": "object", "properties": {}}),
            defer_loading: name.starts_with("mcp__"),
        }
    }

    fn deferred_tool_def(name: &str) -> ToolDefinition {
        ToolDefinition {
            defer_loading: true,
            ..tool_def(name)
        }
    }

    #[test]
    fn max_output_tokens_match_current_ts_defaults() {
        assert_eq!(get_max_output_tokens_for_model("claude-sonnet-4-6"), 64_000);
        assert_eq!(get_max_output_tokens_for_model("claude-opus-4-7"), 64_000);
        assert_eq!(get_max_output_tokens_for_model("claude-opus-4-6"), 64_000);
        assert_eq!(get_max_output_tokens_for_model("claude-haiku-4-5"), 64_000);
        assert_eq!(get_max_output_tokens_for_model("claude-opus-4-1"), 32_000);
        assert_eq!(
            get_max_output_tokens_for_model("claude-3-opus-20240229"),
            4_096
        );
    }

    #[test]
    fn anthropic_beta_header_matches_ts_oauth_1m_order() {
        assert_eq!(
            anthropic_beta_header_value(true, "claude-opus-4-7[1m]"),
            [
                crate::constants::betas::CLAUDE_CODE_20250219,
                crate::constants::oauth::OAUTH_BETA_HEADER,
                crate::constants::betas::CONTEXT_1M,
                crate::constants::betas::INTERLEAVED_THINKING,
                crate::constants::betas::CONTEXT_MANAGEMENT,
                crate::constants::betas::PROMPT_CACHING_SCOPE,
                crate::constants::betas::ADVISOR,
                crate::constants::betas::EFFORT,
            ]
            .join(",")
        );
    }

    #[test]
    fn anthropic_beta_header_omits_oauth_and_1m_when_absent() {
        let header = anthropic_beta_header_value(false, "claude-sonnet-4-6");
        assert!(!header.contains(crate::constants::oauth::OAUTH_BETA_HEADER));
        assert!(!header.contains(crate::constants::betas::CONTEXT_1M));
        assert!(header.contains(crate::constants::betas::ADVISOR));
        assert!(header.contains(crate::constants::betas::EFFORT));
    }

    #[test]
    fn request_metadata_uses_stable_session_and_account_uuid() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDE_CODE_EXTRA_METADATA");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            session_id: "session-123".into(),
            account_uuid: "account-456".into(),
            ..Default::default()
        };
        let body = build_request_body(&config, &[], &[], &[], false);
        let user_id = body["metadata"]["user_id"].as_str().unwrap();
        assert!(user_id.starts_with(r#"{"device_id":"#,));
        assert!(user_id.contains(r#","account_uuid":"account-456","session_id":"session-123""#));
        let parsed: Value = serde_json::from_str(user_id).unwrap();
        assert_eq!(parsed["session_id"], "session-123");
        assert_eq!(parsed["account_uuid"], "account-456");
    }

    #[test]
    fn request_metadata_merges_extra_metadata() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var(
            "CLAUDE_CODE_EXTRA_METADATA",
            r#"{"source":"test","device_id":"ignored"}"#,
        );
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            session_id: "session-123".into(),
            account_uuid: "account-456".into(),
            ..Default::default()
        };
        let body = build_request_body(&config, &[], &[], &[], false);
        let user_id = body["metadata"]["user_id"].as_str().unwrap();
        assert!(user_id.starts_with(r#"{"device_id":"#,));
        assert!(user_id.contains(r#","account_uuid":"account-456","session_id":"session-123""#));
        let parsed: Value = serde_json::from_str(user_id).unwrap();
        assert_eq!(parsed["source"], "test");
        assert_eq!(parsed["session_id"], "session-123");
        assert_eq!(parsed["account_uuid"], "account-456");
        assert_ne!(parsed["device_id"], "ignored");
        std::env::remove_var("CLAUDE_CODE_EXTRA_METADATA");
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
        clear_cache_env();
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
        clear_cache_env();
    }

    #[test]
    fn opus_output_config_uses_supported_ts_effort_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        let config = ApiConfig {
            model: "claude-opus-4-6".into(),
            max_tokens: 64_000,
            thinking: ThinkingConfig::Adaptive,
            ..Default::default()
        };
        let body = build_request_body(&config, &[], &[], &[], false);
        assert_eq!(body["output_config"], json!({"effort": "high"}));
    }

    #[test]
    fn effort_is_limited_to_ts_supported_levels_and_models() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();

        let sonnet_max = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 64_000,
            effort: Some("max".into()),
            ..Default::default()
        };
        let body = build_request_body(&sonnet_max, &[], &[], &[], false);
        assert_eq!(body["output_config"], json!({"effort": "high"}));

        let opus_max = ApiConfig {
            model: "claude-opus-4-6".into(),
            max_tokens: 64_000,
            effort: Some("max".into()),
            ..Default::default()
        };
        let body = build_request_body(&opus_max, &[], &[], &[], false);
        assert_eq!(body["output_config"], json!({"effort": "max"}));

        let stale_xhigh = ApiConfig {
            model: "claude-opus-4-6".into(),
            max_tokens: 64_000,
            effort: Some("xhigh".into()),
            ..Default::default()
        };
        let body = build_request_body(&stale_xhigh, &[], &[], &[], false);
        assert_eq!(body["output_config"], json!({"effort": "high"}));

        let stale_opus = ApiConfig {
            model: "claude-opus-4-7".into(),
            max_tokens: 64_000,
            effort: Some("high".into()),
            ..Default::default()
        };
        let body = build_request_body(&stale_opus, &[], &[], &[], false);
        assert!(body.get("output_config").is_none());

        let unsupported_model = ApiConfig {
            model: "claude-haiku-4-5".into(),
            max_tokens: 64_000,
            effort: Some("high".into()),
            ..Default::default()
        };
        let body = build_request_body(&unsupported_model, &[], &[], &[], false);
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn task_budget_merges_into_output_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        let config = ApiConfig {
            model: "claude-opus-4-6".into(),
            max_tokens: 64_000,
            task_budget_total: Some(12_345),
            ..Default::default()
        };
        let body = build_request_body(&config, &[], &[], &[], false);
        assert_eq!(
            body["output_config"],
            json!({
                "effort": "high",
                "task_budget": {
                    "type": "tokens",
                    "total": 12_345,
                }
            })
        );
    }

    #[test]
    fn workload_is_included_in_oauth_billing_header() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            workload: Some("cron".into()),
            ..Default::default()
        };
        let body = build_request_body(&config, &[], &[], &[], true);
        let first_system = body["system"].as_array().unwrap()[0]["text"]
            .as_str()
            .unwrap();
        assert!(first_system.contains(" cc_workload=cron;"));
    }

    #[test]
    fn cache_markers_on_system_prompt() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
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
        assert_eq!(
            sys[0]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"}),
            "first cacheable system block must have cache_control"
        );
        assert_eq!(
            sys.last().unwrap()["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"}),
            "last system block must have cache_control"
        );
    }

    #[test]
    fn cache_markers_not_added_to_tool_definitions() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
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
                defer_loading: false,
            },
            ToolDefinition {
                name: "Write".into(),
                description: "Write a file".into(),
                input_schema: json!({"type": "object"}),
                defer_loading: false,
            },
        ];
        let body = build_request_body(&config, &[], &[], &tools, false);
        let tools_arr = body["tools"].as_array().unwrap();
        let last = tools_arr.last().unwrap();
        assert!(
            last.get("cache_control").is_none(),
            "tool definitions should not have cache_control"
        );
    }

    #[test]
    fn tool_search_stripped_when_no_deferred_tools_match_ts() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        clear_tool_search_env();
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let tools = vec![tool_def("Read"), tool_def("ToolSearch")];

        let body = build_request_body(&config, &[], &[], &tools, false);
        let names = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["Read"]);
        assert!(!body_uses_tool_search(&body));
    }

    #[test]
    fn tool_search_default_disabled_for_non_first_party_base_url_match_ts() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        clear_tool_search_env();
        let config = ApiConfig {
            base_url: "http://127.0.0.1:8787".into(),
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let tools = vec![
            tool_def("Read"),
            tool_def("ToolSearch"),
            tool_def("mcp__jira__search"),
        ];

        let body = build_request_body(&config, &[], &[], &tools, false);
        let names = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["Read", "mcp__jira__search"]);
        assert!(!body_uses_tool_search(&body));
    }

    #[test]
    fn tool_search_keeps_only_discovered_deferred_tools_match_ts() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        clear_tool_search_env();
        std::env::set_var("ENABLE_TOOL_SEARCH", "true");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let tools = vec![
            tool_def("Read"),
            tool_def("ToolSearch"),
            tool_def("mcp__jira__search"),
            tool_def("mcp__slack__send"),
        ];
        let messages = vec![json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_1",
                "content": [{"type": "tool_reference", "tool_name": "mcp__jira__search"}]
            }]
        })];

        let body = build_request_body(&config, &messages, &[], &tools, false);
        let tools_arr = body["tools"].as_array().unwrap();
        let names = tools_arr
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["Read", "ToolSearch", "mcp__jira__search"]);
        let jira = tools_arr
            .iter()
            .find(|tool| tool["name"] == "mcp__jira__search")
            .unwrap();
        assert_eq!(jira["defer_loading"], true);
        assert!(body_uses_tool_search(&body));
        clear_tool_search_env();
    }

    #[test]
    fn tool_search_defers_builtin_should_defer_tools_match_ts() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        clear_tool_search_env();
        std::env::set_var("ENABLE_TOOL_SEARCH", "true");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let tools = vec![
            tool_def("Read"),
            tool_def("ToolSearch"),
            deferred_tool_def("TodoWrite"),
            deferred_tool_def("WebFetch"),
        ];

        let body = build_request_body(&config, &[], &[], &tools, false);
        let names = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["Read", "ToolSearch"]);

        let messages = vec![json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_1",
                "content": [{"type": "tool_reference", "tool_name": "TodoWrite"}]
            }]
        })];
        let body = build_request_body(&config, &messages, &[], &tools, false);
        let tools_arr = body["tools"].as_array().unwrap();
        let names = tools_arr
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Read", "ToolSearch", "TodoWrite"]);
        let todo = tools_arr
            .iter()
            .find(|tool| tool["name"] == "TodoWrite")
            .unwrap();
        assert_eq!(todo["defer_loading"], true);
        clear_tool_search_env();
    }

    #[test]
    fn tool_search_carries_discovered_tools_from_compact_boundary_match_ts() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        clear_tool_search_env();
        std::env::set_var("ENABLE_TOOL_SEARCH", "true");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let tools = vec![
            tool_def("Read"),
            tool_def("ToolSearch"),
            deferred_tool_def("TodoWrite"),
            tool_def("mcp__jira__search"),
        ];
        let messages = vec![json!({
            "type": "system",
            "subtype": "compact_boundary",
            "compactMetadata": {
                "preCompactDiscoveredTools": ["TodoWrite", "mcp__jira__search"]
            },
            "message": {"summary": "old conversation compacted"}
        })];

        let body = build_request_body(&config, &messages, &[], &tools, false);
        let names = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec!["Read", "ToolSearch", "TodoWrite", "mcp__jira__search"]
        );
        clear_tool_search_env();
    }

    #[test]
    fn tool_search_disabled_for_haiku_match_ts_model_gate() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        clear_tool_search_env();
        std::env::set_var("ENABLE_TOOL_SEARCH", "true");
        let config = ApiConfig {
            model: "claude-haiku-4-5".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let tools = vec![
            tool_def("Read"),
            tool_def("ToolSearch"),
            tool_def("mcp__jira__search"),
        ];

        let body = build_request_body(&config, &[], &[], &tools, false);
        let names = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["Read", "mcp__jira__search"]);
        assert!(!body_uses_tool_search(&body));
        clear_tool_search_env();
    }

    #[test]
    fn tool_search_auto_threshold_matches_ts_char_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        clear_tool_search_env();
        std::env::set_var("ENABLE_TOOL_SEARCH", "auto:100");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let tools = vec![
            tool_def("Read"),
            tool_def("ToolSearch"),
            tool_def("mcp__big__tool"),
        ];
        let body = build_request_body(&config, &[], &[], &tools, false);
        assert!(!body_uses_tool_search(&body));

        std::env::set_var("ENABLE_TOOL_SEARCH", "auto:0");
        let body = build_request_body(&config, &[], &[], &tools, false);
        let names = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Read", "ToolSearch"]);
        assert!(body_uses_tool_search(&body));
        clear_tool_search_env();
    }

    #[test]
    fn tool_search_auto_prefers_token_count_over_char_fallback_match_ts() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        clear_tool_search_env();
        std::env::set_var("ENABLE_TOOL_SEARCH", "auto:10");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let tools = vec![
            tool_def("Read"),
            tool_def("ToolSearch"),
            tool_def("mcp__small__tool"),
        ];

        let char_fallback_body = build_request_body(&config, &[], &[], &tools, false);
        assert!(
            !body_uses_tool_search(&char_fallback_body),
            "short descriptions should remain below the char fallback threshold"
        );

        let counted_body = build_request_body_with_tool_search_count(
            &config,
            &[],
            &[],
            &tools,
            false,
            Some(auto_tool_search_token_threshold(&config.model)),
        );
        assert!(
            body_uses_tool_search(&counted_body),
            "API token count should drive tst-auto when available"
        );
        clear_tool_search_env();
    }

    #[test]
    fn tool_search_auto_token_count_subtracts_ts_tool_overhead() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        clear_tool_search_env();
        std::env::set_var("ENABLE_TOOL_SEARCH", "auto:10");
        assert_eq!(TOOL_TOKEN_COUNT_OVERHEAD, 500);
        assert_eq!(
            auto_tool_search_token_threshold("claude-sonnet-4-6"),
            20_000
        );
        assert_eq!(auto_tool_search_char_threshold("claude-sonnet-4-6"), 50_000);
        clear_tool_search_env();
    }

    #[test]
    fn raw_tool_request_preserves_server_tool_schema() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let raw_tools = vec![json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 8
        })];
        let body = build_request_body_with_raw_tools(&config, &[], &[], &raw_tools, false);
        assert_eq!(body["tools"][0]["type"], "web_search_20250305");
        assert_eq!(body["tools"][0]["name"], "web_search");
        assert_eq!(body["tools"][0]["max_uses"], 8);
        assert!(
            body["tools"][0].get("cache_control").is_none(),
            "raw tool definitions should not have cache_control"
        );
        assert!(body["tools"][0].get("input_schema").is_none());
    }

    #[test]
    fn cache_markers_on_last_message() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
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
        // Last message (index 2) should have cache_control on its content block
        let last_user = &msgs[2];
        let content = last_user["content"].as_array().unwrap();
        assert_eq!(
            content[0]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"}),
            "last message content block must have cache_control"
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
    fn cache_markers_use_last_message_even_when_assistant() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let messages = vec![
            json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
            json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
        ];
        let body = build_request_body(&config, &messages, &[], &[], false);
        let msgs = body["messages"].as_array().unwrap();
        assert!(msgs[0]["content"][0].get("cache_control").is_none());
        assert_eq!(
            msgs[1]["content"][0]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"})
        );
    }

    #[test]
    fn cache_control_env_can_set_ttl_and_scope() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
        std::env::set_var("CLAUDE_RS_CACHE_TTL", "1h");
        std::env::set_var("CLAUDE_RS_CACHE_SCOPE", "global");
        let config = ApiConfig {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8_192,
            ..Default::default()
        };
        let messages = vec![json!({"role": "user", "content": [{"type": "text", "text": "hi"}]})];
        let body = build_request_body(&config, &messages, &[], &[], false);
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h", "scope": "global"})
        );
        clear_cache_env();
    }

    #[test]
    fn cache_markers_skip_thinking_blocks() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
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
        assert_eq!(
            content[0]["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"})
        );
        assert!(content[1].get("cache_control").is_none());
    }

    #[test]
    fn minimal_transport_also_gets_cache_markers() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
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
            defer_loading: false,
        }];
        let messages = vec![json!({"role": "user", "content": [{"type": "text", "text": "hi"}]})];
        let body = build_request_body(&config, &messages, &system, &tools, false);
        let sys = body["system"].as_array().unwrap();
        assert_eq!(
            sys.last().unwrap()["cache_control"],
            json!({"type": "ephemeral", "ttl": "1h"})
        );
        let tools_arr = body["tools"].as_array().unwrap();
        assert!(
            tools_arr.last().unwrap().get("cache_control").is_none(),
            "tool definitions should not have cache_control"
        );
        clear_cache_env();
    }

    #[test]
    fn oauth_request_includes_billing_attribution_without_user_system_prompt() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_cache_env();
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
        assert!(
            system.last().unwrap().get("cache_control").is_none(),
            "billing-only system prompt should not have cache_control"
        );
    }
}
