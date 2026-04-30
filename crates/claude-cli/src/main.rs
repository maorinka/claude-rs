use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use claude_core::session::storage::InternalTranscriptEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

static REMOTE_STREAM_JSON_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<Value>> = OnceLock::new();
static REMOTE_CCR_DELIVERY_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<CcrDeliveryUpdate>> =
    OnceLock::new();
static REMOTE_CCR_WORKER_PATCH_TX: OnceLock<tokio::sync::mpsc::UnboundedSender<Value>> =
    OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
struct CcrDeliveryUpdate {
    event_id: String,
    status: &'static str,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct CcrInternalEvent {
    payload: Value,
    #[serde(default)]
    agent_id: Option<String>,
}

#[derive(Default)]
struct RemoteStreamAccumulator {
    by_message: HashMap<String, Vec<Vec<String>>>,
    scope_to_message: HashMap<String, String>,
}

#[derive(Parser)]
#[command(
    name = "claude-rs",
    about = "Claude Code - AI coding assistant (Rust port)",
    version
)]
pub struct Cli {
    /// Initial prompt (non-interactive mode)
    pub prompt: Option<String>,

    /// Print response and exit (TS-compatible alias for non-interactive mode)
    #[arg(short = 'p', long = "print")]
    pub print: bool,

    /// Minimal mode: skip unrequested startup systems and set CLAUDE_CODE_SIMPLE=1
    #[arg(long = "bare")]
    pub bare: bool,

    /// Output format for non-interactive mode
    #[arg(long = "output-format", value_enum, default_value_t = OutputFormat::Text)]
    pub output_format: OutputFormat,

    /// Input format for non-interactive mode
    #[arg(long = "input-format", value_enum, default_value_t = InputFormat::Text)]
    pub input_format: InputFormat,

    /// JSON Schema for structured output validation
    #[arg(long = "json-schema")]
    pub json_schema: Option<String>,

    /// Include hook lifecycle events in stream-json output
    #[arg(long = "include-hook-events")]
    pub include_hook_events: bool,

    /// Include partial stream message chunks in stream-json output
    #[arg(long = "include-partial-messages")]
    pub include_partial_messages: bool,

    /// Re-emit SDK user stdin messages on stdout
    #[arg(long = "replay-user-messages")]
    pub replay_user_messages: bool,

    /// Model to use
    #[arg(short, long)]
    pub model: Option<String>,

    /// Effort level for the current session
    #[arg(long = "effort", value_parser = ["low", "medium", "high", "max", "auto", "unset"])]
    pub effort: Option<String>,

    /// Beta headers to include in API requests (API key users only)
    #[arg(long = "betas", num_args = 1.., value_delimiter = ',')]
    pub betas: Vec<String>,

    /// Enable automatic fallback to the specified model when the main model is overloaded
    #[arg(long = "fallback-model")]
    pub fallback_model: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Skip all permission checks (dangerous)
    #[arg(long)]
    pub dangerously_skip_permissions: bool,

    /// Permission mode to use for the session
    #[arg(long = "permission-mode")]
    pub permission_mode: Option<String>,

    /// Comma or space-separated list of tool permission rules to allow
    #[arg(
        long = "allowedTools",
        alias = "allowed-tools",
        num_args = 1..,
        value_delimiter = ','
    )]
    pub allowed_tools: Vec<String>,

    /// Comma or space-separated list of tool permission rules to deny
    #[arg(
        long = "disallowedTools",
        alias = "disallowed-tools",
        num_args = 1..,
        value_delimiter = ','
    )]
    pub disallowed_tools: Vec<String>,

    /// Specify available tools ("default", "", or comma/space-separated tool names)
    #[arg(long = "tools", num_args = 1.., value_delimiter = ',')]
    pub tools: Vec<String>,

    /// Additional directories to allow tool access to
    #[arg(long = "add-dir", num_args = 1..)]
    pub add_dirs: Vec<String>,

    /// Load MCP servers from JSON files or JSON strings
    #[arg(long = "mcp-config", num_args = 1..)]
    pub mcp_config: Vec<String>,

    /// Only use MCP servers supplied by --mcp-config
    #[arg(long = "strict-mcp-config")]
    pub strict_mcp_config: bool,

    /// Path to a settings JSON file or an inline JSON settings object
    #[arg(long = "settings")]
    pub settings: Option<String>,

    /// Comma-separated setting sources to load: user, project, local
    #[arg(long = "setting-sources")]
    pub setting_sources: Option<String>,

    /// JSON object defining additional agents
    #[arg(long = "agents", hide = true)]
    pub agents: Option<String>,

    /// Agent for the current session. Overrides the 'agent' setting.
    #[arg(long = "agent")]
    pub agent: Option<String>,

    /// MCP tool to use for permission prompts in print mode
    #[arg(long = "permission-prompt-tool", hide = true)]
    pub permission_prompt_tool: Option<String>,

    /// Working directory
    #[arg(short = 'C', long = "cd")]
    pub working_dir: Option<PathBuf>,

    /// Resume session by ID
    #[arg(long)]
    pub resume: Option<String>,

    /// Use a specific session ID
    #[arg(long = "session-id")]
    pub session_id: Option<String>,

    /// Session SDK URL for Remote Control / Session-Ingress child sessions
    #[arg(long = "sdk-url", hide = true)]
    pub sdk_url: Option<String>,

    /// Write debug logs to file
    #[arg(long = "debug-file", hide = true)]
    pub debug_file: Option<PathBuf>,

    /// Disable session persistence (accepted for TS CLI parity; non-interactive Rust mode does not persist sessions)
    #[arg(long = "no-session-persistence")]
    pub no_session_persistence: bool,

    /// Max conversation turns (non-interactive)
    #[arg(long)]
    pub max_turns: Option<u32>,

    /// Maximum dollar amount to spend on API calls (non-interactive)
    #[arg(long = "max-budget-usd")]
    pub max_budget_usd: Option<f64>,

    /// API-side task budget in tokens
    #[arg(long = "task-budget")]
    pub task_budget: Option<u64>,

    /// Workload tag for billing-header attribution
    #[arg(long = "workload", hide = true)]
    pub workload: Option<String>,

    /// System prompt to use for the session
    #[arg(long = "system-prompt")]
    pub system_prompt: Option<String>,

    /// Read system prompt from a file
    #[arg(long = "system-prompt-file", hide = true)]
    pub system_prompt_file: Option<PathBuf>,

    /// Append text to system prompt
    #[arg(long = "append-system-prompt")]
    pub append_system_prompt: Option<String>,

    /// Read text from a file and append to the default system prompt
    #[arg(long = "append-system-prompt-file", hide = true)]
    pub append_system_prompt_file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<SubCommand>,
}

#[derive(Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
    StreamJson,
}

#[derive(Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum InputFormat {
    Text,
    StreamJson,
}

#[derive(Default)]
struct StreamJsonStdinSeed {
    prompt: Option<String>,
    prompt_content: Option<Value>,
    prompt_delivery_event_id: Option<String>,
    system_prompt: Option<String>,
    append_system_prompt: Option<String>,
    json_schema: Option<String>,
    control_requests: Vec<Value>,
    restored_worker_metadata: Option<Value>,
}

#[derive(Clone, Debug)]
struct RemotePrompt {
    content: Value,
    delivery_event_id: Option<String>,
}

enum RemoteSdkInbound {
    User(RemotePrompt),
    ControlResponse(Value),
    ControlRequest(Value),
    EndSession,
}

struct RemoteSdkSeed {
    seed: StreamJsonStdinSeed,
    rx: tokio::sync::mpsc::UnboundedReceiver<RemoteSdkInbound>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedConnectUrl {
    server_url: String,
    auth_token: Option<String>,
}

fn parse_direct_connect_url(raw: &str) -> Result<ParsedConnectUrl> {
    let url = reqwest::Url::parse(raw).with_context(|| format!("Invalid connect URL: {raw}"))?;
    match url.scheme() {
        "cc" | "cc+unix" => {}
        other => anyhow::bail!("Unsupported connect URL scheme: {other}"),
    }
    let query_server_url = url
        .query_pairs()
        .find_map(|(key, value)| {
            matches!(key.as_ref(), "serverUrl" | "server_url" | "url").then(|| value.into_owned())
        })
        .filter(|value| !value.trim().is_empty());
    let auth_token = url
        .query_pairs()
        .find_map(|(key, value)| {
            matches!(key.as_ref(), "authToken" | "auth_token" | "token").then(|| value.into_owned())
        })
        .filter(|value| !value.trim().is_empty());

    let server_url = if let Some(server_url) = query_server_url {
        server_url
    } else if url.scheme() == "cc" {
        let host = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid connect URL: missing server URL"))?;
        let mut server_url = if host == "http" || host == "https" {
            format!("{host}:{}", url.path())
        } else {
            format!("https://{host}{}", url.path())
        };
        if server_url.ends_with('/') {
            server_url.pop();
        }
        server_url
    } else {
        anyhow::bail!("cc+unix connect URLs must include a serverUrl query parameter");
    };

    Ok(ParsedConnectUrl {
        server_url,
        auth_token,
    })
}

fn rewrite_direct_connect_args(
    args: impl IntoIterator<Item = String>,
) -> Result<(Vec<String>, Option<ParsedConnectUrl>, bool)> {
    let mut args = args.into_iter().collect::<Vec<_>>();
    if args.len() <= 1 {
        return Ok((args, None, false));
    }
    let Some(index) = args
        .iter()
        .skip(1)
        .position(|arg| arg.starts_with("cc://") || arg.starts_with("cc+unix://"))
        .map(|idx| idx + 1)
    else {
        return Ok((args, None, false));
    };
    let raw = args[index].clone();
    let parsed = parse_direct_connect_url(&raw)?;
    let print_mode = args.iter().any(|arg| arg == "-p" || arg == "--print");
    let dsp_flag = "--dangerously-skip-permissions";
    let dangerously_skip_permissions = args.iter().any(|arg| arg == dsp_flag);
    if print_mode {
        let bin = args[0].clone();
        let stripped = args
            .into_iter()
            .enumerate()
            .skip(1)
            .filter_map(|(i, arg)| (i != index && arg != dsp_flag).then_some(arg));
        let mut rewritten = vec![bin, "open".to_string(), raw];
        rewritten.extend(stripped);
        args = rewritten;
    } else {
        args.remove(index);
        args.retain(|arg| arg != dsp_flag);
    }
    Ok((args, Some(parsed), dangerously_skip_permissions))
}

fn sdk_content_to_prompt(content: &serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(blocks) => {
            let mut out = String::new();
            for block in blocks {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    out.push_str(text);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

async fn remote_prompt_from_sdk_message(
    value: &Value,
    delivery_event_id: Option<String>,
) -> Option<RemotePrompt> {
    let fields = claude_core::bridge::inbound_messages::extract_inbound_message_fields(value)?;
    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|session_id| !session_id.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| claude_core::api::client::get_session_id().clone());
    let content = match claude_core::bridge::inbound_attachments::InboundAttachmentResolver::from_runtime_config(session_id).await {
        Ok(resolver) => resolver.resolve_and_prepend(value, fields.content).await,
        Err(err) => {
            tracing::debug!(error = %err, "failed to build inbound attachment resolver");
            fields.content
        }
    };
    Some(RemotePrompt {
        content,
        delivery_event_id,
    })
}

fn parse_stream_json_stdin(stdin: &str) -> Result<StreamJsonStdinSeed> {
    let mut seed = StreamJsonStdinSeed::default();
    for line in stdin.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let message: serde_json::Value = serde_json::from_str(line)
            .map_err(|err| anyhow::anyhow!("Error parsing stream-json input: {err}"))?;
        match message.get("type").and_then(|v| v.as_str()) {
            Some("user") => {
                if let Some(content) = message
                    .get("message")
                    .and_then(|v| v.get("content"))
                    .and_then(sdk_content_to_prompt)
                {
                    seed.prompt = Some(match seed.prompt.take() {
                        Some(existing) if !existing.is_empty() => format!("{existing}\n{content}"),
                        _ => content,
                    });
                }
            }
            Some("control_request") => {
                let request = message.get("request").unwrap_or(&serde_json::Value::Null);
                if request.get("subtype").and_then(|v| v.as_str()) == Some("initialize") {
                    if let Some(system_prompt) =
                        request.get("systemPrompt").and_then(|v| v.as_str())
                    {
                        seed.system_prompt = Some(system_prompt.to_string());
                    }
                    if let Some(append_system_prompt) =
                        request.get("appendSystemPrompt").and_then(|v| v.as_str())
                    {
                        seed.append_system_prompt = Some(append_system_prompt.to_string());
                    }
                    if let Some(json_schema) = request.get("jsonSchema") {
                        seed.json_schema = Some(serde_json::to_string(json_schema)?);
                    }
                } else if matches!(
                    request.get("subtype").and_then(|v| v.as_str()),
                    Some("set_model" | "set_permission_mode")
                ) {
                    seed.control_requests.push(message);
                }
            }
            Some("keep_alive") | Some("update_environment_variables") => {}
            Some(_) | None => {}
        }
    }
    Ok(seed)
}

fn apply_restored_worker_metadata_to_cli(cli: &mut Cli, metadata: &Value) {
    if let Some(model) = metadata.get("model").and_then(|value| value.as_str()) {
        cli.model = Some(model.to_string());
    }
    if let Some(permission_mode) = metadata
        .get("permission_mode")
        .and_then(|value| value.as_str())
    {
        cli.permission_mode = Some(permission_mode.to_string());
    }
}

fn session_ingress_auth_token() -> Option<String> {
    std::env::var("CLAUDE_CODE_SESSION_ACCESS_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
}

fn convert_ws_url_to_session_ingress_post_url(ws_url: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(ws_url)?;
    match url.scheme() {
        "wss" => {
            url.set_scheme("https")
                .map_err(|_| anyhow::anyhow!("invalid sdk url scheme"))?;
        }
        "ws" => {
            url.set_scheme("http")
                .map_err(|_| anyhow::anyhow!("invalid sdk url scheme"))?;
        }
        "https" | "http" => {}
        other => return Err(anyhow::anyhow!("Unsupported protocol: {other}:")),
    }
    let mut pathname = url.path().replace("/ws/", "/session/");
    if !pathname.ends_with("/events") {
        if pathname.ends_with('/') {
            pathname.push_str("events");
        } else {
            pathname.push_str("/events");
        }
    }
    url.set_path(&pathname);
    Ok(url.to_string())
}

enum RemotePostConfig {
    Hybrid {
        post_url: String,
    },
    CcrV2 {
        session_url: String,
        events_url: String,
        delivery_url: String,
        internal_events_url: String,
        worker_url: String,
        worker_epoch: u64,
    },
}

fn remote_post_config_for_sdk_url(sdk_url: &str) -> Result<RemotePostConfig> {
    if claude_core::errors_util::is_env_truthy("CLAUDE_CODE_USE_CCR_V2") {
        let worker_epoch = std::env::var("CLAUDE_CODE_WORKER_EPOCH")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let session_url = sdk_url.trim_end_matches('/').to_string();
        Ok(RemotePostConfig::CcrV2 {
            events_url: format!("{session_url}/worker/events"),
            delivery_url: format!("{session_url}/worker/events/delivery"),
            internal_events_url: format!("{session_url}/worker/internal-events"),
            worker_url: format!("{session_url}/worker"),
            session_url,
            worker_epoch,
        })
    } else {
        Ok(RemotePostConfig::Hybrid {
            post_url: convert_ws_url_to_session_ingress_post_url(sdk_url)?,
        })
    }
}

fn remote_post_config_is_ccr_v2(config: &RemotePostConfig) -> bool {
    matches!(config, RemotePostConfig::CcrV2 { .. })
}

fn is_stream_event_value(value: &Value) -> bool {
    value.get("type").and_then(|value| value.as_str()) == Some("stream_event")
}

fn remote_stream_scope_key(value: &Value) -> String {
    let session_id = value
        .get("session_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let parent_tool_use_id = value
        .get("parent_tool_use_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    format!("{session_id}:{parent_tool_use_id}")
}

fn clear_remote_stream_accumulator_for_assistant(
    state: &mut RemoteStreamAccumulator,
    value: &Value,
) {
    if value.get("type").and_then(|value| value.as_str()) != Some("assistant") {
        return;
    }
    let Some(message_id) = value
        .get("message")
        .and_then(|message| message.get("id"))
        .and_then(|value| value.as_str())
    else {
        return;
    };
    state.by_message.remove(message_id);
    let scope = remote_stream_scope_key(value);
    if state.scope_to_message.get(&scope).map(String::as_str) == Some(message_id) {
        state.scope_to_message.remove(&scope);
    }
}

fn accumulate_remote_stream_events(
    buffered: Vec<Value>,
    state: &mut RemoteStreamAccumulator,
) -> Vec<Value> {
    let mut out = Vec::new();
    let mut touched: HashMap<(String, usize), usize> = HashMap::new();
    for msg in buffered {
        let event_type = msg
            .get("event")
            .and_then(|event| event.get("type"))
            .and_then(|value| value.as_str());
        match event_type {
            Some("message_start") => {
                if let Some(message_id) = msg
                    .get("event")
                    .and_then(|event| event.get("message"))
                    .and_then(|message| message.get("id"))
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned)
                {
                    let scope = remote_stream_scope_key(&msg);
                    if let Some(prev_id) = state.scope_to_message.insert(scope, message_id.clone())
                    {
                        state.by_message.remove(&prev_id);
                    }
                    state.by_message.insert(message_id, Vec::new());
                }
                out.push(msg);
            }
            Some("content_block_delta")
                if msg
                    .get("event")
                    .and_then(|event| event.get("delta"))
                    .and_then(|delta| delta.get("type"))
                    .and_then(|value| value.as_str())
                    == Some("text_delta") =>
            {
                let Some(message_id) = state
                    .scope_to_message
                    .get(&remote_stream_scope_key(&msg))
                    .cloned()
                else {
                    out.push(msg);
                    continue;
                };
                let Some(block_index) = msg
                    .get("event")
                    .and_then(|event| event.get("index"))
                    .and_then(|value| value.as_u64())
                    .and_then(|value| usize::try_from(value).ok())
                else {
                    out.push(msg);
                    continue;
                };
                let Some(delta_text) = msg
                    .get("event")
                    .and_then(|event| event.get("delta"))
                    .and_then(|delta| delta.get("text"))
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned)
                else {
                    out.push(msg);
                    continue;
                };
                let Some(blocks) = state.by_message.get_mut(&message_id) else {
                    out.push(msg);
                    continue;
                };
                if blocks.len() <= block_index {
                    blocks.resize_with(block_index + 1, Vec::new);
                }
                blocks[block_index].push(delta_text);
                let full_text = blocks[block_index].join("");
                let key = (message_id, block_index);
                if let Some(out_index) = touched.get(&key).copied() {
                    if let Some(snapshot) = out.get_mut(out_index) {
                        snapshot["event"]["delta"]["text"] = serde_json::json!(full_text);
                    }
                    continue;
                }
                let mut snapshot = msg;
                snapshot["event"]["delta"]["text"] = serde_json::json!(full_text);
                touched.insert(key, out.len());
                out.push(snapshot);
            }
            _ => out.push(msg),
        }
    }
    out
}

fn flush_remote_stream_event_buffer(
    stream_event_buffer: &mut Vec<Value>,
    stream_accumulator: &mut RemoteStreamAccumulator,
    post_buffer: &mut Vec<Value>,
) {
    if stream_event_buffer.is_empty() {
        return;
    }
    let buffered = std::mem::take(stream_event_buffer);
    post_buffer.extend(accumulate_remote_stream_events(
        buffered,
        stream_accumulator,
    ));
}

fn install_remote_stream_json_poster(sdk_url: &str) -> Result<()> {
    let config = remote_post_config_for_sdk_url(sdk_url)?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let (delivery_tx, mut delivery_rx) =
        tokio::sync::mpsc::unbounded_channel::<CcrDeliveryUpdate>();
    let (internal_tx, mut internal_rx) =
        tokio::sync::mpsc::unbounded_channel::<InternalTranscriptEvent>();
    let (worker_patch_tx, mut worker_patch_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let _ = REMOTE_STREAM_JSON_TX.set(tx);
    let _ = REMOTE_CCR_DELIVERY_TX.set(delivery_tx);
    let _ = REMOTE_CCR_WORKER_PATCH_TX.set(worker_patch_tx);
    if matches!(config, RemotePostConfig::CcrV2 { .. }) {
        claude_core::session::storage::set_internal_event_sender(internal_tx);
    }
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        report_remote_ccr_worker_patch(
            &client,
            &config,
            serde_json::json!({
                "worker_status": "idle",
                "external_metadata": {
                    "pending_action": null,
                    "task_summary": null,
                },
            }),
        )
        .await;
        let mut buffer: Vec<Value> = Vec::new();
        let mut stream_event_buffer: Vec<Value> = Vec::new();
        let mut stream_accumulator = RemoteStreamAccumulator::default();
        let mut delivery_buffer: Vec<CcrDeliveryUpdate> = Vec::new();
        let mut internal_buffer: Vec<InternalTranscriptEvent> = Vec::new();
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let heartbeat_delay = std::time::Duration::from_secs(20);
        let heartbeat = tokio::time::sleep(heartbeat_delay);
        tokio::pin!(heartbeat);

        loop {
            tokio::select! {
                maybe_value = rx.recv() => {
                    let Some(value) = maybe_value else {
                        break;
                    };
                    report_remote_stream_json_state(&client, &config, &value).await;
                    if remote_post_config_is_ccr_v2(&config) && is_stream_event_value(&value) {
                        stream_event_buffer.push(value);
                        continue;
                    }
                    if remote_post_config_is_ccr_v2(&config) {
                        flush_remote_stream_event_buffer(
                            &mut stream_event_buffer,
                            &mut stream_accumulator,
                            &mut buffer,
                        );
                        clear_remote_stream_accumulator_for_assistant(&mut stream_accumulator, &value);
                    }
                    buffer.push(value);
                    if buffer.len() >= 500 {
                        post_remote_stream_json_batch(&client, &config, &mut buffer).await;
                    }
                }
                maybe_delivery = delivery_rx.recv() => {
                    let Some(update) = maybe_delivery else {
                        continue;
                    };
                    delivery_buffer.push(update);
                    if delivery_buffer.len() >= 64 {
                        post_remote_ccr_delivery_batch(&client, &config, &mut delivery_buffer).await;
                    }
                }
                maybe_internal = internal_rx.recv() => {
                    let Some(event) = maybe_internal else {
                        continue;
                    };
                    internal_buffer.push(event);
                    if internal_buffer.len() >= 100 {
                        post_remote_ccr_internal_event_batch(&client, &config, &mut internal_buffer).await;
                    }
                }
                maybe_patch = worker_patch_rx.recv() => {
                    let Some(patch) = maybe_patch else {
                        continue;
                    };
                    report_remote_ccr_worker_patch(&client, &config, patch).await;
                }
                _ = interval.tick() => {
                    if remote_post_config_is_ccr_v2(&config) {
                        flush_remote_stream_event_buffer(
                            &mut stream_event_buffer,
                            &mut stream_accumulator,
                            &mut buffer,
                        );
                    }
                    if !buffer.is_empty() {
                        post_remote_stream_json_batch(&client, &config, &mut buffer).await;
                    }
                    if !delivery_buffer.is_empty() {
                        post_remote_ccr_delivery_batch(&client, &config, &mut delivery_buffer).await;
                    }
                    if !internal_buffer.is_empty() {
                        post_remote_ccr_internal_event_batch(&client, &config, &mut internal_buffer).await;
                    }
                }
                _ = &mut heartbeat => {
                    post_remote_ccr_heartbeat(&client, &config).await;
                    heartbeat.as_mut().reset(tokio::time::Instant::now() + heartbeat_delay);
                }
            }
        }
        if !stream_event_buffer.is_empty() {
            flush_remote_stream_event_buffer(
                &mut stream_event_buffer,
                &mut stream_accumulator,
                &mut buffer,
            );
        }
        if !buffer.is_empty() {
            post_remote_stream_json_batch(&client, &config, &mut buffer).await;
        }
        if !delivery_buffer.is_empty() {
            post_remote_ccr_delivery_batch(&client, &config, &mut delivery_buffer).await;
        }
        if !internal_buffer.is_empty() {
            post_remote_ccr_internal_event_batch(&client, &config, &mut internal_buffer).await;
        }
    });
    Ok(())
}

fn report_remote_ccr_delivery(event_id: &str, status: &'static str) {
    if event_id.is_empty() {
        return;
    }
    if let Some(tx) = REMOTE_CCR_DELIVERY_TX.get() {
        let _ = tx.send(CcrDeliveryUpdate {
            event_id: event_id.to_string(),
            status,
        });
    }
}

fn to_external_permission_mode(
    mode: &claude_core::permissions::types::PermissionMode,
) -> &'static str {
    match mode {
        claude_core::permissions::types::PermissionMode::Default => "default",
        claude_core::permissions::types::PermissionMode::AcceptEdits => "acceptEdits",
        claude_core::permissions::types::PermissionMode::Auto => "default",
        claude_core::permissions::types::PermissionMode::Plan => "plan",
        claude_core::permissions::types::PermissionMode::BypassPermissions => "bypassPermissions",
        claude_core::permissions::types::PermissionMode::DontAsk => "dontAsk",
        claude_core::permissions::types::PermissionMode::Bubble => "default",
    }
}

fn report_remote_ccr_worker_patch_value(patch: Value) {
    if let Some(tx) = REMOTE_CCR_WORKER_PATCH_TX.get() {
        let _ = tx.send(patch);
    }
}

fn report_remote_ccr_metadata(metadata: Value) {
    report_remote_ccr_worker_patch_value(serde_json::json!({
        "external_metadata": metadata,
    }));
}

fn report_remote_ccr_permission_mode_metadata(
    mode: &claude_core::permissions::types::PermissionMode,
    is_ultraplan_mode: Option<bool>,
) {
    report_remote_ccr_metadata(serde_json::json!({
        "permission_mode": to_external_permission_mode(mode),
        "is_ultraplan_mode": is_ultraplan_mode,
    }));
}

fn ccr_heartbeat_url(config: &RemotePostConfig) -> Option<String> {
    let RemotePostConfig::CcrV2 { session_url, .. } = config else {
        return None;
    };
    Some(format!("{session_url}/worker/heartbeat"))
}

async fn post_remote_ccr_heartbeat(client: &reqwest::Client, config: &RemotePostConfig) {
    let RemotePostConfig::CcrV2 {
        session_url,
        worker_epoch,
        ..
    } = config
    else {
        return;
    };
    let Some(token) = session_ingress_auth_token() else {
        return;
    };
    let Some(url) = ccr_heartbeat_url(config) else {
        return;
    };
    let Some(session_id) = session_url
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let _ = client
        .post(url)
        .bearer_auth(token)
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "session_id": session_id,
            "worker_epoch": worker_epoch,
        }))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
}

fn ccr_delivery_body(worker_epoch: u64, updates: &[CcrDeliveryUpdate]) -> serde_json::Value {
    serde_json::json!({
        "worker_epoch": worker_epoch,
        "updates": updates.iter().map(|update| {
            serde_json::json!({
                "event_id": update.event_id,
                "status": update.status,
            })
        }).collect::<Vec<_>>(),
    })
}

fn ccr_internal_events_body(
    worker_epoch: u64,
    events: &[InternalTranscriptEvent],
) -> serde_json::Value {
    serde_json::json!({
        "worker_epoch": worker_epoch,
        "events": events.iter().map(|event| {
            let mut wrapped = serde_json::json!({
                "payload": event.payload.clone(),
            });
            if event.is_compaction {
                wrapped["is_compaction"] = serde_json::json!(true);
            }
            if let Some(agent_id) = &event.agent_id {
                wrapped["agent_id"] = serde_json::json!(agent_id);
            }
            wrapped
        }).collect::<Vec<_>>(),
    })
}

async fn post_remote_ccr_delivery_batch(
    client: &reqwest::Client,
    config: &RemotePostConfig,
    buffer: &mut Vec<CcrDeliveryUpdate>,
) {
    let RemotePostConfig::CcrV2 {
        delivery_url,
        worker_epoch,
        ..
    } = config
    else {
        buffer.clear();
        return;
    };
    let Some(token) = session_ingress_auth_token() else {
        buffer.clear();
        return;
    };
    let updates = std::mem::take(buffer);
    let body = ccr_delivery_body(*worker_epoch, &updates);
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let response = client
            .post(delivery_url)
            .bearer_auth(&token)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => break,
            Ok(resp)
                if resp.status().is_client_error()
                    && resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS =>
            {
                break;
            }
            _ if attempt >= 5 => break,
            _ => {
                let delay_ms = 500u64
                    .saturating_mul(2u64.saturating_pow(attempt - 1))
                    .min(8_000);
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }
    }
}

async fn post_remote_ccr_internal_event_batch(
    client: &reqwest::Client,
    config: &RemotePostConfig,
    buffer: &mut Vec<InternalTranscriptEvent>,
) {
    let RemotePostConfig::CcrV2 {
        internal_events_url,
        worker_epoch,
        ..
    } = config
    else {
        buffer.clear();
        return;
    };
    let Some(token) = session_ingress_auth_token() else {
        buffer.clear();
        return;
    };
    let events = std::mem::take(buffer);
    let body = ccr_internal_events_body(*worker_epoch, &events);
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let response = client
            .post(internal_events_url)
            .bearer_auth(&token)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => break,
            Ok(resp)
                if resp.status().is_client_error()
                    && resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS =>
            {
                break;
            }
            _ if attempt >= 5 => break,
            _ => {
                let delay_ms = 500u64
                    .saturating_mul(2u64.saturating_pow(attempt - 1))
                    .min(8_000);
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }
    }
}

fn ccr_internal_events_read_url(
    sdk_url: &str,
    subagents: bool,
    cursor: Option<&str>,
) -> Result<String> {
    let mut url = reqwest::Url::parse(sdk_url)?;
    let mut pathname = url.path().trim_end_matches('/').to_string();
    pathname.push_str("/worker/internal-events");
    url.set_path(&pathname);
    url.set_query(None);
    if subagents || cursor.is_some_and(|value| !value.is_empty()) {
        let mut query = url.query_pairs_mut();
        if subagents {
            query.append_pair("subagents", "true");
        }
        if let Some(cursor) = cursor.filter(|value| !value.is_empty()) {
            query.append_pair("cursor", cursor);
        }
    }
    Ok(url.to_string())
}

fn ccr_worker_read_url(sdk_url: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(sdk_url)?;
    let mut pathname = url.path().trim_end_matches('/').to_string();
    pathname.push_str("/worker");
    url.set_path(&pathname);
    url.set_query(None);
    Ok(url.to_string())
}

async fn read_remote_ccr_worker_metadata(sdk_url: &str) -> Result<Option<Value>> {
    let Some(token) = session_ingress_auth_token() else {
        return Ok(None);
    };
    let url = ccr_worker_read_url(sdk_url)?;
    let client = reqwest::Client::new();
    let Some(page) = get_remote_ccr_internal_events_page(&client, &url, &token).await? else {
        return Ok(None);
    };
    Ok(page
        .get("worker")
        .and_then(|worker| worker.get("external_metadata"))
        .cloned())
}

async fn get_remote_ccr_internal_events_page(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<Option<Value>> {
    for attempt in 1..=10u32 {
        let response = client
            .get(url)
            .bearer_auth(token)
            .header("anthropic-version", "2023-06-01")
            .header(
                "User-Agent",
                claude_core::user_agent::get_claude_code_user_agent(),
            )
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => {
                return Ok(Some(resp.json::<Value>().await?));
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::CONFLICT => {
                anyhow::bail!("CCRClient: Epoch mismatch (409)");
            }
            Ok(_) | Err(_) if attempt < 10 => {
                let delay_ms = 500u64
                    .saturating_mul(2u64.saturating_pow(attempt - 1))
                    .min(30_000);
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Ok(_) | Err(_) => return Ok(None),
        }
    }
    Ok(None)
}

async fn read_remote_ccr_internal_events(
    sdk_url: &str,
    subagents: bool,
) -> Result<Option<Vec<CcrInternalEvent>>> {
    let Some(token) = session_ingress_auth_token() else {
        return Ok(None);
    };
    let client = reqwest::Client::new();
    let mut cursor: Option<String> = None;
    let mut events = Vec::new();
    loop {
        let url = ccr_internal_events_read_url(sdk_url, subagents, cursor.as_deref())?;
        let Some(page) = get_remote_ccr_internal_events_page(&client, &url, &token).await? else {
            return Ok(None);
        };
        if let Some(data) = page.get("data").and_then(|value| value.as_array()) {
            for item in data {
                if let Ok(event) = serde_json::from_value::<CcrInternalEvent>(item.clone()) {
                    events.push(event);
                }
            }
        }
        cursor = page
            .get("next_cursor")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Ok(Some(events))
}

fn group_ccr_subagent_internal_events(
    events: Vec<CcrInternalEvent>,
) -> HashMap<String, Vec<Value>> {
    let mut by_agent: HashMap<String, Vec<Value>> = HashMap::new();
    for event in events {
        let Some(agent_id) = event.agent_id.filter(|value| !value.is_empty()) else {
            continue;
        };
        by_agent.entry(agent_id).or_default().push(event.payload);
    }
    by_agent
}

async fn hydrate_remote_ccr_internal_events(sdk_url: &str, session_id: &str) -> Result<bool> {
    let Some(events) = read_remote_ccr_internal_events(sdk_url, false).await? else {
        return Ok(false);
    };
    let entries = events
        .into_iter()
        .map(|event| event.payload)
        .collect::<Vec<_>>();
    let storage = claude_core::session::storage::SessionStorage::new(session_id)?;
    storage.replace_transcript(&entries)?;
    if let Some(subagent_events) = read_remote_ccr_internal_events(sdk_url, true).await? {
        for (agent_id, agent_entries) in group_ccr_subagent_internal_events(subagent_events) {
            let storage = claude_core::session::storage::SessionStorage::new(&agent_id)?;
            storage.replace_transcript(&agent_entries)?;
        }
    }
    Ok(!entries.is_empty())
}

fn ccr_worker_state_for_event(event: &Value) -> Option<Value> {
    match event.get("type").and_then(|value| value.as_str()) {
        Some("user") | Some("control_response") => {
            Some(serde_json::json!({"worker_status": "running"}))
        }
        Some("result") => Some(serde_json::json!({"worker_status": "idle"})),
        Some("control_request")
            if event
                .get("request")
                .and_then(|request| request.get("subtype"))
                .and_then(|value| value.as_str())
                == Some("can_use_tool") =>
        {
            let request = event.get("request").unwrap_or(&Value::Null);
            let mut details = serde_json::json!({
                "tool_name": request
                    .get("tool_name")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
                "action_description": request
                    .get("decision_reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
                "request_id": event
                    .get("request_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
            });
            if let Some(tool_use_id) = request.get("tool_use_id").and_then(|value| value.as_str()) {
                details["tool_use_id"] = serde_json::json!(tool_use_id);
            }
            if let Some(input) = request.get("input").filter(|value| value.is_object()) {
                details["input"] = input.clone();
            }
            Some(serde_json::json!({
                "worker_status": "requires_action",
                "requires_action_details": details,
            }))
        }
        _ => None,
    }
}

fn ccr_worker_patch_for_event(event: &Value) -> Option<Value> {
    let mut body = ccr_worker_state_for_event(event)?;
    match body.get("worker_status").and_then(|value| value.as_str()) {
        Some("requires_action") => {
            if let Some(details) = body.get("requires_action_details").cloned() {
                body["external_metadata"] = serde_json::json!({
                    "pending_action": details,
                });
            }
        }
        Some("running") | Some("idle") => {
            body["external_metadata"] = serde_json::json!({
                "pending_action": null,
            });
        }
        _ => {}
    }
    Some(body)
}

async fn report_remote_ccr_worker_patch(
    client: &reqwest::Client,
    config: &RemotePostConfig,
    mut body: Value,
) {
    let RemotePostConfig::CcrV2 {
        worker_url,
        worker_epoch,
        ..
    } = config
    else {
        return;
    };
    let Some(token) = session_ingress_auth_token() else {
        return;
    };
    body["worker_epoch"] = serde_json::json!(worker_epoch);
    let _ = client
        .put(worker_url)
        .bearer_auth(token)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
}

async fn report_remote_stream_json_state(
    client: &reqwest::Client,
    config: &RemotePostConfig,
    event: &Value,
) {
    let Some(body) = ccr_worker_patch_for_event(event) else {
        return;
    };
    report_remote_ccr_worker_patch(client, config, body).await;
}

async fn post_remote_stream_json_batch(
    client: &reqwest::Client,
    config: &RemotePostConfig,
    buffer: &mut Vec<Value>,
) {
    let Some(token) = session_ingress_auth_token() else {
        buffer.clear();
        return;
    };
    let events = std::mem::take(buffer);
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let request = match config {
            RemotePostConfig::Hybrid { post_url } => client
                .post(post_url)
                .bearer_auth(&token)
                .json(&serde_json::json!({ "events": events.clone() })),
            RemotePostConfig::CcrV2 {
                events_url,
                worker_epoch,
                ..
            } => {
                let wrapped = events
                    .iter()
                    .map(|event| {
                        let mut payload = event.clone();
                        if payload
                            .get("uuid")
                            .and_then(|value| value.as_str())
                            .is_none()
                        {
                            payload["uuid"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
                        }
                        let mut wrapped = serde_json::json!({ "payload": payload });
                        if event.get("type").and_then(|value| value.as_str())
                            == Some("stream_event")
                        {
                            wrapped["ephemeral"] = serde_json::json!(true);
                        }
                        wrapped
                    })
                    .collect::<Vec<_>>();
                client
                    .post(events_url)
                    .bearer_auth(&token)
                    .header("anthropic-version", "2023-06-01")
                    .json(&serde_json::json!({
                        "worker_epoch": worker_epoch,
                        "events": wrapped,
                    }))
            }
        };
        let response = request
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => break,
            Ok(resp)
                if resp.status().is_client_error()
                    && resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS =>
            {
                break;
            }
            _ if attempt >= 5 => break,
            _ => {
                let delay_ms = 500u64
                    .saturating_mul(2u64.saturating_pow(attempt - 1))
                    .min(8_000);
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }
    }
}

fn apply_update_environment_variables(message: &Value) {
    let Some(vars) = message.get("variables").and_then(|value| value.as_object()) else {
        return;
    };
    for (key, value) in vars {
        if let Some(value) = value.as_str() {
            std::env::set_var(key, value);
        }
    }
}

fn spawn_remote_stdin_environment_updater() {
    tokio::task::spawn_blocking(|| {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(|line| line.ok()) {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if message.get("type").and_then(|value| value.as_str())
                == Some("update_environment_variables")
            {
                apply_update_environment_variables(&message);
            }
        }
    });
}

fn remote_control_response_success(request_id: &str, response: Value) -> Value {
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": response,
        },
    })
}

fn remote_control_response_error(request_id: &str, error: impl Into<String>) -> Value {
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": request_id,
            "error": error.into(),
        },
    })
}

fn remote_permission_request(
    request_id: &str,
    tool_name: &str,
    tool_use_id: &str,
    input: &Value,
    message: &str,
    suggestions: &Option<Vec<claude_core::permissions::types::PermissionUpdate>>,
    blocked_path: &Option<String>,
) -> Value {
    let mut request = serde_json::json!({
        "subtype": "can_use_tool",
        "tool_name": tool_name,
        "input": input,
        "decision_reason": message,
        "tool_use_id": tool_use_id,
    });
    if let Some(suggestions) = suggestions {
        request["permission_suggestions"] =
            serde_json::to_value(suggestions).unwrap_or_else(|_| serde_json::json!([]));
    }
    if let Some(blocked_path) = blocked_path {
        request["blocked_path"] = serde_json::json!(blocked_path);
    }
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": request,
    })
}

async fn wait_remote_permission_response(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<RemoteSdkInbound>,
    request_id: &str,
    queued_prompts: &mut std::collections::VecDeque<RemotePrompt>,
    queued_control_requests: &mut std::collections::VecDeque<Value>,
) -> Option<Value> {
    while let Some(inbound) = rx.recv().await {
        match inbound {
            RemoteSdkInbound::User(prompt) => queued_prompts.push_back(prompt),
            RemoteSdkInbound::EndSession => return None,
            RemoteSdkInbound::ControlRequest(value) => queued_control_requests.push_back(value),
            RemoteSdkInbound::ControlResponse(value) => {
                let matches_request = value
                    .get("response")
                    .and_then(|response| response.get("request_id"))
                    .and_then(|value| value.as_str())
                    == Some(request_id);
                if matches_request {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn parse_remote_permission_response(
    value: &Value,
) -> Result<(
    bool,
    Option<Value>,
    Option<Vec<claude_core::permissions::types::PermissionUpdate>>,
    String,
)> {
    let response = value.get("response").unwrap_or(&Value::Null);
    if response.get("subtype").and_then(|value| value.as_str()) == Some("error") {
        let message = response
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("Permission response error")
            .to_string();
        return Ok((false, None, None, message));
    }
    let result = response.get("response").unwrap_or(&Value::Null);
    match result.get("behavior").and_then(|value| value.as_str()) {
        Some("allow") => {
            let updated_input = result
                .get("updatedInput")
                .or_else(|| result.get("updated_input"))
                .filter(|value| value.as_object().is_some_and(|obj| !obj.is_empty()))
                .cloned();
            let updated_permissions = result
                .get("updatedPermissions")
                .or_else(|| result.get("updated_permissions"))
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?;
            Ok((true, updated_input, updated_permissions, String::new()))
        }
        Some("deny") => {
            let message = result
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("Permission denied")
                .to_string();
            Ok((false, None, None, message))
        }
        other => Err(anyhow::anyhow!(
            "Invalid permission response behavior: {}",
            other.unwrap_or("missing")
        )),
    }
}

async fn apply_remote_control_request(
    value: Value,
    query_engine: &mut claude_core::query::engine::QueryEngine,
    perm_ctx: &mut claude_core::permissions::types::ToolPermissionContext,
    active_model: &mut String,
    active_model_display: &mut String,
) -> Result<()> {
    let request_id = value
        .get("request_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let request = value.get("request").unwrap_or(&Value::Null);
    match request.get("subtype").and_then(|value| value.as_str()) {
        Some("set_model") => {
            let requested_model = request
                .get("model")
                .and_then(|value| value.as_str())
                .unwrap_or("default");
            let resolved_model = if requested_model == "default" {
                default_main_loop_model_setting().await
            } else {
                requested_model.to_string()
            };
            *active_model = normalize_model_name(&resolved_model);
            *active_model_display = active_model.clone();
            query_engine.set_model(active_model.clone());
            report_remote_ccr_metadata(serde_json::json!({ "model": active_model }));
            query_engine.add_user_message(
                "<local-command-caveat>Caveat: The messages below were generated by the user while running local commands. DO NOT respond to these messages or otherwise consider them in your response unless the user explicitly asks you to.</local-command-caveat>",
            );
            query_engine.add_user_message(&format!(
                "<command-name>/model</command-name>\n            <command-message>model</command-message>\n            <command-args>{requested_model}</command-args>"
            ));
            query_engine.add_user_message(&format!(
                "<local-command-stdout>Set model to {}</local-command-stdout>",
                active_model_display
            ));
            if !request_id.is_empty() {
                emit_stream_json(remote_control_response_success(
                    &request_id,
                    serde_json::json!({}),
                ));
            }
        }
        Some("set_permission_mode") => {
            let requested_mode = request
                .get("mode")
                .and_then(|value| value.as_str())
                .unwrap_or("default");
            let next_mode =
                claude_core::permissions::types::PermissionMode::from_string(requested_mode);
            if next_mode == claude_core::permissions::types::PermissionMode::BypassPermissions
                && !perm_ctx.is_bypass_permissions_mode_available
            {
                if !request_id.is_empty() {
                    emit_stream_json(remote_control_response_error(
                        &request_id,
                        "Cannot set permission mode to bypassPermissions because the session was not launched with --dangerously-skip-permissions",
                    ));
                }
                return Ok(());
            }
            if next_mode == claude_core::permissions::types::PermissionMode::Auto
                && perm_ctx.is_auto_mode_available == Some(false)
            {
                if !request_id.is_empty() {
                    emit_stream_json(remote_control_response_error(
                        &request_id,
                        "Cannot set permission mode to auto",
                    ));
                }
                return Ok(());
            }
            let previous_mode = perm_ctx.mode.clone();
            let mut next_ctx = claude_core::permissions::transition_permission_mode(
                &previous_mode,
                &next_mode,
                perm_ctx.clone(),
            );
            next_ctx.mode = next_mode.clone();
            *perm_ctx = next_ctx;
            let is_ultraplan_mode = request.get("ultraplan").and_then(|value| value.as_bool());
            report_remote_ccr_permission_mode_metadata(&next_mode, is_ultraplan_mode);
            if !request_id.is_empty() {
                emit_stream_json(remote_control_response_success(
                    &request_id,
                    serde_json::json!({ "mode": requested_mode }),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_remote_sdk_non_user_line(
    value: &Value,
    tx: Option<&tokio::sync::mpsc::UnboundedSender<RemoteSdkInbound>>,
) -> Result<Option<StreamJsonStdinSeed>> {
    match value.get("type").and_then(|value| value.as_str()) {
        Some("update_environment_variables") => {
            apply_update_environment_variables(value);
            Ok(None)
        }
        Some("keep_alive") => Ok(None),
        Some("control_request") => {
            let request = value.get("request").unwrap_or(&Value::Null);
            if request.get("subtype").and_then(|value| value.as_str()) == Some("initialize") {
                let parsed = parse_stream_json_stdin(&(value.to_string() + "\n"))?;
                if let Some(request_id) = value.get("request_id").and_then(|v| v.as_str()) {
                    emit_stream_json(remote_control_response_success(
                        request_id,
                        serde_json::json!({}),
                    ));
                }
                Ok(Some(parsed))
            } else if request.get("subtype").and_then(|value| value.as_str()) == Some("end_session")
            {
                if let Some(request_id) = value.get("request_id").and_then(|v| v.as_str()) {
                    emit_stream_json(remote_control_response_success(
                        request_id,
                        serde_json::json!({}),
                    ));
                }
                if let Some(tx) = tx {
                    let _ = tx.send(RemoteSdkInbound::EndSession);
                }
                Ok(None)
            } else {
                if let Some(tx) = tx {
                    let _ = tx.send(RemoteSdkInbound::ControlRequest(value.clone()));
                }
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

fn spawn_remote_sdk_background_reader(
    mut read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    mut buffered: String,
    tx: tokio::sync::mpsc::UnboundedSender<RemoteSdkInbound>,
) {
    tokio::spawn(async move {
        use futures_util::StreamExt;
        loop {
            while let Some(newline) = buffered.find('\n') {
                let line = buffered[..newline].to_string();
                buffered = buffered[newline + 1..].to_string();
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                match value.get("type").and_then(|value| value.as_str()) {
                    Some("user") => {
                        if let Some(prompt) = remote_prompt_from_sdk_message(&value, None).await {
                            let _ = tx.send(RemoteSdkInbound::User(prompt));
                        }
                    }
                    Some("control_response") => {
                        let _ = tx.send(RemoteSdkInbound::ControlResponse(value));
                    }
                    _ => {
                        let _ = handle_remote_sdk_non_user_line(&value, Some(&tx));
                    }
                }
            }

            let Some(message) = read.next().await else {
                break;
            };
            let Ok(message) = message else {
                break;
            };
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                buffered.push_str(&text);
            }
        }
    });
}

fn ccr_v2_sse_url(sdk_url: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(sdk_url)?;
    match url.scheme() {
        "https" | "http" => {}
        "wss" => {
            url.set_scheme("https")
                .map_err(|_| anyhow::anyhow!("invalid sdk url scheme"))?;
        }
        "ws" => {
            url.set_scheme("http")
                .map_err(|_| anyhow::anyhow!("invalid sdk url scheme"))?;
        }
        other => return Err(anyhow::anyhow!("Unsupported protocol: {other}:")),
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    path.push_str("/worker/events/stream");
    url.set_path(&path);
    Ok(url.to_string())
}

async fn handle_remote_sdk_seed_line(value: Value, seed: &mut StreamJsonStdinSeed) -> Result<bool> {
    match value.get("type").and_then(|value| value.as_str()) {
        Some("user") => {
            if let Some(prompt) = remote_prompt_from_sdk_message(&value, None).await {
                seed.prompt = sdk_content_to_prompt(&prompt.content);
                seed.prompt_content = Some(prompt.content);
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Some("control_request") => {
            let subtype = value
                .get("request")
                .and_then(|request| request.get("subtype"))
                .and_then(|value| value.as_str());
            if matches!(subtype, Some("set_model" | "set_permission_mode")) {
                seed.control_requests.push(value);
                Ok(false)
            } else {
                if let Some(parsed) = handle_remote_sdk_non_user_line(&value, None)? {
                    if parsed.system_prompt.is_some() {
                        seed.system_prompt = parsed.system_prompt;
                    }
                    if parsed.append_system_prompt.is_some() {
                        seed.append_system_prompt = parsed.append_system_prompt;
                    }
                    if parsed.json_schema.is_some() {
                        seed.json_schema = parsed.json_schema;
                    }
                }
                Ok(false)
            }
        }
        _ => {
            if let Some(parsed) = handle_remote_sdk_non_user_line(&value, None)? {
                if parsed.system_prompt.is_some() {
                    seed.system_prompt = parsed.system_prompt;
                }
                if parsed.append_system_prompt.is_some() {
                    seed.append_system_prompt = parsed.append_system_prompt;
                }
                if parsed.json_schema.is_some() {
                    seed.json_schema = parsed.json_schema;
                }
            }
            Ok(false)
        }
    }
}

#[derive(Clone, Debug)]
struct CcrStreamEvent {
    event_id: Option<String>,
    payload: Option<Value>,
}

fn parse_sse_payloads(buffer: &mut String) -> Vec<CcrStreamEvent> {
    let mut out = Vec::new();
    while let Some(frame_end) = buffer.find("\n\n") {
        let frame = buffer[..frame_end].to_string();
        *buffer = buffer[frame_end + 2..].to_string();
        let mut event_type = None;
        let mut data_lines = Vec::new();
        for line in frame.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                event_type = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                data_lines.push(rest.trim_start().to_string());
            }
        }
        if event_type.as_deref() != Some("client_event") || data_lines.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(&data_lines.join("\n")) else {
            continue;
        };
        out.push(CcrStreamEvent {
            event_id: event
                .get("event_id")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            payload: event.get("payload").cloned(),
        });
    }
    out
}

fn spawn_remote_ccr_background_reader(
    mut stream: impl futures_util::Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>>
        + Unpin
        + Send
        + 'static,
    mut buffer: String,
    tx: tokio::sync::mpsc::UnboundedSender<RemoteSdkInbound>,
) {
    tokio::spawn(async move {
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else {
                break;
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            for event in parse_sse_payloads(&mut buffer) {
                if let Some(event_id) = event.event_id.as_deref() {
                    report_remote_ccr_delivery(event_id, "received");
                }
                let Some(payload) = event.payload else {
                    continue;
                };
                match payload.get("type").and_then(|value| value.as_str()) {
                    Some("user") => {
                        if let Some(prompt) =
                            remote_prompt_from_sdk_message(&payload, event.event_id.clone()).await
                        {
                            let _ = tx.send(RemoteSdkInbound::User(prompt));
                        }
                    }
                    Some("control_response") => {
                        let _ = tx.send(RemoteSdkInbound::ControlResponse(payload));
                    }
                    _ => {
                        let _ = handle_remote_sdk_non_user_line(&payload, Some(&tx));
                    }
                }
            }
        }
    });
}

async fn read_remote_ccr_seed(sdk_url: &str) -> Result<RemoteSdkSeed> {
    use futures_util::StreamExt;

    let restored_worker_metadata = read_remote_ccr_worker_metadata(sdk_url).await?;
    install_remote_stream_json_poster(sdk_url)?;
    spawn_remote_stdin_environment_updater();

    let sse_url = ccr_v2_sse_url(sdk_url)?;
    let Some(token) = session_ingress_auth_token() else {
        anyhow::bail!("missing CLAUDE_CODE_SESSION_ACCESS_TOKEN for CCR v2 remote session");
    };
    let client = reqwest::Client::new();
    let response = client
        .get(&sse_url)
        .bearer_auth(token)
        .header("accept", "text/event-stream")
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .with_context(|| format!("connecting remote CCR v2 SSE URL {sse_url}"))?;
    if !response.status().is_success() {
        anyhow::bail!("remote CCR v2 SSE failed: {}", response.status());
    }

    let mut stream = response.bytes_stream();
    let mut seed = StreamJsonStdinSeed {
        restored_worker_metadata,
        ..Default::default()
    };
    let mut buffer = String::new();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading remote CCR v2 SSE message")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        for event in parse_sse_payloads(&mut buffer) {
            if let Some(event_id) = event.event_id.as_deref() {
                report_remote_ccr_delivery(event_id, "received");
            }
            let Some(payload) = event.payload else {
                continue;
            };
            if handle_remote_sdk_seed_line(payload, &mut seed).await? {
                seed.prompt_delivery_event_id = event.event_id.clone();
                spawn_remote_ccr_background_reader(stream, buffer, tx);
                return Ok(RemoteSdkSeed { seed, rx });
            }
        }
    }
    Ok(RemoteSdkSeed { seed, rx })
}

async fn read_remote_sdk_seed(sdk_url: &str) -> Result<RemoteSdkSeed> {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    if claude_core::errors_util::is_env_truthy("CLAUDE_CODE_USE_CCR_V2") {
        return read_remote_ccr_seed(sdk_url).await;
    }

    install_remote_stream_json_poster(sdk_url)?;
    spawn_remote_stdin_environment_updater();

    let mut request = sdk_url.into_client_request()?;
    if let Some(token) = session_ingress_auth_token() {
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .context("building session ingress authorization header")?,
        );
    }
    if let Ok(version) = std::env::var("CLAUDE_CODE_ENVIRONMENT_RUNNER_VERSION") {
        if !version.is_empty() {
            request.headers_mut().insert(
                "x-environment-runner-version",
                version
                    .parse()
                    .context("building environment runner version header")?,
            );
        }
    }

    let (stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .with_context(|| format!("connecting remote SDK URL {sdk_url}"))?;
    let (_, mut read) = stream.split();
    let mut seed = StreamJsonStdinSeed::default();
    let mut buffered = String::new();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    while let Some(message) = read.next().await {
        let message = message.context("reading remote SDK WebSocket message")?;
        let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
            continue;
        };
        buffered.push_str(&text);
        while let Some(newline) = buffered.find('\n') {
            let line = buffered[..newline].to_string();
            buffered = buffered[newline + 1..].to_string();
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line)
                .map_err(|err| anyhow::anyhow!("Error parsing remote stream-json input: {err}"))?;
            if handle_remote_sdk_seed_line(value, &mut seed).await? {
                spawn_remote_sdk_background_reader(read, buffered, tx);
                return Ok(RemoteSdkSeed { seed, rx });
            }
        }
    }

    Ok(RemoteSdkSeed { seed, rx })
}

fn direct_connect_message_text(value: &Value) -> String {
    let content = value.pointer("/message/content").unwrap_or(&Value::Null);
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                Some("text") => block.get("text").and_then(Value::as_str),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

async fn run_direct_connect_headless(
    config: claude_core::bridge::direct_connect::DirectConnectConfig,
    prompt: String,
    output_format: OutputFormat,
) -> Result<()> {
    let (inbound_tx, mut inbound_rx) = tokio::sync::mpsc::channel::<
        claude_core::bridge::direct_connect::DirectConnectInbound,
    >(128);
    let ws =
        claude_core::bridge::direct_connect::DirectConnectWebSocket::connect(&config, inbound_tx)
            .await?;
    if !prompt.is_empty() {
        ws.send_message(serde_json::json!(prompt)).await?;
    }

    let started_at = std::time::Instant::now();
    let mut final_text = String::new();
    while let Some(inbound) = inbound_rx.recv().await {
        match inbound {
            claude_core::bridge::direct_connect::DirectConnectInbound::Ignored => {}
            claude_core::bridge::direct_connect::DirectConnectInbound::UnsupportedControlRequest {
                request_id,
                subtype,
            } => {
                let error = format!(
                    "Unsupported control request subtype: {}",
                    subtype.as_deref().unwrap_or_default()
                );
                let _ = ws.send_error_response(&request_id, &error).await;
            }
            claude_core::bridge::direct_connect::DirectConnectInbound::PermissionRequest {
                request_id,
                ..
            } => {
                let _ = ws
                    .respond_to_permission_request(
                        &request_id,
                        "deny",
                        None,
                        Some("Permission denied in headless direct-connect mode"),
                    )
                    .await;
            }
            claude_core::bridge::direct_connect::DirectConnectInbound::Message(value) => {
                match output_format {
                    OutputFormat::StreamJson => {
                        println!("{}", serde_json::to_string(&value)?);
                    }
                    OutputFormat::Text | OutputFormat::Json => {}
                }
                match value.get("type").and_then(Value::as_str) {
                    Some("assistant") => {
                        let text = direct_connect_message_text(&value);
                        if output_format == OutputFormat::Text && !text.is_empty() {
                            print!("{text}");
                        }
                        final_text.push_str(&text);
                    }
                    Some("result") => {
                        if output_format == OutputFormat::Json {
                            println!(
                                "{}",
                                serde_json::to_string(&serde_json::json!({
                                    "type": "result",
                                    "subtype": "success",
                                    "is_error": false,
                                    "result": final_text,
                                    "duration_ms": started_at.elapsed().as_millis(),
                                    "session_id": config.session_id,
                                }))?
                            );
                        }
                        if output_format == OutputFormat::Text && !final_text.ends_with('\n') {
                            println!();
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn read_json_arg_or_file(raw: &str, label: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return Ok(value);
    }
    let path = std::path::PathBuf::from(raw);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| anyhow::anyhow!("Error reading {label} {}: {err}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|err| anyhow::anyhow!("Error parsing {label} {}: {err}", path.display()))
}

fn merge_json_values(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(existing) => merge_json_values(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => {
            *base = overlay;
        }
    }
}

fn expand_env_vars(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        if chars.peek() == Some(&'{') {
            chars.next();
            let mut name = String::new();
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
                name.push(next);
            }
            out.push_str(&std::env::var(name).unwrap_or_default());
            continue;
        }
        let mut name = String::new();
        while let Some(&next) = chars.peek() {
            if next == '_' || next.is_ascii_alphanumeric() {
                name.push(next);
                chars.next();
            } else {
                break;
            }
        }
        if name.is_empty() {
            out.push('$');
        } else {
            out.push_str(&std::env::var(name).unwrap_or_default());
        }
    }
    out
}

fn expand_env_vars_in_json(value: &mut Value) {
    match value {
        Value::String(text) => {
            *text = expand_env_vars(text);
        }
        Value::Array(items) => {
            for item in items {
                expand_env_vars_in_json(item);
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                expand_env_vars_in_json(value);
            }
        }
        _ => {}
    }
}

fn load_settings_overlay(
    raw: &Option<String>,
) -> Result<Option<(claude_core::config::settings::Settings, Value)>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = read_json_arg_or_file(raw, "--settings")?;
    let settings = serde_json::from_value::<claude_core::config::settings::Settings>(value.clone())
        .map_err(|err| anyhow::anyhow!("Error parsing --settings: {err}"))?;
    Ok(Some((settings, value)))
}

fn parse_setting_sources_flag(
    raw: &Option<String>,
) -> Result<Vec<claude_core::permissions::SettingSource>> {
    let Some(raw) = raw else {
        return Ok(claude_core::permissions::SettingSource::defaults().to_vec());
    };
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let mut sources = Vec::new();
    for name in raw.split(',').map(str::trim) {
        match name {
            "user" => sources.push(claude_core::permissions::SettingSource::User),
            "project" => sources.push(claude_core::permissions::SettingSource::Project),
            "local" => sources.push(claude_core::permissions::SettingSource::Local),
            _ => {
                return Err(anyhow::anyhow!(
                    "Error processing --setting-sources: Invalid setting source: {name}. Valid options are: user, project, local"
                ));
            }
        }
    }
    Ok(sources)
}

fn load_settings_from_sources(
    project_root: &std::path::Path,
    sources: &[claude_core::permissions::SettingSource],
) -> claude_core::config::settings::Settings {
    let value =
        claude_core::permissions::load_permission_settings_value_for_sources(project_root, sources);
    serde_json::from_value(value).unwrap_or_default()
}

async fn run_startup_migrations(project_root: &std::path::Path) {
    const CURRENT_MIGRATION_VERSION: i64 = 11;

    let mut global = match claude_core::config::global::load_global_config() {
        Ok(global) => global,
        Err(err) => {
            tracing::debug!(error = %err, "failed to load global config for migrations");
            return;
        }
    };
    if global.extra.get("migrationVersion").and_then(Value::as_i64)
        == Some(CURRENT_MIGRATION_VERSION)
    {
        return;
    }

    let user_settings_path = match claude_core::config::paths::user_settings_path() {
        Ok(path) => path,
        Err(err) => {
            tracing::debug!(error = %err, "failed to resolve user settings path for migrations");
            return;
        }
    };
    let mut user_settings =
        claude_core::config::settings::Settings::load_from_file(&user_settings_path);
    let tokens = claude_core::auth::storage::load_tokens()
        .await
        .ok()
        .flatten();
    let subscription_type = tokens
        .as_ref()
        .and_then(|tokens| tokens.subscription_type.as_deref());
    let rate_limit_tier = tokens
        .as_ref()
        .and_then(|tokens| tokens.rate_limit_tier.as_deref());
    let ctx = claude_core::migrations::MigrationContext {
        is_first_party: tokens.is_some() || claude_core::user_type::is_ant(),
        is_pro: subscription_type == Some("pro"),
        is_max: subscription_type == Some("max"),
        is_team_premium: rate_limit_tier == Some("team_premium"),
        is_ant_user: claude_core::user_type::is_ant(),
        transcript_classifier_enabled: claude_core::errors_util::is_env_truthy(
            "TRANSCRIPT_CLASSIFIER",
        ),
        auto_mode_enabled_state: std::env::var("CLAUDE_CODE_AUTO_MODE_ENABLED_STATE").ok(),
    };
    let applied = claude_core::migrations::run_all(&ctx, &mut global, &mut user_settings);

    let project_config_path = project_root.join(".claude.json");
    let local_settings_path = project_root.join(".claude").join("settings.local.json");
    let mut project_migrated = false;
    if let Ok(project_text) = std::fs::read_to_string(&project_config_path) {
        if let Ok(mut project_value) = serde_json::from_str::<Value>(&project_text) {
            if let Some(project_obj) = project_value.as_object_mut() {
                let mut local_settings =
                    claude_core::config::settings::Settings::load_from_file(&local_settings_path);
                if claude_core::migrations::settings_moves::migrate_project_mcp_approval_fields(
                    project_obj,
                    &mut local_settings,
                ) {
                    if let Some(parent) = local_settings_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Ok(text) = serde_json::to_string_pretty(&local_settings) {
                        let _ = std::fs::write(&local_settings_path, text);
                    }
                    if let Ok(text) = serde_json::to_string_pretty(&project_value) {
                        let _ = std::fs::write(&project_config_path, text);
                    }
                    project_migrated = true;
                }
            }
        }
    }

    global.extra.insert(
        "migrationVersion".into(),
        Value::Number(serde_json::Number::from(CURRENT_MIGRATION_VERSION)),
    );
    if !applied.is_empty() || project_migrated {
        if let Some(parent) = user_settings_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&user_settings) {
            let _ = std::fs::write(&user_settings_path, text);
        }
    }
    let _ = claude_core::config::global::save_global_config(|_| global);
}

fn load_dynamic_mcp_configs(
    values: &[String],
) -> Result<(
    std::collections::HashMap<String, claude_core::mcp::types::ScopedMcpServerConfig>,
    Vec<String>,
)> {
    let mut configs = std::collections::HashMap::new();
    let mut order = Vec::new();
    for item in values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        let mut value = read_json_arg_or_file(item, "--mcp-config")?;
        expand_env_vars_in_json(&mut value);
        let servers = value
            .get("mcpServers")
            .and_then(|v| v.as_object())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Error: Invalid MCP configuration:\ncommand line: Missing mcpServers object"
                )
            })?;
        for (name, config_value) in servers {
            let config = claude_core::mcp::types::McpServerConfig::from_value(config_value.clone())
                .map_err(|err| {
                    anyhow::anyhow!(
                        "Error: Invalid MCP configuration:\ncommand line: {name}: {err}"
                    )
                })?;
            if !configs.contains_key(name) {
                order.push(name.clone());
            }
            configs.insert(
                name.clone(),
                claude_core::mcp::types::ScopedMcpServerConfig {
                    config,
                    scope: claude_core::mcp::types::ConfigScope::Dynamic,
                },
            );
        }
    }
    Ok((configs, order))
}

fn resolve_prompt_file_option(
    inline: &Option<String>,
    file: &Option<PathBuf>,
    both_error: &str,
    missing_prefix: &str,
    read_prefix: &str,
) -> Result<Option<String>> {
    if let Some(path) = file {
        if inline.is_some() {
            anyhow::bail!("{both_error}");
        }
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        return match std::fs::read_to_string(&resolved) {
            Ok(content) => Ok(Some(content)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                anyhow::bail!("{missing_prefix}: {}", resolved.display())
            }
            Err(err) => anyhow::bail!("{read_prefix}: {err}"),
        };
    }
    Ok(inline.clone())
}

#[derive(clap::Subcommand)]
pub enum SubCommand {
    /// Authenticate with Anthropic
    Login {
        /// Pre-populate email on the login form
        #[arg(long)]
        email: Option<String>,
        /// Force SSO login method
        #[arg(long)]
        sso: bool,
        /// Use Console auth flow (API key creation)
        #[arg(long)]
        console: bool,
        /// Use Claude.ai auth flow (default)
        #[arg(long)]
        claudeai: bool,
    },
    /// Remove stored credentials
    Logout,
    /// Show current configuration
    Config,
    /// Manage MCP servers
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Start the IDE bridge server
    Server {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Connect your local environment for remote-control sessions via claude.ai/code
    #[command(hide = true, alias = "rc", aliases = ["remote", "sync", "bridge"])]
    RemoteControl {
        /// Name for the session (shown in claude.ai/code)
        #[arg(long)]
        name: Option<String>,
        /// Prefix for auto-generated session names
        #[arg(long = "remote-control-session-name-prefix")]
        remote_control_session_name_prefix: Option<String>,
        /// Permission mode for spawned sessions
        #[arg(long = "permission-mode")]
        permission_mode: Option<String>,
        /// Write debug logs to file
        #[arg(long = "debug-file")]
        debug_file: Option<PathBuf>,
        /// Enable sandboxing for remote-control child sessions
        #[arg(long = "sandbox", hide = true)]
        sandbox: bool,
        /// Disable sandboxing for remote-control child sessions
        #[arg(long = "no-sandbox", hide = true)]
        no_sandbox: bool,
        /// Session timeout in minutes
        #[arg(long = "session-timeout", hide = true)]
        session_timeout: Option<String>,
        /// Enable verbose output
        #[arg(short, long)]
        verbose: bool,
        /// Spawn mode: same-dir, worktree, session
        #[arg(long)]
        spawn: Option<String>,
        /// Max concurrent sessions in worktree or same-dir mode
        #[arg(long)]
        capacity: Option<String>,
        /// Resume a specific remote-control session by ID
        #[arg(long = "session-id", hide = true)]
        session_id: Option<String>,
        /// Resume the last remote-control session in this directory
        #[arg(long = "continue", short = 'c', hide = true)]
        continue_session: bool,
        /// Pre-create a session in the current directory
        #[arg(long = "create-session-in-dir")]
        create_session_in_dir: bool,
        /// Do not pre-create a session in the current directory
        #[arg(long = "no-create-session-in-dir")]
        no_create_session_in_dir: bool,
    },
    /// Connect to a Claude Code server (internal — use cc:// URLs)
    #[command(hide = true)]
    Open {
        cc_url: String,
        #[arg(short = 'p', long = "print")]
        print: Option<Option<String>>,
        #[arg(long = "output-format", value_enum, default_value_t = OutputFormat::Text)]
        output_format: OutputFormat,
    },
}

#[derive(clap::Subcommand)]
pub enum McpCommand {
    /// Manage the XAA (SEP-990) IdP connection
    Xaa {
        #[command(subcommand)]
        command: McpXaaCommand,
    },
}

#[derive(clap::Subcommand)]
pub enum McpXaaCommand {
    /// Configure the IdP connection (one-time setup for all XAA-enabled servers)
    Setup {
        /// IdP issuer URL (OIDC discovery)
        #[arg(long)]
        issuer: String,
        /// Claude Code's client_id at the IdP
        #[arg(long = "client-id")]
        client_id: String,
        /// Read IdP client secret from MCP_XAA_IDP_CLIENT_SECRET env var
        #[arg(long = "client-secret")]
        client_secret: bool,
        /// Fixed loopback callback port
        #[arg(long = "callback-port")]
        callback_port: Option<u32>,
    },
    /// Cache an IdP id_token so XAA-enabled MCP servers authenticate silently
    Login {
        /// Ignore any cached id_token and re-login
        #[arg(long)]
        force: bool,
        /// Write this pre-obtained id_token directly to cache
        #[arg(long = "id-token")]
        id_token: Option<String>,
    },
    /// Show the current IdP connection config
    Show,
    /// Clear the IdP connection config and cached id_token
    Clear,
}

async fn handle_mcp_command(command: &McpCommand) -> Result<()> {
    match command {
        McpCommand::Xaa { command } => handle_mcp_xaa_command(command).await,
    }
}

async fn handle_mcp_xaa_command(command: &McpXaaCommand) -> Result<()> {
    match command {
        McpXaaCommand::Setup {
            issuer,
            client_id,
            client_secret,
            callback_port,
        } => {
            let input =
                claude_core::mcp::xaa_idp::validate_setup_input(issuer, client_id, *callback_port)
                    .map_err(|err| anyhow::anyhow!("Error: {err}"))?;
            let secret = if *client_secret {
                let secret = std::env::var("MCP_XAA_IDP_CLIENT_SECRET").ok();
                if secret.as_deref().unwrap_or_default().is_empty() {
                    anyhow::bail!(
                        "Error: --client-secret requires MCP_XAA_IDP_CLIENT_SECRET env var"
                    );
                }
                secret
            } else {
                None
            };

            let old = load_xaa_idp_user_setting()?;
            save_xaa_idp_user_setting(Some(&claude_core::config::settings::XaaIdpSettings {
                issuer: input.issuer.clone(),
                client_id: input.client_id.clone(),
                callback_port: input.callback_port,
            }))?;

            if let Some(old) = old {
                if claude_core::mcp::xaa_idp::issuer_key(&old.issuer)
                    != claude_core::mcp::xaa_idp::issuer_key(&input.issuer)
                {
                    claude_core::mcp::xaa_idp::clear_idp_id_token(&old.issuer).await?;
                    claude_core::mcp::xaa_idp::clear_idp_client_secret(&old.issuer).await?;
                } else if old.client_id != input.client_id {
                    claude_core::mcp::xaa_idp::clear_idp_id_token(&old.issuer).await?;
                    claude_core::mcp::xaa_idp::clear_idp_client_secret(&old.issuer).await?;
                }
            }

            if let Some(secret) = secret {
                claude_core::mcp::xaa_idp::save_idp_client_secret(&input.issuer, &secret).await?;
            }

            println!("XAA IdP connection configured for {}", input.issuer);
            Ok(())
        }
        McpXaaCommand::Login { force, id_token } => {
            let Some(idp) = load_xaa_idp_user_setting()? else {
                anyhow::bail!("Error: no XAA IdP connection. Run 'claude mcp xaa setup' first.");
            };

            if let Some(id_token) = id_token {
                let expires_at =
                    claude_core::mcp::xaa_idp::save_idp_id_token_from_jwt(&idp.issuer, id_token)
                        .await?;
                println!(
                    "id_token cached for {} (expires {})",
                    idp.issuer,
                    format_epoch_ms_iso(expires_at)
                );
                return Ok(());
            }

            if *force {
                claude_core::mcp::xaa_idp::clear_idp_id_token(&idp.issuer).await?;
            }

            if claude_core::mcp::xaa_idp::get_cached_idp_id_token(&idp.issuer)
                .await?
                .is_some()
            {
                println!(
                    "Already logged in to {} (cached id_token still valid). Use --force to re-login.",
                    idp.issuer
                );
                return Ok(());
            }

            println!("Opening browser for IdP login at {}…", idp.issuer);
            let secret = claude_core::mcp::xaa_idp::get_idp_client_secret(&idp.issuer).await?;
            let on_authorization_url = |url: &str| {
                println!("If the browser did not open, visit:\n  {url}");
            };
            claude_core::mcp::xaa_idp::acquire_idp_id_token(
                claude_core::mcp::xaa_idp::IdpLoginOptions {
                    idp_issuer: &idp.issuer,
                    idp_client_id: &idp.client_id,
                    idp_client_secret: secret.as_deref(),
                    callback_port: idp.callback_port,
                    on_authorization_url: Some(&on_authorization_url),
                    skip_browser_open: false,
                },
            )
            .await
            .map_err(|err| anyhow::anyhow!("IdP login failed: {err}"))?;
            println!("Logged in. MCP servers with --xaa will now authenticate silently.");
            Ok(())
        }
        McpXaaCommand::Show => {
            let Some(idp) = load_xaa_idp_user_setting()? else {
                println!("No XAA IdP connection configured.");
                return Ok(());
            };
            let has_secret = claude_core::mcp::xaa_idp::get_idp_client_secret(&idp.issuer)
                .await?
                .is_some();
            let has_id_token = claude_core::mcp::xaa_idp::get_cached_idp_id_token(&idp.issuer)
                .await?
                .is_some();
            println!("Issuer:        {}", idp.issuer);
            println!("Client ID:     {}", idp.client_id);
            if let Some(port) = idp.callback_port {
                println!("Callback port: {port}");
            }
            println!(
                "Client secret: {}",
                if has_secret {
                    "(stored in keychain)"
                } else {
                    "(not set — PKCE-only)"
                }
            );
            println!(
                "Logged in:     {}",
                if has_id_token {
                    "yes (id_token cached)"
                } else {
                    "no — run 'claude mcp xaa login'"
                }
            );
            Ok(())
        }
        McpXaaCommand::Clear => {
            let idp = load_xaa_idp_user_setting()?;
            save_xaa_idp_user_setting(None)?;
            if let Some(idp) = idp {
                claude_core::mcp::xaa_idp::clear_idp_id_token(&idp.issuer).await?;
                claude_core::mcp::xaa_idp::clear_idp_client_secret(&idp.issuer).await?;
            }
            println!("XAA IdP connection cleared");
            Ok(())
        }
    }
}

fn load_xaa_idp_user_setting() -> Result<Option<claude_core::config::settings::XaaIdpSettings>> {
    let path = claude_core::config::paths::user_settings_path()?;
    let value = read_json_object_file(&path)?;
    let Some(raw) = value.get("xaaIdp") else {
        return Ok(None);
    };
    let idp = serde_json::from_value(raw.clone())
        .map_err(|err| anyhow::anyhow!("Error parsing settings.xaaIdp: {err}"))?;
    Ok(Some(idp))
}

fn save_xaa_idp_user_setting(
    idp: Option<&claude_core::config::settings::XaaIdpSettings>,
) -> Result<()> {
    let path = claude_core::config::paths::user_settings_path()?;
    let mut value = read_json_object_file(&path)?;
    match idp {
        Some(idp) => {
            value.insert("xaaIdp".into(), serde_json::to_value(idp)?);
        }
        None => {
            value.remove("xaaIdp");
        }
    }
    write_json_object_file(&path, &value)
}

fn read_json_object_file(path: &std::path::Path) -> Result<serde_json::Map<String, Value>> {
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let text = std::fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(serde_json::Map::new());
    }
    match serde_json::from_str::<Value>(&text)? {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!(
            "Error: settings file must contain a JSON object: {}",
            path.display()
        ),
    }
}

fn write_json_object_file(
    path: &std::path::Path,
    value: &serde_json::Map<String, Value>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn format_epoch_ms_iso(ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Resolve short model names to full API model IDs.
fn normalize_model_name(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    let has_1m = lower.ends_with("[1m]");
    let base = if has_1m {
        lower.trim_end_matches("[1m]").trim()
    } else {
        lower.as_str()
    };
    let suffix = if has_1m { "[1m]" } else { "" };

    match base {
        "opus" => format!("{}{}", default_opus_model(), suffix),
        "sonnet" => format!("{}{}", default_sonnet_model(), suffix),
        "haiku" => format!("{}{}", default_haiku_model(), suffix),
        "best" => default_opus_model(),
        "opusplan" => format!("{}{}", default_sonnet_model(), suffix),
        _ => trimmed.into(),
    }
}

fn default_opus_model() -> String {
    std::env::var("ANTHROPIC_DEFAULT_OPUS_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "claude-opus-4-7".into())
}

fn default_sonnet_model() -> String {
    std::env::var("ANTHROPIC_DEFAULT_SONNET_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "claude-sonnet-4-6".into())
}

fn default_haiku_model() -> String {
    std::env::var("ANTHROPIC_DEFAULT_HAIKU_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "claude-haiku-4-5".into())
}

async fn default_main_loop_model_setting() -> String {
    if claude_core::user_type::is_ant() {
        return format!("{}[1m]", default_opus_model());
    }

    let tokens = claude_core::auth::storage::load_tokens()
        .await
        .ok()
        .flatten();
    let subscription_type = tokens
        .as_ref()
        .and_then(|tokens| tokens.subscription_type.as_deref());
    let rate_limit_tier = tokens
        .as_ref()
        .and_then(|tokens| tokens.rate_limit_tier.as_deref());

    let is_max = subscription_type == Some("max");
    let is_team_premium =
        subscription_type == Some("team") && rate_limit_tier == Some("default_claude_max_5x");

    if is_max || is_team_premium {
        format!("{}[1m]", default_opus_model())
    } else {
        default_sonnet_model()
    }
}

fn resolve_max_output_tokens(
    model: &str,
    settings: &claude_core::config::settings::Settings,
) -> u64 {
    if let Ok(value) = std::env::var("CLAUDE_CODE_MAX_OUTPUT_TOKENS") {
        if let Ok(parsed) = value.trim().parse::<u64>() {
            return parsed;
        }
    }

    if let Some(max_tokens) = settings.max_tokens {
        return u64::from(max_tokens);
    }

    claude_core::api::client::get_max_output_tokens_for_model(model)
}

fn resolve_effort_for_api(
    cli_effort: Option<&str>,
    settings: &claude_core::config::settings::Settings,
) -> Option<String> {
    let raw = std::env::var("CLAUDE_CODE_EFFORT_LEVEL")
        .ok()
        .or_else(|| cli_effort.map(ToString::to_string))
        .or_else(|| settings.effort_level.clone())?;
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("auto")
        || trimmed.eq_ignore_ascii_case("unset")
    {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn is_env_defined_falsy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            matches!(value.as_str(), "0" | "false" | "no" | "off")
        })
        .unwrap_or(false)
}

fn normalize_name_for_mcp(name: &str) -> String {
    let mut normalized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if name.starts_with("claude.ai ") {
        let mut collapsed = String::with_capacity(normalized.len());
        let mut previous_underscore = false;
        for ch in normalized.chars() {
            if ch == '_' {
                if !previous_underscore {
                    collapsed.push(ch);
                }
                previous_underscore = true;
            } else {
                collapsed.push(ch);
                previous_underscore = false;
            }
        }
        normalized = collapsed.trim_matches('_').to_string();
    }

    normalized
}

fn mcp_server_signature(config: &claude_core::mcp::types::ScopedMcpServerConfig) -> Option<String> {
    use claude_core::mcp::types::McpServerConfig;

    match &config.config {
        McpServerConfig::Stdio(stdio) => {
            let mut command = Vec::with_capacity(1 + stdio.args.len());
            command.push(stdio.command.clone());
            command.extend(stdio.args.clone());
            serde_json::to_string(&command)
                .ok()
                .map(|value| format!("stdio:{}", value))
        }
        McpServerConfig::Sse(sse) | McpServerConfig::SseIde(sse) => {
            Some(format!("url:{}", unwrap_ccr_proxy_url(&sse.url)))
        }
        McpServerConfig::Http(http) => Some(format!("url:{}", unwrap_ccr_proxy_url(&http.url))),
        McpServerConfig::Ws(ws) | McpServerConfig::WsIde(ws) => {
            Some(format!("url:{}", unwrap_ccr_proxy_url(&ws.url)))
        }
    }
}

fn unwrap_ccr_proxy_url(url: &str) -> String {
    const MARKERS: [&str; 2] = ["/v2/session_ingress/shttp/mcp/", "/v2/ccr-sessions/"];
    if !MARKERS.iter().any(|marker| url.contains(marker)) {
        return url.to_string();
    }

    let Some((_, query)) = url.split_once('?') else {
        return url.to_string();
    };
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key == "mcp_url" {
            return value.replace("%3A", ":").replace("%2F", "/");
        }
    }
    url.to_string()
}

fn split_tool_args(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|value| value.split_whitespace())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn filter_allowed_sdk_betas(betas: &[String], is_oauth: bool) -> Vec<String> {
    if betas.is_empty() {
        return Vec::new();
    }
    if is_oauth {
        eprintln!(
            "Warning: Custom betas are only available for API key users. Ignoring provided betas."
        );
        return Vec::new();
    }
    let allowed = [claude_core::constants::betas::CONTEXT_1M];
    let mut result = Vec::new();
    for beta in betas
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        if allowed.contains(&beta) {
            if !result.iter().any(|existing: &String| existing == beta) {
                result.push(beta.to_string());
            }
        } else {
            eprintln!(
                "Warning: Beta header '{beta}' is not allowed. Only the following betas are supported: {}",
                allowed.join(", ")
            );
        }
    }
    result
}

fn filter_registry_by_cli_tools(registry: &mut claude_tools::ToolRegistry, values: &[String]) {
    if values.is_empty() {
        return;
    }

    let requested = split_tool_args(values);
    if requested.iter().any(|value| value == "default") {
        return;
    }

    if requested.is_empty() {
        for name in registry
            .all()
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<_>>()
        {
            registry.remove(&name);
        }
        return;
    }

    let keep = requested
        .into_iter()
        .filter_map(|name| registry.get(&name).map(|tool| tool.name().to_string()))
        .collect::<std::collections::HashSet<_>>();

    for name in registry
        .all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>()
    {
        if !keep.contains(&name) {
            registry.remove(&name);
        }
    }
}

fn base_tool_denials_from_cli_tools(
    base_tools_cli: &[String],
    all_tool_names: &[String],
) -> Vec<String> {
    use claude_core::permissions::{normalize_legacy_tool_name, parse_tool_list_from_cli};

    if base_tools_cli.is_empty() {
        return Vec::new();
    }

    let joined = base_tools_cli.join(" ");
    let trimmed = joined.trim();
    if trimmed.is_empty() || trimmed == "default" {
        return Vec::new();
    }

    let base_tools = parse_tool_list_from_cli(base_tools_cli)
        .into_iter()
        .map(|name| normalize_legacy_tool_name(&name))
        .collect::<std::collections::HashSet<_>>();
    if base_tools.is_empty() {
        return all_tool_names.to_vec();
    }

    all_tool_names
        .iter()
        .filter(|name| !base_tools.contains(*name))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_core::permissions::types::{PermissionBehavior, PermissionRuleSource};
    use claude_core::permissions::{
        parse_permission_rules_from_settings_value,
        permission_additional_directories_from_settings_value,
    };

    #[test]
    fn permission_settings_parse_rules_and_directories() {
        let value = serde_json::json!({
            "permissions": {
                "allow": ["Bash(git status)"],
                "deny": ["Write"],
                "ask": ["Edit"],
                "additionalDirectories": ["/tmp/work", 42, "/tmp/other"]
            }
        });

        let rules =
            parse_permission_rules_from_settings_value(&value, PermissionRuleSource::LocalSettings);
        assert_eq!(rules.len(), 3);
        assert!(rules.iter().any(|rule| {
            rule.source == PermissionRuleSource::LocalSettings
                && rule.rule_behavior == PermissionBehavior::Allow
                && rule.rule_value.to_rule_string() == "Bash(git status)"
        }));
        assert!(rules.iter().any(|rule| {
            rule.rule_behavior == PermissionBehavior::Deny
                && rule.rule_value.to_rule_string() == "Write"
        }));
        assert!(rules.iter().any(|rule| {
            rule.rule_behavior == PermissionBehavior::Ask
                && rule.rule_value.to_rule_string() == "Edit"
        }));

        assert_eq!(
            permission_additional_directories_from_settings_value(&value),
            vec!["/tmp/work".to_string(), "/tmp/other".to_string()]
        );
    }

    #[test]
    fn base_tools_create_denials_for_non_base_tools() {
        let all_tools = vec![
            "Bash".to_string(),
            "Read".to_string(),
            "Write".to_string(),
            "Agent".to_string(),
        ];

        let denials = base_tool_denials_from_cli_tools(&["Bash,Read".to_string()], &all_tools);

        assert_eq!(denials, vec!["Write".to_string(), "Agent".to_string()]);
        assert!(base_tool_denials_from_cli_tools(&["default".to_string()], &all_tools).is_empty());
    }

    #[test]
    fn parses_cli_agents_like_ts_flag_settings() {
        let agents = parse_cli_agents_json(
            r#"{
                "reviewer": {
                    "description": "Review code changes",
                    "tools": ["Read", "Write", "Bash(git status, git diff)"],
                    "disallowedTools": ["Write"],
                    "prompt": "Review the code.",
                    "model": "inherit",
                    "permissionMode": "acceptEdits",
                    "background": true,
                    "isolation": "worktree",
                    "initialPrompt": "/status"
                }
            }"#,
        );

        assert_eq!(agents.len(), 1);
        let agent = &agents[0];
        assert_eq!(agent.agent_type, "reviewer");
        assert_eq!(agent.when_to_use, "Review code changes");
        assert_eq!(
            agent.source,
            claude_tools::agent_tool::AgentSource::FlagSettings
        );
        assert_eq!(
            agent.tools.as_ref().unwrap(),
            &vec![
                "Read".to_string(),
                "Write".to_string(),
                "Bash(git status, git diff)".to_string()
            ]
        );
        assert_eq!(
            agent.disallowed_tools.as_ref().unwrap(),
            &vec!["Write".to_string()]
        );
        assert_eq!(agent.model.as_deref(), Some("inherit"));
        assert_eq!(agent.permission_mode.as_deref(), Some("acceptEdits"));
        assert_eq!(agent.background, Some(true));
        assert_eq!(agent.isolation.as_deref(), Some("worktree"));
        assert_eq!(agent.initial_prompt.as_deref(), Some("/status"));
    }

    #[test]
    fn cli_agent_wildcard_tools_mean_all_tools() {
        let agents = parse_cli_agents_json(
            r#"{
                "runner": {
                    "description": "Run everything",
                    "tools": ["*"],
                    "prompt": "Run all checks."
                }
            }"#,
        );

        assert_eq!(agents.len(), 1);
        assert!(agents[0].tools.is_none());
    }

    #[test]
    fn parses_markdown_agent_frontmatter_like_ts() {
        let mut frontmatter = claude_core::frontmatter::Frontmatter::new();
        frontmatter.insert(
            "name".into(),
            claude_core::frontmatter::FrontmatterValue::String("reviewer".into()),
        );
        frontmatter.insert(
            "description".into(),
            claude_core::frontmatter::FrontmatterValue::String("Review\\nchanges".into()),
        );
        frontmatter.insert(
            "tools".into(),
            claude_core::frontmatter::FrontmatterValue::List(vec![
                claude_core::frontmatter::FrontmatterValue::String("Read".into()),
                claude_core::frontmatter::FrontmatterValue::String("Bash(git status)".into()),
            ]),
        );
        frontmatter.insert(
            "background".into(),
            claude_core::frontmatter::FrontmatterValue::Bool(true),
        );
        frontmatter.insert(
            "permissionMode".into(),
            claude_core::frontmatter::FrontmatterValue::String("acceptEdits".into()),
        );
        frontmatter.insert(
            "initialPrompt".into(),
            claude_core::frontmatter::FrontmatterValue::String("/check".into()),
        );

        let file = claude_core::markdown_config_loader::MarkdownFile {
            file_path: std::path::PathBuf::from("/tmp/reviewer.md"),
            base_dir: std::path::PathBuf::from("/tmp"),
            frontmatter,
            content: "System prompt body\n".into(),
            source: claude_core::markdown_config_loader::MarkdownSource::Project,
        };

        let agent = parse_markdown_agent(&file).expect("agent should parse");
        assert_eq!(agent.agent_type, "reviewer");
        assert_eq!(agent.when_to_use, "Review\nchanges");
        assert_eq!(
            agent.source,
            claude_tools::agent_tool::AgentSource::ProjectSettings
        );
        assert_eq!(
            agent.tools.as_ref().unwrap(),
            &vec!["Read".to_string(), "Bash(git status)".to_string()]
        );
        assert_eq!(agent.system_prompt.as_deref(), Some("System prompt body"));
        assert_eq!(agent.background, Some(true));
        assert_eq!(agent.permission_mode.as_deref(), Some("acceptEdits"));
        assert_eq!(agent.initial_prompt.as_deref(), Some("/check"));
    }

    #[test]
    fn stream_json_stdin_seed_reads_user_and_initialize_messages() {
        let input = concat!(
            "{\"type\":\"control_request\",\"request\":{\"subtype\":\"initialize\",\"systemPrompt\":\"sys\",\"appendSystemPrompt\":\"append\",\"jsonSchema\":{\"type\":\"object\"}}}\n",
            "{\"type\":\"user\",\"session_id\":\"\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"parent_tool_use_id\":null}\n",
            "{\"type\":\"user\",\"session_id\":\"\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"there\"}]},\"parent_tool_use_id\":null}\n",
        );

        let seed = parse_stream_json_stdin(input).unwrap();

        assert_eq!(seed.system_prompt.as_deref(), Some("sys"));
        assert_eq!(seed.append_system_prompt.as_deref(), Some("append"));
        assert_eq!(seed.json_schema.as_deref(), Some("{\"type\":\"object\"}"));
        assert_eq!(seed.prompt.as_deref(), Some("hi\nthere"));
    }

    #[test]
    fn dynamic_mcp_config_parses_json_and_expands_env() {
        std::env::set_var("CLAUDE_RS_TEST_MCP_TOKEN", "secret");
        let raw = r#"{
            "mcpServers": {
                "docs": {
                    "type": "http",
                    "url": "https://example.com/mcp",
                    "headers": {
                        "Authorization": "Bearer $CLAUDE_RS_TEST_MCP_TOKEN"
                    }
                }
            }
        }"#;

        let (configs, order) = load_dynamic_mcp_configs(&[raw.to_string()]).unwrap();

        assert_eq!(order, vec!["docs".to_string()]);
        let config = configs.get("docs").unwrap();
        assert_eq!(config.scope, claude_core::mcp::types::ConfigScope::Dynamic);
        match &config.config {
            claude_core::mcp::types::McpServerConfig::Http(http) => {
                assert_eq!(http.url, "https://example.com/mcp");
                assert_eq!(
                    http.headers
                        .as_ref()
                        .unwrap()
                        .get("Authorization")
                        .map(String::as_str),
                    Some("Bearer secret")
                );
            }
            other => panic!("expected http config, got {other:?}"),
        }
        std::env::remove_var("CLAUDE_RS_TEST_MCP_TOKEN");
    }

    #[test]
    fn setting_sources_flag_matches_ts_names() {
        assert_eq!(
            parse_setting_sources_flag(&None).unwrap(),
            vec![
                claude_core::permissions::SettingSource::User,
                claude_core::permissions::SettingSource::Project,
                claude_core::permissions::SettingSource::Local,
            ]
        );
        assert_eq!(
            parse_setting_sources_flag(&Some("project,local".to_string())).unwrap(),
            vec![
                claude_core::permissions::SettingSource::Project,
                claude_core::permissions::SettingSource::Local,
            ]
        );
        assert!(parse_setting_sources_flag(&Some("".to_string()))
            .unwrap()
            .is_empty());
        assert!(parse_setting_sources_flag(&Some("workspace".to_string())).is_err());
    }

    #[test]
    fn max_budget_usd_cli_and_stream_event_match_ts_shape() {
        let cli =
            Cli::try_parse_from(["claude-rs", "-p", "--max-budget-usd", "0.25", "hi"]).unwrap();
        assert_eq!(cli.max_budget_usd, Some(0.25));

        let usage = claude_core::types::usage::Usage {
            input_tokens: 10,
            output_tokens: 2,
            cache_creation_input_tokens: Some(0),
            cache_read_input_tokens: Some(0),
        };
        let event = stream_json_max_budget_usd_event_with_meta(
            0.25,
            "session-1",
            StreamJsonResultMeta {
                duration_ms: 1,
                num_turns: 1,
                stop_reason: "end_turn",
                total_usage: Some(&usage),
                latest_usage: Some(&usage),
                model_display: "claude-sonnet-4-6",
                max_tokens: 64_000,
                context_window: 200_000,
                total_cost_usd: 0.25,
            },
        );
        assert_eq!(event["subtype"], serde_json::json!("error_max_budget_usd"));
        assert_eq!(event["is_error"], serde_json::json!(true));
        assert_eq!(
            event["errors"],
            serde_json::json!(["Reached maximum budget ($0.25)"])
        );
    }

    #[test]
    fn fallback_model_cli_flag_is_accepted() {
        let cli = Cli::try_parse_from([
            "claude-rs",
            "-p",
            "--model",
            "opus",
            "--fallback-model",
            "sonnet",
            "hi",
        ])
        .unwrap();
        assert_eq!(cli.model.as_deref(), Some("opus"));
        assert_eq!(cli.fallback_model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn effort_resolution_matches_ts_precedence_and_auto_escape() {
        std::env::remove_var("CLAUDE_CODE_EFFORT_LEVEL");
        let settings = claude_core::config::settings::Settings {
            effort_level: Some("medium".to_string()),
            ..Default::default()
        };
        assert_eq!(
            resolve_effort_for_api(Some("high"), &settings).as_deref(),
            Some("high")
        );

        std::env::set_var("CLAUDE_CODE_EFFORT_LEVEL", "low");
        assert_eq!(
            resolve_effort_for_api(Some("high"), &settings).as_deref(),
            Some("low")
        );

        std::env::set_var("CLAUDE_CODE_EFFORT_LEVEL", "auto");
        assert!(resolve_effort_for_api(Some("high"), &settings).is_none());
        std::env::remove_var("CLAUDE_CODE_EFFORT_LEVEL");

        let cli = Cli::try_parse_from(["claude-rs", "--effort", "auto", "-p", "hi"]).unwrap();
        assert_eq!(cli.effort.as_deref(), Some("auto"));
    }

    #[test]
    fn bare_cli_flag_is_accepted() {
        let cli = Cli::try_parse_from(["claude-rs", "--bare", "-p", "hi"]).unwrap();
        assert!(cli.bare);
    }

    #[test]
    fn sdk_betas_follow_ts_allowlist() {
        let betas = filter_allowed_sdk_betas(
            &[
                claude_core::constants::betas::CONTEXT_1M.to_string(),
                "not-allowed".to_string(),
                claude_core::constants::betas::CONTEXT_1M.to_string(),
            ],
            false,
        );
        assert_eq!(
            betas,
            vec![claude_core::constants::betas::CONTEXT_1M.to_string()]
        );
        assert!(filter_allowed_sdk_betas(
            &[claude_core::constants::betas::CONTEXT_1M.to_string()],
            true
        )
        .is_empty());
    }

    #[test]
    fn stream_json_rate_limit_event_matches_sdk_shape() {
        let event = stream_json_rate_limit_event(
            "session-1",
            &claude_core::rate_limits::ClaudeAiLimits::default(),
        );
        assert_eq!(event["type"], "rate_limit_event");
        assert_eq!(event["session_id"], "session-1");
        assert_eq!(event["rate_limit_info"]["status"], "allowed");
        assert_eq!(event["rate_limit_info"]["isUsingOverage"], false);
        assert!(event["uuid"].as_str().is_some_and(|uuid| !uuid.is_empty()));
    }

    #[test]
    fn shadow_tool_descriptions_refresh_knowledge_date() {
        let description =
            "Search docs. Knowledge up-to-date as at 28 April 2026. Combine results.".to_string();
        let refreshed = refresh_shadow_tool_description_for_date(
            description,
            chrono::NaiveDate::from_ymd_opt(2026, 4, 29).unwrap(),
        );
        assert_eq!(
            refreshed,
            "Search docs. Knowledge up-to-date as at 29 April 2026. Combine results."
        );
    }

    #[test]
    fn stream_json_hook_execution_events_match_sdk_shape() {
        let event = hook_execution_event_to_stream_json(
            claude_core::hooks::HookExecutionEvent::Response {
                hook_id: "hook-1".into(),
                hook_name: "PreToolUse:Bash".into(),
                hook_event: "PreToolUse".into(),
                output: "out".into(),
                stdout: "out".into(),
                stderr: String::new(),
                exit_code: Some(0),
                outcome: "success".into(),
            },
            "session-1",
        );

        assert_eq!(event["type"], "system");
        assert_eq!(event["subtype"], "hook_response");
        assert_eq!(event["hook_id"], "hook-1");
        assert_eq!(event["hook_name"], "PreToolUse:Bash");
        assert_eq!(event["hook_event"], "PreToolUse");
        assert_eq!(event["output"], "out");
        assert_eq!(event["stdout"], "out");
        assert_eq!(event["stderr"], "");
        assert_eq!(event["exit_code"], 0);
        assert_eq!(event["outcome"], "success");
        assert_eq!(event["session_id"], "session-1");
        assert!(event["uuid"].as_str().is_some_and(|uuid| !uuid.is_empty()));
    }

    #[test]
    fn stream_json_partial_messages_gate_raw_sse_events() {
        let raw = claude_core::types::events::StreamEvent::RawSse {
            event: serde_json::json!({
                "type": "message_delta",
                "delta": {"stop_reason": "tool_use"},
                "usage": {"output_tokens": 42},
            }),
        };

        assert!(stream_event_to_stream_json_events(&raw, "session-1", false).is_empty());

        let events = stream_event_to_stream_json_events(&raw, "session-1", true);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "stream_event");
        assert_eq!(events[0]["session_id"], "session-1");
        assert_eq!(events[0]["parent_tool_use_id"], serde_json::Value::Null);
        assert_eq!(events[0]["event"]["type"], "message_delta");
    }

    #[test]
    fn stream_json_usage_matches_sdk_shape() {
        let usage = claude_core::types::usage::Usage {
            input_tokens: 6,
            output_tokens: 8,
            cache_creation_input_tokens: Some(15445),
            cache_read_input_tokens: Some(55640),
        };

        let assistant_usage = stream_json_usage_value(&usage, "not_available", false);
        assert_eq!(assistant_usage["service_tier"], "standard");
        assert_eq!(assistant_usage["inference_geo"], "not_available");
        assert_eq!(
            assistant_usage["cache_creation"]["ephemeral_5m_input_tokens"],
            0
        );
        assert_eq!(
            assistant_usage["cache_creation"]["ephemeral_1h_input_tokens"],
            15445
        );
        assert!(assistant_usage.get("speed").is_none());

        let result_usage = stream_json_usage_value(&usage, "", true);
        assert_eq!(result_usage["speed"], "standard");
        assert_eq!(result_usage["inference_geo"], "");
    }

    #[test]
    fn model_tool_result_matches_ts_write_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "Write",
                &serde_json::json!({
                    "type": "create",
                    "filePath": "/tmp/example.txt",
                    "content": "hello",
                    "originalFile": null
                })
            ),
            "File created successfully at: /tmp/example.txt"
        );
        assert_eq!(
            format_tool_result_for_model(
                "Write",
                &serde_json::json!({
                    "type": "update",
                    "filePath": "/tmp/example.txt",
                    "content": "hello",
                    "originalFile": "old"
                })
            ),
            "The file /tmp/example.txt has been updated successfully."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_read_line_number_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "Read",
                &serde_json::json!({
                    "type": "text",
                    "file": {
                        "filePath": "/tmp/example.txt",
                        "content": "alpha\nbeta\n",
                        "startLine": 7,
                        "numLines": 3,
                        "totalLines": 3
                    }
                })
            ),
            "7\talpha\n8\tbeta\n9\t"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_mcp_resource_mapping() {
        assert_eq!(
            format_tool_result_for_model("ListMcpResourcesTool", &serde_json::json!([])),
            "No resources found. MCP servers may still provide tools even if they have no resources."
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&format_tool_result_for_model(
                "ListMcpResourcesTool",
                &serde_json::json!([
                    {
                        "uri": "file:///tmp/a.txt",
                        "name": "a.txt",
                        "mimeType": "text/plain",
                        "server": "example"
                    }
                ])
            ))
            .unwrap(),
            serde_json::json!([{
                "mimeType": "text/plain",
                "name": "a.txt",
                "server": "example",
                "uri": "file:///tmp/a.txt"
            }])
        );
    }

    #[test]
    fn model_tool_result_matches_ts_edit_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "Edit",
                &serde_json::json!({
                    "filePath": "/tmp/example.txt",
                    "replaceAll": false,
                    "userModified": false
                })
            ),
            "The file /tmp/example.txt has been updated successfully."
        );
        assert_eq!(
            format_tool_result_for_model(
                "Edit",
                &serde_json::json!({
                    "filePath": "/tmp/example.txt",
                    "replaceAll": true,
                    "userModified": false
                })
            ),
            "The file /tmp/example.txt has been updated. All occurrences were successfully replaced."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_todo_write_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "TodoWrite",
                &serde_json::json!({
                    "oldTodos": [],
                    "newTodos": [],
                    "verificationNudgeNeeded": false
                })
            ),
            "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_plan_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "EnterPlanMode",
                &serde_json::json!({
                    "message": "Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach."
                })
            ),
            "Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.\n\nIn plan mode, you should:\n1. Thoroughly explore the codebase to understand existing patterns\n2. Identify similar features and architectural approaches\n3. Consider multiple approaches and their trade-offs\n4. Use AskUserQuestion if you need to clarify the approach\n5. Design a concrete implementation strategy\n6. When ready, use ExitPlanMode to present your plan for approval\n\nRemember: DO NOT write or edit any files yet. This is a read-only exploration and planning phase."
        );
        assert_eq!(
            format_tool_result_for_model(
                "ExitPlanMode",
                &serde_json::json!({
                    "plan": "",
                    "filePath": "/tmp/plan.md",
                    "hasTaskTool": true,
                    "planWasEdited": false
                })
            ),
            "User has approved exiting plan mode. You can now proceed."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_web_and_config_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "WebFetch",
                &serde_json::json!({
                    "result": "Fetched page summary"
                })
            ),
            "Fetched page summary"
        );
        assert_eq!(
            format_tool_result_for_model(
                "Config",
                &serde_json::json!({
                    "action": "get",
                    "key": "theme",
                    "value": "dark"
                })
            ),
            "theme = \"dark\""
        );
        assert_eq!(
            format_tool_result_for_model(
                "Config",
                &serde_json::json!({
                    "action": "set",
                    "key": "theme",
                    "value": "dark"
                })
            ),
            "Set theme to \"dark\""
        );
    }

    #[test]
    fn model_tool_result_matches_ts_notebook_edit_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "NotebookEdit",
                &serde_json::json!({
                    "cell_id": "cell-1",
                    "edit_mode": "replace",
                    "new_source": "print(1)",
                    "error": ""
                })
            ),
            "Updated cell cell-1 with print(1)"
        );
        assert_eq!(
            format_tool_result_for_model(
                "NotebookEdit",
                &serde_json::json!({
                    "cell_id": "abc123",
                    "edit_mode": "insert",
                    "new_source": "# title",
                    "error": ""
                })
            ),
            "Inserted cell abc123 with # title"
        );
        assert_eq!(
            format_tool_result_for_model(
                "NotebookEdit",
                &serde_json::json!({
                    "cell_id": "abc123",
                    "edit_mode": "delete",
                    "new_source": "",
                    "error": ""
                })
            ),
            "Deleted cell abc123"
        );
    }

    #[test]
    fn model_tool_result_uses_ts_message_field_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "EnterWorktree",
                &serde_json::json!({
                    "worktreePath": "/tmp/wt",
                    "worktreeBranch": "worktree/demo",
                    "message": "Created worktree at /tmp/wt. The session is now working in the worktree."
                })
            ),
            "Created worktree at /tmp/wt. The session is now working in the worktree."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_skill_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "Skill",
                &serde_json::json!({
                    "skill": "plugin:example",
                    "content": "large skill body",
                    "message": "Skill 'plugin:example' loaded successfully."
                })
            ),
            "Launching skill: plugin:example"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_task_output_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "TaskOutput",
                &serde_json::json!({
                    "taskId": "task-1",
                    "status": "completed",
                    "output": "hello\n",
                    "pid": 123
                })
            ),
            "<retrieval_status>found</retrieval_status>\n\n<task_id>task-1</task_id>\n\n<task_type>background</task_type>\n\n<status>completed</status>\n\n<output>\nhello\n</output>"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_task_family_mapping() {
        let task = serde_json::json!({
            "id": "7",
            "subject": "Wire task formatting",
            "description": "Match TS",
            "status": "pending",
            "blockedBy": [],
            "blocks": []
        });
        assert_eq!(
            format_tool_result_for_model("TaskCreate", &task),
            "Task #7 created successfully: Wire task formatting"
        );
        assert_eq!(
            format_tool_result_for_model("TaskGet", &task),
            "Task #7: Wire task formatting\nStatus: pending\nDescription: Match TS"
        );
        assert_eq!(
            format_tool_result_for_model("TaskList", &serde_json::json!({"tasks": [task]})),
            "#7 [pending] Wire task formatting"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_tool_search_mapping() {
        assert_eq!(
            format_tool_result_content_for_model(
                "ToolSearch",
                &serde_json::json!({"matches": ["Read", "Grep"]})
            ),
            serde_json::json!([
                {"type": "tool_reference", "tool_name": "Read"},
                {"type": "tool_reference", "tool_name": "Grep"}
            ])
        );
        assert_eq!(
            format_tool_result_content_for_model("ToolSearch", &serde_json::json!({"matches": []})),
            serde_json::json!("No matching deferred tools found")
        );
    }

    #[test]
    fn model_tool_result_matches_ts_ask_user_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "AskUserQuestion",
                &serde_json::json!({
                    "answers": { "Which approach?": "Use Rust" },
                    "annotations": {
                        "Which approach?": {
                            "preview": "cargo test",
                            "notes": "matches the port"
                        }
                    }
                })
            ),
            "User has answered your questions: \"Which approach?\"=\"Use Rust\" selected preview:\ncargo test user notes: matches the port. You can now continue with the user's answers in mind."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_agent_completed_mapping() {
        assert_eq!(
            format_tool_result_content_for_model(
                "Agent",
                &serde_json::json!({
                    "status": "completed",
                    "agentId": "agent-1",
                    "agentType": "general-purpose",
                    "content": [{ "type": "text", "text": "Done." }],
                    "totalTokens": 42,
                    "totalToolUseCount": 3,
                    "totalDurationMs": 99
                })
            ),
            serde_json::json!([
                { "type": "text", "text": "Done." },
                { "type": "text", "text": "agentId: agent-1 (use SendMessage with to: 'agent-1' to continue this agent)\n<usage>total_tokens: 42\ntool_uses: 3\nduration_ms: 99</usage>" }
            ])
        );
        assert_eq!(
            format_tool_result_content_for_model(
                "Agent",
                &serde_json::json!({
                    "status": "completed",
                    "agentId": "agent-1",
                    "agentType": "Explore",
                    "content": [{ "type": "text", "text": "Report." }],
                    "totalTokens": 42,
                    "totalToolUseCount": 3,
                    "totalDurationMs": 99
                })
            ),
            serde_json::json!([{ "type": "text", "text": "Report." }])
        );
    }

    #[test]
    fn model_tool_result_matches_ts_lsp_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "LSP",
                &serde_json::json!({
                    "operation": "hover",
                    "result": "fn main()",
                    "filePath": "src/main.rs"
                })
            ),
            "fn main()"
        );
    }

    #[test]
    fn model_tool_result_matches_ts_background_tool_mappings() {
        assert_eq!(
            format_tool_result_for_model(
                "CronCreate",
                &serde_json::json!({
                    "id": "job-1",
                    "humanSchedule": "every hour",
                    "recurring": true,
                    "durable": true
                })
            ),
            "Scheduled recurring job job-1 (every hour). Persisted to .claude/scheduled_tasks.json. Auto-expires after 7 days. Use CronDelete to cancel sooner."
        );
        assert_eq!(
            format_tool_result_for_model("CronDelete", &serde_json::json!({"id": "job-1"})),
            "Cancelled job job-1."
        );
        assert_eq!(
            format_tool_result_for_model(
                "CronList",
                &serde_json::json!({
                    "jobs": [{
                        "id": "job-1",
                        "humanSchedule": "every hour",
                        "recurring": true,
                        "prompt": "check status"
                    }]
                })
            ),
            "job-1 — every hour (recurring): check status"
        );
        assert_eq!(
            format_tool_result_for_model(
                "RemoteTrigger",
                &serde_json::json!({"status": 202, "json": "{\"ok\":true}"})
            ),
            "HTTP 202\n{\"ok\":true}"
        );
        assert_eq!(
            format_tool_result_for_model(
                "Monitor",
                &serde_json::json!({
                    "backgroundTaskId": "monitor-1",
                    "outputPath": "/tmp/monitor-1.output"
                })
            ),
            "Monitor running in background with ID: monitor-1. Output is being written to: /tmp/monitor-1.output"
        );
        assert_eq!(
            format_tool_result_for_model(
                "Bash",
                &serde_json::json!({
                    "stdout": "",
                    "stderr": "",
                    "backgroundTaskId": "bg-1",
                    "outputPath": "/tmp/bg-1.output"
                })
            ),
            "Command running in background with ID: bg-1. Output is being written to: /tmp/bg-1.output"
        );
        assert_eq!(
            format_tool_result_for_model(
                "Bash",
                &serde_json::json!({
                    "stdout": "started\n",
                    "stderr": "",
                    "backgroundTaskId": "bg-2",
                    "outputPath": "/tmp/bg-2.output",
                    "backgroundedByUser": true
                })
            ),
            "started\nCommand was manually backgrounded by user with ID: bg-2. Output is being written to: /tmp/bg-2.output"
        );
        assert_eq!(
            format_tool_result_for_model(
                "Bash",
                &serde_json::json!({
                    "stdout": "",
                    "stderr": "",
                    "backgroundTaskId": "bg-3",
                    "outputPath": "/tmp/bg-3.output",
                    "assistantAutoBackgrounded": true
                })
            ),
            "Command exceeded the assistant-mode blocking budget (15s) and was moved to the background with ID: bg-3. It is still running — you will be notified when it completes. Output is being written to: /tmp/bg-3.output. In assistant mode, delegate long-running work to a subagent or use run_in_background to keep this conversation responsive."
        );
    }

    #[test]
    fn model_tool_result_matches_ts_empty_result_mapping() {
        assert_eq!(
            format_tool_result_for_model(
                "Bash",
                &serde_json::json!({
                    "stdout": "",
                    "stderr": ""
                })
            ),
            "(Bash completed with no output)"
        );
        assert_eq!(
            ensure_non_empty_tool_result_content(
                "Example",
                serde_json::json!([
                    { "type": "text", "text": "   " },
                    { "type": "text", "text": "" }
                ])
            ),
            serde_json::json!("(Example completed with no output)")
        );
        assert_eq!(
            ensure_non_empty_tool_result_content(
                "Example",
                serde_json::json!([{ "type": "image", "source": { "type": "base64" } }])
            ),
            serde_json::json!([{ "type": "image", "source": { "type": "base64" } }])
        );
    }
}

fn enabled_plugin_roots(
    project_root: &std::path::Path,
) -> Vec<(String, String, std::path::PathBuf)> {
    let Ok(claude_dir) = claude_core::config::paths::claude_dir() else {
        return Vec::new();
    };

    let mut roots = Vec::new();
    for plugin_id in claude_core::plugins::skill::enabled_plugins_for_project(project_root) {
        let Some((name, source)) = plugin_id.split_once('@') else {
            continue;
        };
        let cache_root = claude_dir
            .join("plugins")
            .join("cache")
            .join(source)
            .join(name);
        let Ok(entries) = std::fs::read_dir(cache_root) else {
            continue;
        };
        let mut versions: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        versions.sort();
        if let Some(root) = versions.pop() {
            roots.push((name.to_string(), source.to_string(), root));
        }
    }
    roots
}

fn load_enabled_plugin_mcp_servers(
    project_root: &std::path::Path,
) -> (
    std::collections::HashMap<String, claude_core::config::settings::McpServerSettingsEntry>,
    Vec<String>,
) {
    let mut servers = std::collections::HashMap::new();
    let mut order = Vec::new();
    for (plugin_name, _, root) in enabled_plugin_roots(project_root) {
        let mcp_path = root.join(".mcp.json");
        let Ok(text) = std::fs::read_to_string(mcp_path) else {
            continue;
        };
        let Ok(entries) = serde_json::from_str::<
            std::collections::HashMap<
                String,
                claude_core::config::settings::McpServerSettingsEntry,
            >,
        >(&text) else {
            continue;
        };
        for (server_name, entry) in entries {
            let name = format!("plugin:{}:{}", plugin_name, server_name);
            order.push(name.clone());
            servers.insert(name, entry);
        }
    }
    (servers, order)
}

fn stream_json_plugin_entries(project_root: &std::path::Path) -> Vec<serde_json::Value> {
    enabled_plugin_roots(project_root)
        .into_iter()
        .map(|(name, source, root)| {
            serde_json::json!({
                "name": name,
                "path": root.display().to_string(),
                "source": format!("{name}@{source}"),
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliAgentJson {
    description: String,
    tools: Option<Value>,
    disallowed_tools: Option<Value>,
    prompt: String,
    model: Option<String>,
    permission_mode: Option<String>,
    background: Option<bool>,
    isolation: Option<String>,
    initial_prompt: Option<String>,
    required_mcp_servers: Option<Vec<String>>,
}

fn parse_agent_tool_list(value: Option<&Value>) -> Option<Vec<String>> {
    let Some(value) = value else {
        return None;
    };
    if value.is_null() {
        return None;
    }
    if value.as_str() == Some("") {
        return Some(Vec::new());
    }

    let raw = match value {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };

    if raw.is_empty() {
        return Some(Vec::new());
    }
    let parsed = claude_core::permissions::parse_tool_list_from_cli(&raw);
    if parsed.iter().any(|tool| tool == "*") {
        None
    } else {
        Some(parsed)
    }
}

fn parse_cli_agents_json(raw: &str) -> Vec<claude_tools::agent_tool::RuntimeAgentDefinition> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Value::Object(map) = value else {
        return Vec::new();
    };

    map.into_iter()
        .filter_map(|(name, definition)| {
            let parsed = serde_json::from_value::<CliAgentJson>(definition).ok()?;
            let description = parsed.description.trim();
            let prompt = parsed.prompt.trim();
            if description.is_empty() || prompt.is_empty() {
                return None;
            }
            let model = parsed.model.and_then(|model| {
                let trimmed = model.trim();
                if trimmed.is_empty() {
                    None
                } else if trimmed.eq_ignore_ascii_case("inherit") {
                    Some("inherit".to_string())
                } else {
                    Some(trimmed.to_string())
                }
            });
            let mut agent = claude_tools::agent_tool::RuntimeAgentDefinition::flag_agent(
                name,
                description.to_string(),
                parse_agent_tool_list(parsed.tools.as_ref()),
                parse_agent_tool_list(parsed.disallowed_tools.as_ref()),
                prompt.to_string(),
                model,
            );
            agent.permission_mode = parsed.permission_mode;
            agent.background = parsed.background;
            agent.isolation = parsed.isolation;
            agent.initial_prompt = parsed.initial_prompt;
            agent.required_mcp_servers = parsed.required_mcp_servers;
            Some(agent)
        })
        .collect()
}

fn parse_agent_frontmatter_tools(
    value: Option<&claude_core::frontmatter::FrontmatterValue>,
) -> Option<Vec<String>> {
    let parsed = claude_core::markdown_config_loader::parse_tool_list(value)?;
    if parsed.iter().any(|tool| tool == "*") {
        None
    } else {
        Some(parsed)
    }
}

fn frontmatter_string<'a>(
    frontmatter: &'a claude_core::frontmatter::Frontmatter,
    key: &str,
) -> Option<&'a str> {
    frontmatter.get(key).and_then(|value| value.as_str())
}

fn frontmatter_bool(
    frontmatter: &claude_core::frontmatter::Frontmatter,
    key: &str,
) -> Option<bool> {
    frontmatter.get(key).and_then(|value| {
        value.as_bool().or_else(|| match value.as_str()? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
    })
}

fn frontmatter_string_list(
    frontmatter: &claude_core::frontmatter::Frontmatter,
    key: &str,
) -> Option<Vec<String>> {
    use claude_core::frontmatter::FrontmatterValue;
    match frontmatter.get(key)? {
        FrontmatterValue::String(value) => {
            let items = value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            (!items.is_empty()).then_some(items)
        }
        FrontmatterValue::List(items) => {
            let values = items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(values)
        }
        _ => None,
    }
}

fn parse_markdown_agent(
    file: &claude_core::markdown_config_loader::MarkdownFile,
) -> Option<claude_tools::agent_tool::RuntimeAgentDefinition> {
    let agent_type = frontmatter_string(&file.frontmatter, "name")?;
    let when_to_use = frontmatter_string(&file.frontmatter, "description")?.replace("\\n", "\n");
    if agent_type.is_empty() || when_to_use.is_empty() {
        return None;
    }

    let model = frontmatter_string(&file.frontmatter, "model").and_then(|model| {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.eq_ignore_ascii_case("inherit") {
            Some("inherit".to_string())
        } else {
            Some(trimmed.to_string())
        }
    });
    let source = match file.source {
        claude_core::markdown_config_loader::MarkdownSource::User => {
            claude_tools::agent_tool::AgentSource::UserSettings
        }
        claude_core::markdown_config_loader::MarkdownSource::Project => {
            claude_tools::agent_tool::AgentSource::ProjectSettings
        }
    };

    Some(claude_tools::agent_tool::RuntimeAgentDefinition {
        agent_type: agent_type.to_string(),
        when_to_use,
        source,
        tools: parse_agent_frontmatter_tools(file.frontmatter.get("tools")),
        disallowed_tools: if file.frontmatter.contains_key("disallowedTools") {
            parse_agent_frontmatter_tools(file.frontmatter.get("disallowedTools"))
        } else {
            None
        },
        system_prompt: Some(file.content.trim().to_string()),
        model,
        permission_mode: frontmatter_string(&file.frontmatter, "permissionMode")
            .map(str::to_string),
        background: frontmatter_bool(&file.frontmatter, "background"),
        isolation: frontmatter_string(&file.frontmatter, "isolation").map(str::to_string),
        initial_prompt: frontmatter_string(&file.frontmatter, "initialPrompt").map(str::to_string),
        required_mcp_servers: frontmatter_string_list(&file.frontmatter, "requiredMcpServers"),
    })
}

fn load_markdown_agents(
    cwd: &std::path::Path,
) -> Vec<claude_tools::agent_tool::RuntimeAgentDefinition> {
    if claude_core::errors_util::is_env_truthy("CLAUDE_CODE_SIMPLE") {
        return Vec::new();
    }
    claude_core::markdown_config_loader::load_markdown_files_for_subdir("agents", cwd)
        .iter()
        .filter_map(parse_markdown_agent)
        .collect()
}

fn sanitize_plugin_id(plugin_id: &str) -> String {
    plugin_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn plugin_data_dir(source_name: &str) -> Option<std::path::PathBuf> {
    let claude_dir = claude_core::config::paths::claude_dir().ok()?;
    let dir = claude_dir
        .join("plugins")
        .join("data")
        .join(sanitize_plugin_id(source_name));
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

fn substitute_plugin_variables(
    system_prompt: &str,
    root: &std::path::Path,
    source_name: &str,
) -> String {
    let root_text = root.display().to_string();
    let mut out = system_prompt.replace("${CLAUDE_PLUGIN_ROOT}", &root_text);
    if let Some(data_dir) = plugin_data_dir(source_name) {
        out = out.replace("${CLAUDE_PLUGIN_DATA}", &data_dir.display().to_string());
    }
    out
}

fn collect_plugin_markdown_files(
    dir: &std::path::Path,
    namespace: &[String],
    out: &mut Vec<(std::path::PathBuf, Vec<String>)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries = entries
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            let mut child_namespace = namespace.to_vec();
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                child_namespace.push(name.to_string());
            }
            collect_plugin_markdown_files(&path, &child_namespace, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push((path, namespace.to_vec()));
        }
    }
}

fn load_plugin_agents(
    project_root: &std::path::Path,
) -> Vec<claude_tools::agent_tool::RuntimeAgentDefinition> {
    if claude_core::errors_util::is_env_truthy("CLAUDE_CODE_SIMPLE") {
        return Vec::new();
    }

    let mut agents = Vec::new();
    for (plugin_name, source, root) in enabled_plugin_roots(project_root) {
        let agents_dir = root.join("agents");
        let mut files = Vec::new();
        collect_plugin_markdown_files(&agents_dir, &[], &mut files);
        let source_name = format!("{plugin_name}@{source}");
        for (path, namespace) in files {
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let parsed = claude_core::frontmatter::parse_frontmatter(&raw);
            let base_name = parsed
                .frontmatter
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .or_else(|| {
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "agent".to_string());
            let mut name_parts = vec![plugin_name.clone()];
            name_parts.extend(namespace);
            name_parts.push(base_name);
            let agent_type = name_parts.join(":");
            let when_to_use = parsed
                .frontmatter
                .get("description")
                .and_then(|value| value.as_str())
                .or_else(|| {
                    parsed
                        .frontmatter
                        .get("when-to-use")
                        .and_then(|value| value.as_str())
                })
                .map(str::to_string)
                .unwrap_or_else(|| format!("Agent from {plugin_name} plugin"));
            let model = parsed.frontmatter.get("model").and_then(|value| {
                let trimmed = value.as_str()?.trim();
                if trimmed.is_empty() {
                    None
                } else if trimmed.eq_ignore_ascii_case("inherit") {
                    Some("inherit".to_string())
                } else {
                    Some(trimmed.to_string())
                }
            });
            agents.push(claude_tools::agent_tool::RuntimeAgentDefinition {
                agent_type,
                when_to_use,
                source: claude_tools::agent_tool::AgentSource::Plugin,
                tools: parse_agent_frontmatter_tools(parsed.frontmatter.get("tools")),
                disallowed_tools: if parsed.frontmatter.contains_key("disallowedTools") {
                    parse_agent_frontmatter_tools(parsed.frontmatter.get("disallowedTools"))
                } else {
                    None
                },
                system_prompt: Some(substitute_plugin_variables(
                    parsed.content.trim(),
                    &root,
                    &source_name,
                )),
                model,
                permission_mode: None,
                background: frontmatter_bool(&parsed.frontmatter, "background"),
                isolation: frontmatter_string(&parsed.frontmatter, "isolation")
                    .filter(|value| *value == "worktree")
                    .map(str::to_string),
                initial_prompt: None,
                required_mcp_servers: frontmatter_string_list(
                    &parsed.frontmatter,
                    "requiredMcpServers",
                ),
            });
        }
    }
    agents
}

fn stream_json_agent_names(project_root: &std::path::Path) -> Vec<String> {
    let _ = project_root;
    claude_tools::agent_tool::active_agent_names()
}

fn stream_json_slash_commands(
    registered_skills: &[claude_tools::skill_tool::SkillEntry],
    discovered_skills: &[claude_core::plugins::types::Skill],
    mcp_prompt_commands: &[String],
) -> Vec<String> {
    let mut commands = Vec::new();

    commands.extend(stream_json_registered_skill_names(registered_skills));
    commands.extend(
        discovered_skills
            .iter()
            .filter(|skill| {
                !matches!(
                    skill.source,
                    claude_core::plugins::types::SkillSource::Plugin(_)
                )
            })
            .filter(|skill| !skill.is_plugin_command)
            .filter(|skill| skill.user_invocable)
            .map(|skill| skill.name.clone()),
    );
    commands.extend(
        discovered_skills
            .iter()
            .filter(|skill| skill.is_plugin_command)
            .filter(|skill| skill.user_invocable)
            .map(|skill| skill.name.clone()),
    );
    commands.extend(
        discovered_skills
            .iter()
            .filter(|skill| {
                matches!(
                    skill.source,
                    claude_core::plugins::types::SkillSource::Plugin(_)
                )
            })
            .filter(|skill| !skill.is_plugin_command)
            .filter(|skill| skill.user_invocable)
            .map(|skill| skill.name.clone()),
    );

    let registry = claude_core::commands::builtin::build_default_commands();
    for name in [
        "clear",
        "compact",
        "context",
        "heapdump",
        "init",
        "review",
        "security-review",
        "extra-usage",
        "usage",
        "insights",
        "team-onboarding",
    ] {
        if registry.get(name).is_some() {
            commands.push(name.to_string());
        }
    }
    commands.extend(mcp_prompt_commands.iter().cloned());

    let mut seen = std::collections::HashSet::new();
    commands
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

fn stream_json_skill_names(
    registered_skills: &[claude_tools::skill_tool::SkillEntry],
    discovered_skills: &[claude_core::plugins::types::Skill],
) -> Vec<String> {
    let mut names = stream_json_registered_skill_names(registered_skills);
    let mut seen = names
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();

    for name in stream_json_discovered_skill_names(discovered_skills) {
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }

    names
}

fn skills_reminder_block(skills: &[claude_core::plugins::types::Skill]) -> String {
    let mut skills_text = String::from(
        "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n",
    );
    for skill in skills {
        skills_text.push_str(&format!("- {}: {}", skill.name, skill.description));
        if let Some(ref hint) = skill.when_to_use {
            skills_text.push_str(&format!(" (use when: {})", hint));
        }
        skills_text.push('\n');
    }
    skills_text.push_str("</system-reminder>\n");
    skills_text
}

fn dynamic_skill_file_paths(tool_name: &str, input: &serde_json::Value) -> Vec<std::path::PathBuf> {
    let key = match tool_name {
        "Read" | "Edit" | "Write" => "file_path",
        _ => return Vec::new(),
    };
    input
        .get(key)
        .and_then(|value| value.as_str())
        .map(std::path::PathBuf::from)
        .into_iter()
        .collect()
}

fn stream_json_registered_skill_names(
    registered_skills: &[claude_tools::skill_tool::SkillEntry],
) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for skill in registered_skills {
        if skill.prompt_phase == claude_tools::skill_tool::SkillPromptPhase::StaticCommand {
            continue;
        }
        if !skill.user_invocable {
            continue;
        }
        if seen.insert(skill.name.clone()) {
            names.push(skill.name.clone());
        }
    }

    names
}

fn stream_json_discovered_skill_names(
    discovered_skills: &[claude_core::plugins::types::Skill],
) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for skill in discovered_skills {
        if skill.is_plugin_command {
            continue;
        }
        if !skill.user_invocable {
            continue;
        }
        if seen.insert(skill.name.clone()) {
            names.push(skill.name.clone());
        }
    }

    names
}

fn stream_json_api_key_source(auth: &claude_core::api::client::AuthMethod) -> &'static str {
    match auth {
        claude_core::api::client::AuthMethod::OAuthToken(_) => "none",
        claude_core::api::client::AuthMethod::ApiKey(_) => {
            if std::env::var("ANTHROPIC_API_KEY")
                .map(|value| !value.is_empty())
                .unwrap_or(false)
            {
                "env"
            } else if std::env::var("CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR").is_ok() {
                "fd"
            } else {
                "user"
            }
        }
    }
}

fn stream_json_memory_paths(cwd: &std::path::Path) -> serde_json::Value {
    if !claude_core::memdir::paths::auto_memory_enabled() {
        return serde_json::json!({});
    }
    let mut auto = claude_core::memdir::paths::get_auto_mem_path(cwd)
        .display()
        .to_string();
    if !auto.ends_with(std::path::MAIN_SEPARATOR) {
        auto.push(std::path::MAIN_SEPARATOR);
    }
    serde_json::json!({ "auto": auto })
}

fn stream_json_mcp_servers_in_order(
    connections: Vec<claude_core::mcp::types::McpServerConnection>,
    order: &[String],
) -> Vec<serde_json::Value> {
    let mut by_name = connections
        .into_iter()
        .map(|conn| {
            let status = match conn.status {
                claude_core::mcp::types::McpConnectionStatus::Connected { .. } => "connected",
                claude_core::mcp::types::McpConnectionStatus::Failed { .. } => "failed",
                claude_core::mcp::types::McpConnectionStatus::Pending { .. } => "pending",
                claude_core::mcp::types::McpConnectionStatus::Disabled => "disabled",
                claude_core::mcp::types::McpConnectionStatus::NeedsAuth => "needs-auth",
            };
            (
                conn.name.clone(),
                serde_json::json!({
                    "name": conn.name,
                    "status": status,
                }),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();

    let mut out = Vec::new();
    for name in order {
        if let Some(value) = by_name.remove(name) {
            out.push(value);
        }
    }
    let mut remaining = by_name.into_iter().collect::<Vec<_>>();
    remaining.sort_by(|a, b| a.0.cmp(&b.0));
    out.extend(remaining.into_iter().map(|(_, value)| value));
    out
}

async fn wait_for_headless_mcp_prewait(
    manager: &Arc<RwLock<claude_core::mcp::manager::McpManager>>,
    expected_names: &[String],
) {
    if expected_names.is_empty() {
        return;
    }

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(2_000);
    loop {
        let connections = {
            let mgr = manager.read().await;
            mgr.connections().await
        };
        let by_name = connections
            .iter()
            .map(|conn| (conn.name.as_str(), conn))
            .collect::<std::collections::HashMap<_, _>>();
        let has_unsettled = expected_names.iter().any(|name| {
            by_name.get(name.as_str()).is_none_or(|conn| {
                matches!(
                    conn.status,
                    claude_core::mcp::types::McpConnectionStatus::Pending { .. }
                )
            })
        });

        if !has_unsettled || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let mgr = manager.read().await;
    if let Err(err) = mgr.refresh_tools().await {
        tracing::warn!(error = %err, "Failed to refresh MCP tools after startup prewait");
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeAiMcpServersResponse {
    data: Vec<ClaudeAiMcpServer>,
}

#[derive(Debug, Deserialize)]
struct ClaudeAiMcpServer {
    #[serde(rename = "id")]
    _id: String,
    display_name: String,
    url: String,
}

#[derive(Debug, Default)]
struct ClaudeAiMcpDiscovery {
    configs: std::collections::HashMap<String, claude_core::mcp::types::ScopedMcpServerConfig>,
    order: Vec<String>,
    original_urls: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct TsToolContract {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct ShadowMcpServer {
    name: String,
    url: Option<String>,
    needs_auth: bool,
    include_tools: bool,
}

async fn fetch_claude_ai_mcp_configs_if_eligible() -> ClaudeAiMcpDiscovery {
    use claude_core::mcp::types::{
        ConfigScope, McpHttpServerConfig, McpServerConfig, ScopedMcpServerConfig,
    };

    if is_env_defined_falsy("ENABLE_CLAUDEAI_MCP_SERVERS") {
        tracing::debug!("[claudeai-mcp] disabled by ENABLE_CLAUDEAI_MCP_SERVERS");
        return ClaudeAiMcpDiscovery::default();
    }

    let Ok(Some(tokens)) = claude_core::auth::storage::load_tokens().await else {
        tracing::debug!("[claudeai-mcp] no Claude.ai OAuth token");
        return ClaudeAiMcpDiscovery::default();
    };
    if tokens.access_token.is_empty() {
        tracing::debug!("[claudeai-mcp] empty Claude.ai OAuth token");
        return ClaudeAiMcpDiscovery::default();
    }
    if !tokens
        .scopes
        .iter()
        .any(|scope| scope == "user:mcp_servers")
    {
        tracing::debug!(
            scopes = ?tokens.scopes,
            "[claudeai-mcp] missing user:mcp_servers scope"
        );
        return ClaudeAiMcpDiscovery::default();
    }

    let url =
        claude_core::auth::login::proxy_url("https://api.anthropic.com/v1/mcp_servers?limit=1000");
    let response = claude_core::auth::login::debug_http_client()
        .get(url)
        .header("Authorization", format!("Bearer {}", tokens.access_token))
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "mcp-servers-2025-12-04")
        .header("anthropic-version", "2023-06-01")
        .timeout(std::time::Duration::from_millis(5_000))
        .send()
        .await;

    let Ok(response) = response else {
        tracing::debug!("[claudeai-mcp] fetch failed");
        return ClaudeAiMcpDiscovery::default();
    };
    let Ok(payload) = response.json::<ClaudeAiMcpServersResponse>().await else {
        tracing::debug!("[claudeai-mcp] failed to decode response");
        return ClaudeAiMcpDiscovery::default();
    };

    let mut discovery = ClaudeAiMcpDiscovery::default();
    let mut used_normalized_names = std::collections::HashSet::new();
    for server in payload.data {
        let base_name = format!("claude.ai {}", server.display_name);
        let mut final_name = base_name.clone();
        let mut final_normalized = normalize_name_for_mcp(&final_name);
        let mut count = 1;
        while used_normalized_names.contains(&final_normalized) {
            count += 1;
            final_name = format!("{} ({})", base_name, count);
            final_normalized = normalize_name_for_mcp(&final_name);
        }
        used_normalized_names.insert(final_normalized);

        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", tokens.access_token),
        );
        headers.insert(
            "User-Agent".to_string(),
            claude_core::user_agent::get_mcp_user_agent(),
        );
        headers.insert(
            "X-Mcp-Client-Session-Id".to_string(),
            claude_core::api::client::get_session_id().clone(),
        );
        let proxy_url = match claude_core::constants::oauth::get_oauth_config() {
            Ok(oauth) => format!(
                "{}{}",
                oauth.mcp_proxy_url,
                oauth.mcp_proxy_path.replace("{server_id}", &server._id)
            ),
            Err(_) => server.url.clone(),
        };

        discovery.configs.insert(
            final_name.clone(),
            ScopedMcpServerConfig {
                config: McpServerConfig::Http(McpHttpServerConfig {
                    url: proxy_url,
                    headers: Some(headers),
                    oauth: None,
                }),
                scope: ConfigScope::ClaudeAi,
            },
        );
        discovery
            .original_urls
            .insert(final_name.clone(), server.url);
        discovery.order.push(final_name);
    }

    tracing::debug!(
        count = discovery.configs.len(),
        "[claudeai-mcp] fetched servers"
    );
    discovery
}

fn mcp_contract_shadow_tools(
    servers: impl IntoIterator<Item = ShadowMcpServer>,
) -> Vec<claude_core::mcp::manager::McpToolInfo> {
    let Ok(contracts) = serde_json::from_str::<Vec<TsToolContract>>(include_str!(
        "../../claude-tools/src/ts_tool_contracts_2_1_119.json"
    )) else {
        return Vec::new();
    };

    let server_by_normalized = servers
        .into_iter()
        .filter(|server| server.include_tools)
        .map(|server| (normalize_name_for_mcp(&server.name), server))
        .collect::<std::collections::HashMap<_, _>>();

    let mut tools = Vec::new();
    for server in server_by_normalized
        .values()
        .filter(|server| server.needs_auth)
    {
        tools.extend(mcp_auth_shadow_tools(server));
    }

    tools.extend(contracts.into_iter().filter_map(|contract| {
        let tool_name = contract.name.clone();
        let rest = tool_name.strip_prefix("mcp__")?;
        let (normalized_server, original_name) = rest.split_once("__")?;
        let server = server_by_normalized.get(normalized_server)?;
        if server.needs_auth {
            return None;
        }
        Some(claude_core::mcp::manager::McpToolInfo {
            name: contract.name,
            original_name: original_name.to_string(),
            server_name: server.name.clone(),
            description: Some(refresh_shadow_tool_description(contract.description)),
            input_schema: contract.input_schema,
        })
    }));
    tools
}

fn refresh_shadow_tool_description(description: String) -> String {
    refresh_shadow_tool_description_for_date(description, chrono::Local::now().date_naive())
}

fn refresh_shadow_tool_description_for_date(
    description: String,
    date: chrono::NaiveDate,
) -> String {
    let marker = "Knowledge up-to-date as at ";
    let Some(marker_start) = description.find(marker) else {
        return description;
    };
    let date_start = marker_start + marker.len();
    let Some(relative_end) = description[date_start..].find('.') else {
        return description;
    };
    let date_end = date_start + relative_end;
    let current = format_long_english_date(date);
    format!(
        "{}{}{}",
        &description[..date_start],
        current,
        &description[date_end..]
    )
}

fn format_long_english_date(date: chrono::NaiveDate) -> String {
    use chrono::Datelike;

    let month = match date.month() {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => unreachable!("chrono month is always 1..=12"),
    };
    format!("{} {} {}", date.day(), month, date.year())
}

fn mcp_auth_shadow_tools(server: &ShadowMcpServer) -> Vec<claude_core::mcp::manager::McpToolInfo> {
    let normalized_server = normalize_name_for_mcp(&server.name);
    let server_label = server.name.as_str();
    let url = server.url.as_deref().unwrap_or("unknown URL");
    vec![
        claude_core::mcp::manager::McpToolInfo {
            name: format!("mcp__{}__authenticate", normalized_server),
            original_name: "authenticate".to_string(),
            server_name: server.name.clone(),
            description: Some(format!(
                "The `{server_label}` MCP server (claudeai-proxy at {url}) is installed but requires authentication. Call this tool to start the OAuth flow — you'll receive an authorization URL to share with the user. Once the user completes authorization in their browser, the server's real tools will become available automatically."
            )),
            input_schema: Some(serde_json::json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "additionalProperties": false,
                "properties": {},
                "type": "object",
            })),
        },
        claude_core::mcp::manager::McpToolInfo {
            name: format!("mcp__{}__complete_authentication", normalized_server),
            original_name: "complete_authentication".to_string(),
            server_name: server.name.clone(),
            description: Some(format!(
                "Complete an in-progress OAuth flow for the `{server_label}` MCP server by submitting the callback URL. Call `mcp__{normalized_server}__authenticate` first to start the flow and get the authorization URL. After the user authorizes in their browser, the browser is redirected to a `http://localhost:<port>/callback?code=...&state=...` URL — on remote sessions that page fails to load, but the URL in the address bar is still valid. Pass that full URL here as `callback_url`."
            )),
            input_schema: Some(serde_json::json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "additionalProperties": false,
                "properties": {
                    "callback_url": {
                        "description": "The full callback URL from the browser address bar after authorizing, e.g. http://localhost:<port>/callback?code=...&state=...",
                        "type": "string",
                    }
                },
                "required": ["callback_url"],
                "type": "object",
            })),
        },
    ]
}

fn mcp_config_url(config: &claude_core::mcp::types::ScopedMcpServerConfig) -> Option<String> {
    match &config.config {
        claude_core::mcp::types::McpServerConfig::Http(http) => Some(http.url.clone()),
        claude_core::mcp::types::McpServerConfig::Sse(sse)
        | claude_core::mcp::types::McpServerConfig::SseIde(sse) => Some(sse.url.clone()),
        _ => None,
    }
}

fn claude_ai_server_uses_auth_shadow(
    name: &str,
    _config: &claude_core::mcp::types::ScopedMcpServerConfig,
) -> bool {
    // TS respects the MCP needs-auth cache before attempting another remote
    // connection. Do the same here instead of inferring auth state from URL
    // substrings such as "login".
    claude_core::mcp::auth_cache::is_mcp_auth_cached(name)
}

fn dedup_claude_ai_mcp_servers(
    claude_ai_servers: std::collections::HashMap<
        String,
        claude_core::mcp::types::ScopedMcpServerConfig,
    >,
    manual_servers: &std::collections::HashMap<
        String,
        claude_core::mcp::types::ScopedMcpServerConfig,
    >,
) -> std::collections::HashMap<String, claude_core::mcp::types::ScopedMcpServerConfig> {
    let manual_sigs = manual_servers
        .iter()
        .filter_map(|(name, config)| mcp_server_signature(config).map(|sig| (sig, name.clone())))
        .collect::<std::collections::HashMap<_, _>>();

    claude_ai_servers
        .into_iter()
        .filter_map(|(name, config)| {
            if let Some(sig) = mcp_server_signature(&config) {
                if let Some(duplicate_of) = manual_sigs.get(&sig) {
                    tracing::debug!(
                        server = %name,
                        duplicate_of = %duplicate_of,
                        "suppressing duplicate claude.ai MCP connector"
                    );
                    return None;
                }
            }
            Some((name, config))
        })
        .collect()
}

fn emit_stream_json(value: serde_json::Value) {
    if let Some(tx) = REMOTE_STREAM_JSON_TX.get() {
        let _ = tx.send(value.clone());
    }
    println!(
        "{}",
        serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
    );
}

fn normalize_model_display_for_stream_json(model: &str) -> String {
    model.replace("[1M]", "[1m]").replace("[2M]", "[2m]")
}

#[allow(dead_code)]
fn format_bash_tool_result_for_model(data: &serde_json::Value) -> String {
    let Some(obj) = data.as_object() else {
        return data.as_str().unwrap_or(&data.to_string()).to_string();
    };
    let stdout = obj.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr = obj.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
    let stdout = process_bash_stdout_for_model(stdout);
    let mut parts = Vec::new();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, true) => parts.push(stdout),
        (true, false) => parts.push(stderr.trim_end_matches('\n').to_string()),
        (false, false) => parts.push(
            format!("{stdout}\n{stderr}")
                .trim_end_matches('\n')
                .to_string(),
        ),
        (true, true) => {}
    }

    let task_id = obj
        .get("backgroundTaskId")
        .or_else(|| obj.get("task_id"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let output_path = obj
        .get("outputPath")
        .or_else(|| obj.get("output_file"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if !task_id.is_empty() && !output_path.is_empty() {
        let background_info = if obj
            .get("assistantAutoBackgrounded")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            format!("Command exceeded the assistant-mode blocking budget (15s) and was moved to the background with ID: {task_id}. It is still running — you will be notified when it completes. Output is being written to: {output_path}. In assistant mode, delegate long-running work to a subagent or use run_in_background to keep this conversation responsive.")
        } else if obj
            .get("backgroundedByUser")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            format!(
                "Command was manually backgrounded by user with ID: {task_id}. Output is being written to: {output_path}"
            )
        } else {
            format!(
                "Command running in background with ID: {task_id}. Output is being written to: {output_path}"
            )
        };
        parts.push(background_info);
    }

    parts.retain(|part| !part.is_empty());
    parts.join("\n")
}

#[allow(dead_code)]
fn process_bash_stdout_for_model(stdout: &str) -> String {
    let mut rest = stdout;
    loop {
        let Some(index) = rest.find('\n') else {
            break;
        };
        let line_with_newline = &rest[..=index];
        if line_with_newline.trim().is_empty() {
            rest = &rest[index + 1..];
        } else {
            break;
        }
    }
    rest.trim_end_matches('\n').to_string()
}

fn format_tool_result_for_model(tool_name: &str, data: &serde_json::Value) -> String {
    claude_core::tool_result_format::format_tool_result_for_model(tool_name, data)
}

#[allow(dead_code)]
fn add_line_numbers_ts(content: &str, start_line: usize) -> String {
    if content.is_empty() {
        return String::new();
    }
    content
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .enumerate()
        .map(|(index, line)| format!("{}\t{}", start_line + index, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_tool_result_content_for_model(
    tool_name: &str,
    data: &serde_json::Value,
) -> serde_json::Value {
    claude_core::tool_result_format::format_tool_result_content_for_model(tool_name, data)
}

#[allow(dead_code)]
fn ensure_non_empty_tool_result_string(tool_name: &str, content: String) -> String {
    if content.trim().is_empty() {
        format!("({tool_name} completed with no output)")
    } else {
        content
    }
}

#[allow(dead_code)]
fn ensure_non_empty_tool_result_content(
    tool_name: &str,
    content: serde_json::Value,
) -> serde_json::Value {
    claude_core::tool_result_format::ensure_non_empty_tool_result_content(tool_name, content)
}

#[allow(dead_code)]
fn is_tool_result_content_empty(content: &serde_json::Value) -> bool {
    match content {
        serde_json::Value::Null => true,
        serde_json::Value::String(text) => text.trim().is_empty(),
        serde_json::Value::Array(blocks) => {
            blocks.is_empty()
                || blocks.iter().all(|block| {
                    block.get("type").and_then(|value| value.as_str()) == Some("text")
                        && block
                            .get("text")
                            .and_then(|value| value.as_str())
                            .map(str::trim)
                            .unwrap_or("")
                            .is_empty()
                })
        }
        _ => false,
    }
}

#[allow(dead_code)]
fn truncate_single_line(text: &str, max_width: usize) -> String {
    let first_line = text.split('\n').next().unwrap_or("");
    let had_newline = first_line.len() != text.len();
    let mut truncated: String = first_line.chars().take(max_width).collect();
    let over_width = first_line.chars().count() > max_width;
    if over_width {
        if max_width <= 1 {
            return "…".to_string();
        }
        truncated = first_line.chars().take(max_width - 1).collect();
    }
    if had_newline || over_width {
        truncated.push('…');
    }
    truncated
}

#[allow(dead_code)]
fn format_agent_tool_result_content_for_model(data: &serde_json::Value) -> serde_json::Value {
    let Some(status) = data.get("status").and_then(|value| value.as_str()) else {
        return serde_json::Value::String(data.to_string());
    };

    if status == "teammate_spawned" {
        let teammate_id = data
            .get("teammate_id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let name = data
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let team_name = data
            .get("team_name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return serde_json::json!([{
            "type": "text",
            "text": format!("Spawned successfully.\nagent_id: {teammate_id}\nname: {name}\nteam_name: {team_name}\nThe agent is now running and will receive instructions via mailbox.")
        }]);
    }

    if status == "remote_launched" {
        let task_id = data
            .get("taskId")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let session_url = data
            .get("sessionUrl")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let output_file = data
            .get("outputFile")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return serde_json::json!([{
            "type": "text",
            "text": format!("Remote agent launched in CCR.\ntaskId: {task_id}\nsession_url: {session_url}\noutput_file: {output_file}\nThe agent is running remotely. You will be notified automatically when it completes.\nBriefly tell the user what you launched and end your response.")
        }]);
    }

    if status == "async_launched" {
        let agent_id = data
            .get("agentId")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let prefix = format!(
            "Async agent launched successfully.\nagentId: {agent_id} (internal ID - do not mention to user. Use SendMessage with to: '{agent_id}' to continue this agent.)\nThe agent is working in the background. You will be notified automatically when it completes."
        );
        let instructions = if data
            .get("canReadOutputFile")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            let output_file = data
                .get("outputFile")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            format!("Do not duplicate this agent's work — avoid working with the same files or topics it is using. Work on non-overlapping tasks, or briefly tell the user what you launched and end your response.\noutput_file: {output_file}\nIf asked, you can check progress before completion by using Read or Bash tail on the output file.")
        } else {
            "Briefly tell the user what you launched and end your response. Do not generate any other text — agent results will arrive in a subsequent message.".to_string()
        };
        return serde_json::json!([{ "type": "text", "text": format!("{prefix}\n{instructions}") }]);
    }

    if status == "completed" {
        let mut content = data
            .get("content")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        if content.is_empty() {
            content.push(serde_json::json!({
                    "type": "text",
                    "text": "(Subagent completed but returned no output.)"
            }));
        }

        let worktree_info = match (
            data.get("worktreePath").and_then(|value| value.as_str()),
            data.get("worktreeBranch").and_then(|value| value.as_str()),
        ) {
            (Some(path), Some(branch)) => {
                format!("\nworktreePath: {path}\nworktreeBranch: {branch}")
            }
            _ => String::new(),
        };
        let one_shot = data
            .get("agentType")
            .and_then(|value| value.as_str())
            .map(|agent_type| matches!(agent_type, "Explore" | "Plan"))
            .unwrap_or(false);
        if one_shot && worktree_info.is_empty() {
            return serde_json::Value::Array(content);
        }

        let agent_id = data
            .get("agentId")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let total_tokens = data
            .get("totalTokens")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        let total_tool_use_count = data
            .get("totalToolUseCount")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        let total_duration_ms = data
            .get("totalDurationMs")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        content.push(serde_json::json!({
            "type": "text",
            "text": format!("agentId: {agent_id} (use SendMessage with to: '{agent_id}' to continue this agent){worktree_info}\n<usage>total_tokens: {total_tokens}\ntool_uses: {total_tool_use_count}\nduration_ms: {total_duration_ms}</usage>")
        }));
        return serde_json::Value::Array(content);
    }

    serde_json::Value::String(data.to_string())
}

#[allow(dead_code)]
fn format_tool_result_string_for_model(tool_name: &str, data: &serde_json::Value) -> String {
    if tool_name == "Bash" || tool_name == "PowerShell" {
        return format_bash_tool_result_for_model(data);
    }

    if tool_name == "Monitor" {
        let task_id = data
            .get("backgroundTaskId")
            .or_else(|| data.get("task_id"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let output_path = data
            .get("outputPath")
            .or_else(|| data.get("output_file"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !task_id.is_empty() && !output_path.is_empty() {
            return format!(
                "Monitor running in background with ID: {task_id}. Output is being written to: {output_path}"
            );
        }
        return data.to_string();
    }

    if tool_name == "AskUserQuestion" || tool_name == "AskUser" {
        let answers_text = data
            .get("answers")
            .and_then(|value| value.as_object())
            .map(|answers| {
                answers
                    .iter()
                    .map(|(question_text, answer)| {
                        let answer = answer.as_str().unwrap_or("");
                        let mut parts = vec![format!("\"{question_text}\"=\"{answer}\"")];
                        let annotation = data
                            .get("annotations")
                            .and_then(|value| value.as_object())
                            .and_then(|annotations| annotations.get(question_text));
                        if let Some(preview) = annotation
                            .and_then(|value| value.get("preview"))
                            .and_then(|value| value.as_str())
                        {
                            parts.push(format!("selected preview:\n{preview}"));
                        }
                        if let Some(notes) = annotation
                            .and_then(|value| value.get("notes"))
                            .and_then(|value| value.as_str())
                        {
                            parts.push(format!("user notes: {notes}"));
                        }
                        parts.join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        return format!(
            "User has answered your questions: {answers_text}. You can now continue with the user's answers in mind."
        );
    }

    if tool_name == "LSP" {
        return data
            .get("result")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_else(|| data.to_string());
    }

    if tool_name == "SendMessage" || tool_name == "TaskStop" {
        return data.to_string();
    }

    if tool_name == "CronCreate" || tool_name == "ScheduleCron" {
        let id = data
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let human_schedule = data
            .get("humanSchedule")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let recurring = data
            .get("recurring")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let durable = data
            .get("durable")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let where_text = if durable {
            "Persisted to .claude/scheduled_tasks.json"
        } else {
            "Session-only (not written to disk, dies when Claude exits)"
        };
        if recurring {
            return format!(
                "Scheduled recurring job {id} ({human_schedule}). {where_text}. Auto-expires after 7 days. Use CronDelete to cancel sooner."
            );
        }
        return format!(
            "Scheduled one-shot task {id} ({human_schedule}). {where_text}. It will fire once then auto-delete."
        );
    }

    if tool_name == "CronDelete" {
        let id = data
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return format!("Cancelled job {id}.");
    }

    if tool_name == "CronList" {
        let Some(jobs) = data.get("jobs").and_then(|value| value.as_array()) else {
            return "No scheduled jobs.".to_string();
        };
        if jobs.is_empty() {
            return "No scheduled jobs.".to_string();
        }
        return jobs
            .iter()
            .map(|job| {
                let id = job.get("id").and_then(|value| value.as_str()).unwrap_or("");
                let human_schedule = job
                    .get("humanSchedule")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let recurring = job
                    .get("recurring")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let durable_suffix =
                    if job.get("durable").and_then(|value| value.as_bool()) == Some(false) {
                        " [session-only]"
                    } else {
                        ""
                    };
                let prompt = job
                    .get("prompt")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                format!(
                    "{id} — {human_schedule}{}{}: {}",
                    if recurring {
                        " (recurring)"
                    } else {
                        " (one-shot)"
                    },
                    durable_suffix,
                    truncate_single_line(prompt, 80)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
    }

    if tool_name == "RemoteTrigger" {
        if let (Some(status), Some(json)) = (
            data.get("status").and_then(|value| value.as_i64()),
            data.get("json").and_then(|value| value.as_str()),
        ) {
            return format!("HTTP {status}\n{json}");
        }
        return data.to_string();
    }

    if tool_name == "EnterPlanMode" {
        let message = data
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.");
        let mut content = format!(
            "{message}\n\nIn plan mode, you should:\n1. Thoroughly explore the codebase to understand existing patterns\n2. Identify similar features and architectural approaches\n3. Consider multiple approaches and their trade-offs\n4. Use AskUserQuestion if you need to clarify the approach\n5. Design a concrete implementation strategy\n6. When ready, use ExitPlanMode to present your plan for approval\n\nRemember: DO NOT write or edit any files yet. This is a read-only exploration and planning phase."
        );
        if let Some(instructions) = data.get("instructions").and_then(|value| value.as_str()) {
            content.push_str("\n\n<system-reminder>\n");
            content.push_str(instructions);
            content.push_str("\n</system-reminder>");
        }
        return content;
    }

    if tool_name == "ExitPlanMode" {
        if data
            .get("isAgent")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return "User has approved the plan. There is nothing else needed from you now. Please respond with \"ok\"".to_string();
        }
        let plan = data
            .get("plan")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if plan.trim().is_empty() {
            return "User has approved exiting plan mode. You can now proceed.".to_string();
        }
        let file_path = data
            .get("filePath")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let team_hint = if data
            .get("hasTaskTool")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            "\n\nIf this plan can be broken down into multiple independent tasks, consider using the Task tool to create a team and parallelize the work."
        } else {
            ""
        };
        let plan_label = if data
            .get("planWasEdited")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            "Approved Plan (edited by user)"
        } else {
            "Approved Plan"
        };
        return format!(
            "User has approved your plan. You can now start coding. Start with updating your todo list if applicable\n\nYour plan has been saved to: {file_path}\nYou can refer back to it if needed during implementation.{team_hint}\n\n## {plan_label}:\n{plan}"
        );
    }

    if tool_name == "Write" {
        if let (Some(file_path), Some(write_type)) = (
            data.get("filePath").and_then(|value| value.as_str()),
            data.get("type").and_then(|value| value.as_str()),
        ) {
            return match write_type {
                "create" => format!("File created successfully at: {file_path}"),
                "update" => format!("The file {file_path} has been updated successfully."),
                _ => data.to_string(),
            };
        }
    }

    if tool_name == "Edit" {
        if let Some(file_path) = data.get("filePath").and_then(|value| value.as_str()) {
            let modified_note = if data
                .get("userModified")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                ".  The user modified your proposed changes before accepting them. "
            } else {
                ""
            };
            if data
                .get("replaceAll")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return format!(
                    "The file {file_path} has been updated{modified_note}. All occurrences were successfully replaced."
                );
            }
            return format!("The file {file_path} has been updated successfully{modified_note}.");
        }
    }

    if tool_name == "TodoWrite" {
        let base = "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable";
        if data
            .get("verificationNudgeNeeded")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return format!("{base}\n\nNOTE: You just closed out 3+ tasks and none of them was a verification step. Before writing your final summary, spawn the verification agent (subagent_type=\"verification\"). You cannot self-assign PARTIAL by listing caveats in your summary — only the verifier issues a verdict.");
        }
        return base.to_string();
    }

    if tool_name == "NotebookEdit" {
        if let Some(error) = data.get("error").and_then(|value| value.as_str()) {
            if !error.is_empty() {
                return error.to_string();
            }
        }
        let cell_id = data
            .get("cell_id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let new_source = data
            .get("new_source")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return match data.get("edit_mode").and_then(|value| value.as_str()) {
            Some("replace") => format!("Updated cell {cell_id} with {new_source}"),
            Some("insert") => format!("Inserted cell {cell_id} with {new_source}"),
            Some("delete") => format!("Deleted cell {cell_id}"),
            _ => "Unknown edit mode".to_string(),
        };
    }

    if tool_name == "Skill" {
        if let Some(status) = data.get("status").and_then(|value| value.as_str()) {
            if status == "forked" {
                let command_name = data
                    .get("commandName")
                    .or_else(|| data.get("skill"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let result = data
                    .get("result")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                return format!(
                    "Skill \"{command_name}\" completed (forked execution).\n\nResult:\n{result}"
                );
            }
        }
        let command_name = data
            .get("commandName")
            .or_else(|| data.get("skill"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return format!("Launching skill: {command_name}");
    }

    if tool_name == "TaskOutput" {
        let mut parts = vec![format!(
            "<retrieval_status>{}</retrieval_status>",
            data.get("retrieval_status")
                .and_then(|value| value.as_str())
                .unwrap_or("found")
        )];
        let task_value = data.get("task").unwrap_or(data);
        if let Some(task_id) = task_value
            .get("task_id")
            .or_else(|| task_value.get("taskId"))
            .and_then(|value| value.as_str())
        {
            parts.push(format!("<task_id>{task_id}</task_id>"));
            let task_type = task_value
                .get("task_type")
                .or_else(|| task_value.get("taskType"))
                .and_then(|value| value.as_str())
                .unwrap_or("background");
            parts.push(format!("<task_type>{task_type}</task_type>"));
            if let Some(status) = task_value.get("status").and_then(|value| value.as_str()) {
                parts.push(format!("<status>{status}</status>"));
            }
            if let Some(exit_code) = task_value
                .get("exitCode")
                .or_else(|| task_value.get("exit_code"))
                .and_then(|value| value.as_i64())
            {
                parts.push(format!("<exit_code>{exit_code}</exit_code>"));
            }
            if let Some(output) = task_value.get("output").and_then(|value| value.as_str()) {
                if !output.trim().is_empty() {
                    parts.push(format!("<output>\n{}\n</output>", output.trim_end()));
                }
            }
            if let Some(error) = task_value.get("error").and_then(|value| value.as_str()) {
                parts.push(format!("<error>{error}</error>"));
            }
        }
        return parts.join("\n\n");
    }

    if tool_name == "TaskCreate" {
        let task = data.get("task").unwrap_or(data);
        if let (Some(id), Some(subject)) = (
            task.get("id").and_then(|value| value.as_str()),
            task.get("subject").and_then(|value| value.as_str()),
        ) {
            return format!("Task #{id} created successfully: {subject}");
        }
    }

    if tool_name == "TaskGet" {
        let task = data.get("task").unwrap_or(data);
        if task.is_null() {
            return "Task not found".to_string();
        }
        if let Some(id) = task.get("id").and_then(|value| value.as_str()) {
            let subject = task
                .get("subject")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let status = task
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let description = task
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let mut lines = vec![
                format!("Task #{id}: {subject}"),
                format!("Status: {status}"),
                format!("Description: {description}"),
            ];
            if let Some(blocked_by) = task.get("blockedBy").and_then(|value| value.as_array()) {
                if !blocked_by.is_empty() {
                    lines.push(format!(
                        "Blocked by: {}",
                        blocked_by
                            .iter()
                            .filter_map(|value| value.as_str().map(|id| format!("#{id}")))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            if let Some(blocks) = task.get("blocks").and_then(|value| value.as_array()) {
                if !blocks.is_empty() {
                    lines.push(format!(
                        "Blocks: {}",
                        blocks
                            .iter()
                            .filter_map(|value| value.as_str().map(|id| format!("#{id}")))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            return lines.join("\n");
        }
    }

    if tool_name == "TaskList" {
        if let Some(tasks) = data.get("tasks").and_then(|value| value.as_array()) {
            if tasks.is_empty() {
                return "No tasks found".to_string();
            }
            return tasks
                .iter()
                .filter_map(|task| {
                    let id = task.get("id").and_then(|value| value.as_str())?;
                    let status = task.get("status").and_then(|value| value.as_str())?;
                    let subject = task.get("subject").and_then(|value| value.as_str())?;
                    let owner = task
                        .get("owner")
                        .and_then(|value| value.as_str())
                        .map(|owner| format!(" ({owner})"))
                        .unwrap_or_default();
                    let blocked = task
                        .get("blockedBy")
                        .and_then(|value| value.as_array())
                        .filter(|items| !items.is_empty())
                        .map(|items| {
                            format!(
                                " [blocked by {}]",
                                items
                                    .iter()
                                    .filter_map(|value| value.as_str().map(|id| format!("#{id}")))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        })
                        .unwrap_or_default();
                    Some(format!("#{id} [{status}] {subject}{owner}{blocked}"))
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }

    if tool_name == "WebFetch" {
        if let Some(result) = data.get("result").and_then(|value| value.as_str()) {
            return result.to_string();
        }
    }

    if tool_name == "WebSearch" {
        if let Some(query) = data.get("query").and_then(|value| value.as_str()) {
            let mut formatted = format!("Web search results for query: \"{query}\"\n\n");
            if let Some(results) = data.get("results").and_then(|value| value.as_array()) {
                for result in results {
                    if result.is_null() {
                        continue;
                    }
                    if let Some(text) = result.as_str() {
                        formatted.push_str(text);
                        formatted.push_str("\n\n");
                    } else if result
                        .get("content")
                        .and_then(|value| value.as_array())
                        .map(|content| !content.is_empty())
                        .unwrap_or(false)
                    {
                        formatted.push_str("Links: ");
                        formatted.push_str(
                            &serde_json::to_string(result.get("content").unwrap())
                                .unwrap_or_else(|_| "[]".to_string()),
                        );
                        formatted.push_str("\n\n");
                    } else {
                        formatted.push_str("No links found.\n\n");
                    }
                }
            }
            formatted.push_str(
                "\nREMINDER: You MUST include the sources above in your response to the user using markdown hyperlinks.",
            );
            return formatted.trim().to_string();
        }
    }

    if tool_name == "Config" {
        if let Some(error) = data.get("error").and_then(|value| value.as_str()) {
            return format!("Error: {error}");
        }
        let action = data
            .get("operation")
            .or_else(|| data.get("action"))
            .and_then(|value| value.as_str());
        let setting = data
            .get("setting")
            .or_else(|| data.get("key"))
            .and_then(|value| value.as_str());
        if let (Some(action), Some(setting)) = (action, setting) {
            if action == "get" {
                let value = data.get("value").unwrap_or(&serde_json::Value::Null);
                return format!("{setting} = {}", json_stringify_for_ts(value));
            }
            if action == "set" {
                let value = data
                    .get("newValue")
                    .or_else(|| data.get("value"))
                    .unwrap_or(&serde_json::Value::Null);
                return format!("Set {setting} to {}", json_stringify_for_ts(value));
            }
        }
    }

    if tool_name == "ListMcpResourcesTool" {
        if data
            .as_array()
            .is_some_and(|resources| resources.is_empty())
        {
            return "No resources found. MCP servers may still provide tools even if they have no resources.".to_string();
        }
        return json_stringify_for_ts(data);
    }

    if let Some(text) = data.as_str() {
        return text.to_string();
    }

    if data.get("type").and_then(|value| value.as_str()) == Some("text") {
        if let Some(file) = data.get("file") {
            if let Some(content) = file.get("content").and_then(|content| content.as_str()) {
                let start_line = file
                    .get("startLine")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(1) as usize;
                return add_line_numbers_ts(content, start_line);
            }
        }
        if let Some(content) = data.get("content").and_then(|content| content.as_str()) {
            return content.to_string();
        }
    }

    if let Some(mode) = data.get("mode").and_then(|value| value.as_str()) {
        let content = data
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let limit_info = format_search_limit_info(data);
        match mode {
            "content" => {
                let result = if content.is_empty() {
                    "No matches found".to_string()
                } else {
                    content.to_string()
                };
                return if let Some(limit_info) = limit_info {
                    format!("{result}\n\n[Showing results with pagination = {limit_info}]")
                } else {
                    result
                };
            }
            "count" => {
                let raw_content = if content.is_empty() {
                    "No matches found".to_string()
                } else {
                    content.to_string()
                };
                let matches = data
                    .get("numMatches")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                let files = data
                    .get("numFiles")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                let occurrence = if matches == 1 {
                    "occurrence"
                } else {
                    "occurrences"
                };
                let file = if files == 1 { "file" } else { "files" };
                let mut summary =
                    format!("\n\nFound {matches} total {occurrence} across {files} {file}.");
                if let Some(limit_info) = limit_info {
                    summary.push_str(&format!(" with pagination = {limit_info}"));
                }
                return raw_content + &summary;
            }
            _ => {
                let filenames = data
                    .get("filenames")
                    .and_then(|value| value.as_array())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let files = data
                    .get("numFiles")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(filenames.len() as u64);
                if files == 0 {
                    return "No files found".to_string();
                }
                let file = if files == 1 { "file" } else { "files" };
                let limit = limit_info
                    .map(|info| format!(" {info}"))
                    .unwrap_or_default();
                return format!("Found {files} {file}{limit}\n{}", filenames.join("\n"));
            }
        }
    }

    if let Some(filenames) = data.get("filenames").and_then(|value| value.as_array()) {
        let mut lines = filenames
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        if data
            .get("truncated")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            lines.push(
                "(Results are truncated. Consider using a more specific path or pattern.)"
                    .to_string(),
            );
        }
        return if lines.is_empty() {
            "No files found".to_string()
        } else {
            lines.join("\n")
        };
    }

    if let Some(message) = data.get("message").and_then(|value| value.as_str()) {
        return message.to_string();
    }

    if let Some(error) = data.get("error").and_then(|value| value.as_str()) {
        return error.to_string();
    }

    data.to_string()
}

#[allow(dead_code)]
fn format_search_limit_info(data: &serde_json::Value) -> Option<String> {
    let limit = data.get("appliedLimit").and_then(|value| value.as_u64());
    let offset = data
        .get("appliedOffset")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    limit.map(|limit| {
        if offset > 0 {
            format!("limit: {limit}, offset: {offset}")
        } else {
            format!("limit: {limit}")
        }
    })
}

#[allow(dead_code)]
fn json_stringify_for_ts(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn permission_mode_hook_name(
    mode: &claude_core::permissions::types::PermissionMode,
) -> &'static str {
    use claude_core::permissions::types::PermissionMode;
    match mode {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Auto => "auto",
        PermissionMode::Plan => "plan",
        PermissionMode::BypassPermissions => "bypassPermissions",
        PermissionMode::DontAsk => "dontAsk",
        PermissionMode::Bubble => "bubble",
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionPromptToolOutput {
    behavior: String,
    #[serde(default)]
    updated_input: Option<serde_json::Value>,
    #[serde(default)]
    updated_permissions: Option<Vec<claude_core::permissions::types::PermissionUpdate>>,
    #[serde(default)]
    message: Option<String>,
}

fn prompt_tool_output_text(data: &serde_json::Value) -> anyhow::Result<&str> {
    data.as_str().ok_or_else(|| {
        anyhow::anyhow!(
            "Permission prompt tool returned an invalid result. Expected a single text block param with type=\"text\" and a string text value."
        )
    })
}

async fn run_permission_prompt_tool(
    permission_prompt_tool: &Arc<dyn claude_tools::ToolExecutor>,
    target_tool_name: &str,
    target_tool_use_id: &str,
    target_input: &serde_json::Value,
    cwd: &std::path::Path,
    read_file_state: Arc<std::sync::Mutex<claude_tools::registry::ReadFileState>>,
    permission_mode: claude_core::permissions::types::PermissionMode,
    model: &str,
    cancel: tokio_util::sync::CancellationToken,
) -> anyhow::Result<PermissionPromptToolOutput> {
    let ctx = claude_tools::ToolUseContext::new(
        cwd.to_path_buf(),
        read_file_state,
        permission_mode.clone(),
        claude_core::permissions::ToolPermissionContext {
            mode: permission_mode,
            working_directory: cwd.to_path_buf(),
            ..Default::default()
        },
        std::sync::Arc::new(
            claude_core::tool_use_context_options::ToolUseContextOptions::minimal(model),
        ),
        std::sync::Arc::new(claude_core::tool_host::NullToolHost),
    );
    let prompt_input = serde_json::json!({
        "tool_name": target_tool_name,
        "input": target_input,
        "tool_use_id": target_tool_use_id,
    });
    let result = permission_prompt_tool
        .call(&prompt_input, &ctx, cancel, None)
        .await?;
    let text = prompt_tool_output_text(&result.data)?;
    let parsed: PermissionPromptToolOutput = serde_json::from_str(text)?;
    Ok(parsed)
}

fn merge_hook_updated_input(
    input: &serde_json::Value,
    updated: &Option<std::collections::HashMap<String, serde_json::Value>>,
) -> serde_json::Value {
    let Some(updated) = updated else {
        return input.clone();
    };
    let mut merged = input.clone();
    if let Some(obj) = merged.as_object_mut() {
        for (key, value) in updated {
            obj.insert(key.clone(), value.clone());
        }
    }
    merged
}

fn permission_decision_to_rule_check(
    decision: &claude_core::permissions::types::PermissionDecision,
) -> claude_core::hooks::RuleCheckResult {
    use claude_core::hooks::RuleCheckResult;
    use claude_core::permissions::types::PermissionDecision;
    match decision {
        PermissionDecision::Allow(_) => RuleCheckResult::NoMatch,
        PermissionDecision::Ask(_) => RuleCheckResult::Ask,
        PermissionDecision::Deny(deny) => RuleCheckResult::Deny(Some(deny.message.clone())),
    }
}

fn hook_blocking_errors_text(errors: &[claude_core::hooks::HookBlockingError]) -> Option<String> {
    if errors.is_empty() {
        None
    } else {
        Some(
            errors
                .iter()
                .map(|err| err.blocking_error.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

fn stream_event_to_stream_json_events(
    ev: &claude_core::types::events::StreamEvent,
    session_id: &str,
    include_partial_messages: bool,
) -> Vec<serde_json::Value> {
    use claude_core::types::events::StreamEvent;
    use serde_json::json;

    let value = match ev {
        // TS stream-json exposes model messages, user tool-result turns, system
        // records, and the final result. It does not print Rust's internal
        // request/delta/progress bookkeeping events as first-class records.
        StreamEvent::RawSse { event } => {
            if include_partial_messages {
                json!({
                    "type": "stream_event",
                    "event": event,
                    "parent_tool_use_id": serde_json::Value::Null,
                    "session_id": session_id,
                    "uuid": uuid::Uuid::new_v4().to_string(),
                })
            } else {
                return Vec::new();
            }
        }
        StreamEvent::RequestStart { .. }
        | StreamEvent::ToolStart { .. }
        | StreamEvent::ToolProgress { .. }
        | StreamEvent::ToolResult { .. }
        | StreamEvent::ThinkingDelta { .. }
        | StreamEvent::TextDelta { .. }
        | StreamEvent::UsageUpdate(_)
        | StreamEvent::Done { .. } => return Vec::new(),
        StreamEvent::AssistantMessage(message) => {
            let mut sdk_message =
                serde_json::to_value(&message.message).unwrap_or(serde_json::Value::Null);
            normalize_stream_json_assistant_message(&mut sdk_message);
            return split_stream_json_assistant_message(
                sdk_message,
                session_id,
                message.uuid.to_string(),
            );
        }
        StreamEvent::Compacted { summary } => json!({
            "type": "system",
            "subtype": "compact_boundary",
            "session_id": session_id,
            "uuid": uuid::Uuid::new_v4().to_string(),
            "timestamp": chrono::Utc::now(),
            "message": {"summary": summary},
        }),
        StreamEvent::Error(error) => json!({"type": "error", "error": format!("{:?}", error)}),
    };
    vec![value]
}

fn stream_json_usage_value(
    usage: &claude_core::types::usage::Usage,
    inference_geo: &str,
    include_speed: bool,
) -> serde_json::Value {
    let cache_creation_input_tokens = usage.cache_creation_input_tokens.unwrap_or(0);
    let mut value = serde_json::json!({
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
        "cache_creation_input_tokens": cache_creation_input_tokens,
        "cache_read_input_tokens": usage.cache_read_input_tokens.unwrap_or(0),
        "cache_creation": {
            "ephemeral_5m_input_tokens": 0,
            "ephemeral_1h_input_tokens": cache_creation_input_tokens,
        },
        "service_tier": "standard",
        "inference_geo": inference_geo,
    });
    if include_speed {
        value["speed"] = serde_json::json!("standard");
    }
    value
}

fn normalize_stream_json_assistant_message(message: &mut serde_json::Value) {
    let Some(object) = message.as_object_mut() else {
        return;
    };
    object
        .entry("type".to_string())
        .or_insert_with(|| serde_json::json!("message"));
    object
        .entry("stop_sequence".to_string())
        .or_insert(serde_json::Value::Null);
    object
        .entry("stop_details".to_string())
        .or_insert(serde_json::Value::Null);
    object
        .entry("context_management".to_string())
        .or_insert(serde_json::Value::Null);

    if let Some(usage_value) = object.get("usage").cloned() {
        if let Ok(usage) = serde_json::from_value::<claude_core::types::usage::Usage>(usage_value) {
            object.insert(
                "usage".to_string(),
                stream_json_usage_value(&usage, "not_available", false),
            );
        }
    }
}

fn split_stream_json_assistant_message(
    message: serde_json::Value,
    session_id: &str,
    fallback_uuid: String,
) -> Vec<serde_json::Value> {
    let Some(content) = message.get("content").and_then(|value| value.as_array()) else {
        return vec![serde_json::json!({
            "type": "assistant",
            "message": message,
            "parent_tool_use_id": serde_json::Value::Null,
            "session_id": session_id,
            "uuid": fallback_uuid,
        })];
    };

    if content.len() <= 1 {
        let mut single = message;
        if let Some(object) = single.as_object_mut() {
            object.insert("stop_reason".to_string(), serde_json::Value::Null);
        }
        return vec![serde_json::json!({
            "type": "assistant",
            "message": single,
            "parent_tool_use_id": serde_json::Value::Null,
            "session_id": session_id,
            "uuid": fallback_uuid,
        })];
    }

    content
        .iter()
        .map(|block| {
            let mut split = message.clone();
            if let Some(object) = split.as_object_mut() {
                object.insert(
                    "content".to_string(),
                    serde_json::Value::Array(vec![block.clone()]),
                );
                object.insert("stop_reason".to_string(), serde_json::Value::Null);
            }
            serde_json::json!({
                "type": "assistant",
                "message": split,
                "parent_tool_use_id": serde_json::Value::Null,
                "session_id": session_id,
                "uuid": uuid::Uuid::new_v4().to_string(),
            })
        })
        .collect()
}

fn stream_json_user_tool_result_event(
    tool_results: Vec<serde_json::Value>,
    tool_use_results: Vec<serde_json::Value>,
    session_id: &str,
) -> serde_json::Value {
    let mut event = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": tool_results,
        },
        "parent_tool_use_id": serde_json::Value::Null,
        "session_id": session_id,
        "uuid": uuid::Uuid::new_v4().to_string(),
        "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    });
    if tool_use_results.len() == 1 {
        event["tool_use_result"] = tool_use_results.into_iter().next().unwrap();
    }
    event
}

fn stream_json_assistant_tool_use_event(
    tool_info: &claude_core::query::engine::ToolUseInfo,
    session_id: &str,
    model: &str,
    usage: Option<&claude_core::types::usage::Usage>,
) -> serde_json::Value {
    let api_model = tool_info
        .model
        .clone()
        .unwrap_or_else(|| model.replace("[1m]", "").replace("[1M]", ""));
    let api_message_id = tool_info
        .message_id
        .clone()
        .unwrap_or_else(|| format!("msg_{}", uuid::Uuid::new_v4().simple()));
    let api_usage = tool_info.usage.as_ref().or(usage);
    serde_json::json!({
        "type": "assistant",
        "message": {
            "id": api_message_id,
            "type": "message",
            "role": "assistant",
            "model": api_model,
            "content": [{
                "type": "tool_use",
                "id": tool_info.id,
                "name": tool_info.name,
                "input": tool_info.input,
            }],
            "stop_reason": serde_json::Value::Null,
            "stop_sequence": serde_json::Value::Null,
            "stop_details": serde_json::Value::Null,
            "context_management": serde_json::Value::Null,
            "usage": api_usage
                .map(|usage| stream_json_usage_value(usage, "not_available", false))
                .unwrap_or_else(|| stream_json_usage_value(&claude_core::types::usage::Usage::default(), "not_available", false)),
        },
        "parent_tool_use_id": serde_json::Value::Null,
        "session_id": session_id,
        "uuid": uuid::Uuid::new_v4().to_string(),
    })
}

#[derive(Default)]
struct StreamJsonPrintState {
    text: String,
    latest_usage: Option<claude_core::types::usage::Usage>,
}

fn merge_stream_usage(
    current: &mut Option<claude_core::types::usage::Usage>,
    update: claude_core::types::usage::Usage,
) {
    match current {
        Some(existing) => {
            if update.input_tokens > 0 {
                existing.input_tokens = update.input_tokens;
            }
            existing.output_tokens = existing.output_tokens.max(update.output_tokens);
            if update.cache_creation_input_tokens.is_some() {
                existing.cache_creation_input_tokens = update.cache_creation_input_tokens;
            }
            if update.cache_read_input_tokens.is_some() {
                existing.cache_read_input_tokens = update.cache_read_input_tokens;
            }
        }
        None => *current = Some(update),
    }
}

fn accumulate_stream_usage(
    current: &mut Option<claude_core::types::usage::Usage>,
    update: &claude_core::types::usage::Usage,
) {
    match current {
        Some(existing) => {
            existing.input_tokens += update.input_tokens;
            existing.output_tokens += update.output_tokens;
            existing.cache_creation_input_tokens = Some(
                existing.cache_creation_input_tokens.unwrap_or(0)
                    + update.cache_creation_input_tokens.unwrap_or(0),
            );
            existing.cache_read_input_tokens = Some(
                existing.cache_read_input_tokens.unwrap_or(0)
                    + update.cache_read_input_tokens.unwrap_or(0),
            );
        }
        None => *current = Some(update.clone()),
    }
}

struct StreamJsonResultMeta<'a> {
    duration_ms: u128,
    num_turns: u32,
    stop_reason: &'a str,
    total_usage: Option<&'a claude_core::types::usage::Usage>,
    latest_usage: Option<&'a claude_core::types::usage::Usage>,
    model_display: &'a str,
    max_tokens: u64,
    context_window: u64,
    total_cost_usd: f64,
}

enum PrintTerminalOutcome {
    Completed { stop_reason: String },
    MaxTurns { max_turns: u32, turn_count: u32 },
    MaxBudgetUsd { max_budget_usd: f64 },
    StructuredOutputRetries { max_retries: u32 },
}

fn stream_json_result_event_with_meta(
    final_text: &str,
    session_id: &str,
    meta: StreamJsonResultMeta<'_>,
) -> serde_json::Value {
    let usage = meta.total_usage.map(|total_usage| {
        let iteration_usage = meta.latest_usage.unwrap_or(total_usage);
        let iteration = stream_json_usage_value(iteration_usage, "", false);
        let mut usage_value = stream_json_usage_value(total_usage, "", true);
        usage_value["server_tool_use"] = serde_json::json!({
            "web_search_requests": 0,
            "web_fetch_requests": 0,
        });
        usage_value["iterations"] = serde_json::json!([{
            "input_tokens": iteration["input_tokens"].clone(),
            "output_tokens": iteration["output_tokens"].clone(),
            "cache_creation_input_tokens": iteration["cache_creation_input_tokens"].clone(),
            "cache_read_input_tokens": iteration["cache_read_input_tokens"].clone(),
            "cache_creation": iteration["cache_creation"].clone(),
            "type": "message",
        }]);
        usage_value
    });
    let model_usage = meta.total_usage.map(|usage| {
        let mut map = serde_json::Map::new();
        map.insert(
            normalize_model_display_for_stream_json(meta.model_display),
            serde_json::json!({
                "inputTokens": usage.input_tokens,
                "outputTokens": usage.output_tokens,
                "cacheReadInputTokens": usage.cache_read_input_tokens.unwrap_or(0),
                "cacheCreationInputTokens": usage.cache_creation_input_tokens.unwrap_or(0),
                "webSearchRequests": 0,
                "costUSD": meta.total_cost_usd,
                "contextWindow": meta.context_window,
                "maxOutputTokens": meta.max_tokens,
            }),
        );
        serde_json::Value::Object(map)
    });

    serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "api_error_status": serde_json::Value::Null,
        "duration_ms": meta.duration_ms,
        "duration_api_ms": meta.duration_ms,
        "num_turns": meta.num_turns,
        "result": final_text,
        "stop_reason": meta.stop_reason,
        "session_id": session_id,
        "total_cost_usd": meta.total_cost_usd,
        "usage": usage,
        "modelUsage": model_usage,
        "permission_denials": [],
        "terminal_reason": "completed",
        "fast_mode_state": "off",
        "uuid": uuid::Uuid::new_v4().to_string(),
    })
}

fn stream_json_max_turns_event_with_meta(
    max_turns: u32,
    session_id: &str,
    meta: StreamJsonResultMeta<'_>,
) -> serde_json::Value {
    let usage = meta.total_usage.map(|total_usage| {
        let iteration_usage = meta.latest_usage.unwrap_or(total_usage);
        let iteration = stream_json_usage_value(iteration_usage, "", false);
        let mut usage_value = stream_json_usage_value(total_usage, "", true);
        usage_value["server_tool_use"] = serde_json::json!({
            "web_search_requests": 0,
            "web_fetch_requests": 0,
        });
        usage_value["iterations"] = serde_json::json!([{
            "input_tokens": iteration["input_tokens"].clone(),
            "output_tokens": iteration["output_tokens"].clone(),
            "cache_creation_input_tokens": iteration["cache_creation_input_tokens"].clone(),
            "cache_read_input_tokens": iteration["cache_read_input_tokens"].clone(),
            "cache_creation": iteration["cache_creation"].clone(),
            "type": "message",
        }]);
        usage_value
    });
    let model_usage = meta.total_usage.map(|usage| {
        let mut map = serde_json::Map::new();
        map.insert(
            normalize_model_display_for_stream_json(meta.model_display),
            serde_json::json!({
                "inputTokens": usage.input_tokens,
                "outputTokens": usage.output_tokens,
                "cacheReadInputTokens": usage.cache_read_input_tokens.unwrap_or(0),
                "cacheCreationInputTokens": usage.cache_creation_input_tokens.unwrap_or(0),
                "webSearchRequests": 0,
                "costUSD": meta.total_cost_usd,
                "contextWindow": meta.context_window,
                "maxOutputTokens": meta.max_tokens,
            }),
        );
        serde_json::Value::Object(map)
    });

    serde_json::json!({
        "type": "result",
        "subtype": "error_max_turns",
        "is_error": true,
        "duration_ms": meta.duration_ms,
        "duration_api_ms": meta.duration_ms,
        "num_turns": meta.num_turns,
        "stop_reason": meta.stop_reason,
        "session_id": session_id,
        "total_cost_usd": meta.total_cost_usd,
        "usage": usage,
        "modelUsage": model_usage,
        "permission_denials": [],
        "terminal_reason": "max_turns",
        "fast_mode_state": "off",
        "uuid": uuid::Uuid::new_v4().to_string(),
        "errors": [format!("Reached maximum number of turns ({max_turns})")],
    })
}

fn stream_json_max_budget_usd_event_with_meta(
    max_budget_usd: f64,
    session_id: &str,
    meta: StreamJsonResultMeta<'_>,
) -> serde_json::Value {
    let usage = meta.total_usage.map(|total_usage| {
        let iteration_usage = meta.latest_usage.unwrap_or(total_usage);
        let iteration = stream_json_usage_value(iteration_usage, "", false);
        let mut usage_value = stream_json_usage_value(total_usage, "", true);
        usage_value["server_tool_use"] = serde_json::json!({
            "web_search_requests": 0,
            "web_fetch_requests": 0,
        });
        usage_value["iterations"] = serde_json::json!([{
            "input_tokens": iteration["input_tokens"].clone(),
            "output_tokens": iteration["output_tokens"].clone(),
            "cache_creation_input_tokens": iteration["cache_creation_input_tokens"].clone(),
            "cache_read_input_tokens": iteration["cache_read_input_tokens"].clone(),
            "cache_creation": iteration["cache_creation"].clone(),
            "type": "message",
        }]);
        usage_value
    });
    let model_usage = meta.total_usage.map(|usage| {
        let mut map = serde_json::Map::new();
        map.insert(
            normalize_model_display_for_stream_json(meta.model_display),
            serde_json::json!({
                "inputTokens": usage.input_tokens,
                "outputTokens": usage.output_tokens,
                "cacheReadInputTokens": usage.cache_read_input_tokens.unwrap_or(0),
                "cacheCreationInputTokens": usage.cache_creation_input_tokens.unwrap_or(0),
                "webSearchRequests": 0,
                "costUSD": meta.total_cost_usd,
                "contextWindow": meta.context_window,
                "maxOutputTokens": meta.max_tokens,
            }),
        );
        serde_json::Value::Object(map)
    });

    serde_json::json!({
        "type": "result",
        "subtype": "error_max_budget_usd",
        "is_error": true,
        "duration_ms": meta.duration_ms,
        "duration_api_ms": meta.duration_ms,
        "num_turns": meta.num_turns,
        "stop_reason": meta.stop_reason,
        "session_id": session_id,
        "total_cost_usd": meta.total_cost_usd,
        "usage": usage,
        "modelUsage": model_usage,
        "permission_denials": [],
        "fast_mode_state": "off",
        "uuid": uuid::Uuid::new_v4().to_string(),
        "errors": [format!("Reached maximum budget (${max_budget_usd})")],
    })
}

fn total_cost_for_usage(model: &str, usage: Option<&claude_core::types::usage::Usage>) -> f64 {
    let mut cost_tracker = claude_core::cost::tracker::CostTracker::new(model);
    if let Some(usage) = usage {
        cost_tracker.add_usage(usage);
    }
    cost_tracker.total_cost_usd()
}

fn stream_json_rate_limit_event(
    session_id: &str,
    limits: &claude_core::rate_limits::ClaudeAiLimits,
) -> serde_json::Value {
    serde_json::json!({
        "type": "rate_limit_event",
        "rate_limit_info": limits,
        "uuid": uuid::Uuid::new_v4().to_string(),
        "session_id": session_id,
    })
}

fn stream_json_status_event(session_id: &str, status: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "type": "system",
        "subtype": "status",
        "status": status,
        "session_id": session_id,
        "uuid": uuid::Uuid::new_v4().to_string(),
    })
}

struct StreamJsonInitMeta<'a> {
    cwd: &'a std::path::Path,
    session_id: &'a str,
    tool_names: Vec<String>,
    mcp_servers: Vec<serde_json::Value>,
    model_display: &'a str,
    permission_mode: &'a claude_core::permissions::types::PermissionMode,
    registered_skills: &'a [claude_tools::skill_tool::SkillEntry],
    discovered_skills: &'a [claude_core::plugins::types::Skill],
    stream_skill_names: &'a [String],
    mcp_prompt_commands: &'a [String],
    output_style: Option<&'a str>,
    auth: &'a claude_core::api::client::AuthMethod,
}

fn stream_json_init_event(meta: StreamJsonInitMeta<'_>) -> serde_json::Value {
    serde_json::json!({
        "type": "system",
        "subtype": "init",
        "cwd": meta.cwd.display().to_string(),
        "session_id": meta.session_id,
        "tools": meta.tool_names,
        "mcp_servers": meta.mcp_servers,
        "model": normalize_model_display_for_stream_json(meta.model_display),
        "permissionMode": serde_json::to_value(meta.permission_mode).unwrap_or(serde_json::json!("default")),
        "slash_commands": stream_json_slash_commands(meta.registered_skills, meta.discovered_skills, meta.mcp_prompt_commands),
        "apiKeySource": stream_json_api_key_source(meta.auth),
        "claude_code_version": env!("CARGO_PKG_VERSION"),
        "output_style": meta.output_style.unwrap_or("default"),
        "agents": stream_json_agent_names(meta.cwd),
        "skills": meta.stream_skill_names,
        "plugins": stream_json_plugin_entries(meta.cwd),
        "analytics_disabled": claude_core::privacy_level::is_telemetry_disabled(),
        "memory_paths": stream_json_memory_paths(meta.cwd),
        "fast_mode_state": "off",
        "uuid": uuid::Uuid::new_v4().to_string(),
    })
}

fn hook_execution_event_to_stream_json(
    event: claude_core::hooks::HookExecutionEvent,
    session_id: &str,
) -> serde_json::Value {
    match event {
        claude_core::hooks::HookExecutionEvent::Started {
            hook_id,
            hook_name,
            hook_event,
        } => serde_json::json!({
            "type": "system",
            "subtype": "hook_started",
            "hook_id": hook_id,
            "hook_name": hook_name,
            "hook_event": hook_event,
            "uuid": uuid::Uuid::new_v4().to_string(),
            "session_id": session_id,
        }),
        claude_core::hooks::HookExecutionEvent::Progress {
            hook_id,
            hook_name,
            hook_event,
            stdout,
            stderr,
            output,
        } => serde_json::json!({
            "type": "system",
            "subtype": "hook_progress",
            "hook_id": hook_id,
            "hook_name": hook_name,
            "hook_event": hook_event,
            "stdout": stdout,
            "stderr": stderr,
            "output": output,
            "uuid": uuid::Uuid::new_v4().to_string(),
            "session_id": session_id,
        }),
        claude_core::hooks::HookExecutionEvent::Response {
            hook_id,
            hook_name,
            hook_event,
            output,
            stdout,
            stderr,
            exit_code,
            outcome,
        } => serde_json::json!({
            "type": "system",
            "subtype": "hook_response",
            "hook_id": hook_id,
            "hook_name": hook_name,
            "hook_event": hook_event,
            "output": output,
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code,
            "outcome": outcome,
            "uuid": uuid::Uuid::new_v4().to_string(),
            "session_id": session_id,
        }),
    }
}

fn emit_stream_json_hook_execution_event(
    event: claude_core::hooks::HookExecutionEvent,
    session_id: &str,
) {
    emit_stream_json(hook_execution_event_to_stream_json(event, session_id));
}

fn is_remote_control_command(arg: &str) -> bool {
    matches!(arg, "remote-control" | "rc" | "remote" | "sync" | "bridge")
}

fn remote_control_help_text() -> &'static str {
    "\
Remote Control - Connect your local environment to claude.ai/code

USAGE
  claude remote-control [options]
OPTIONS
  --name <name>                    Name for the session (shown in claude.ai/code)
  --remote-control-session-name-prefix <prefix>
                                   Prefix for auto-generated session names
                                   (default: hostname; env:
                                   CLAUDE_REMOTE_CONTROL_SESSION_NAME_PREFIX)
  --permission-mode <mode>         Permission mode for spawned sessions
                                   (acceptEdits, auto, bypassPermissions, default, dontAsk, plan)
  --debug-file <path>              Write debug logs to file
  -v, --verbose                    Enable verbose output
  -h, --help                       Show this help
  --spawn <mode>                   Spawn mode: same-dir, worktree, session
                                   (default: same-dir)
  --capacity <N>                   Max concurrent sessions in worktree or
                                   same-dir mode (default: 32)
  --[no-]create-session-in-dir     Pre-create a session in the current
                                   directory; in worktree mode this session
                                   stays in cwd while on-demand sessions get
                                   isolated worktrees (default: on)

DESCRIPTION
  Remote Control allows you to control sessions on your local device from
  claude.ai/code (https://claude.ai/code). Run this command in the
  directory you want to work in, then connect from the Claude app or web.

  Remote Control runs as a persistent server that accepts multiple concurrent
  sessions in the current directory. One session is pre-created on start so
  you have somewhere to type immediately. Use --spawn=worktree to isolate
  each on-demand session in its own git worktree, or --spawn=session for
  the classic single-session mode (exits when that session ends). Press 'w'
  during runtime to toggle between same-dir and worktree.

NOTES
  - You must be logged in with a Claude account that has a subscription
  - Run `claude` first in the directory to accept the workspace trust dialog
  - Worktree mode requires a git repository or WorktreeCreate/WorktreeRemove hooks
"
}

fn normalize_remote_control_spawn(raw: Option<&str>) -> Result<Option<&'static str>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    match raw {
        "session" => Ok(Some("single-session")),
        "same-dir" => Ok(Some("same-dir")),
        "worktree" => Ok(Some("worktree")),
        value => Err(format!(
            "--spawn requires one of: session, same-dir, worktree (got: {value})"
        )),
    }
}

fn parse_remote_control_capacity(raw: Option<&str>) -> Result<Option<u32>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    match raw.parse::<u32>() {
        Ok(value) if value >= 1 => Ok(Some(value)),
        _ => Err(format!(
            "--capacity requires a positive integer (got: {raw})"
        )),
    }
}

fn validate_remote_control_bridge_options(
    spawn: Option<&str>,
    capacity: Option<&str>,
    session_id: Option<&str>,
    continue_session: bool,
    create_session_override: bool,
) -> Result<(Option<&'static str>, Option<u32>), String> {
    let spawn = normalize_remote_control_spawn(spawn)?;
    let capacity = parse_remote_control_capacity(capacity)?;
    if spawn == Some("single-session") && capacity.is_some() {
        return Err(
            "--capacity cannot be used with --spawn=session (single-session mode has fixed capacity 1)."
                .to_string(),
        );
    }
    if (session_id.is_some() || continue_session)
        && (spawn.is_some() || capacity.is_some() || create_session_override)
    {
        return Err(
            "--session-id and --continue cannot be used with --spawn, --capacity, or --create-session-in-dir."
                .to_string(),
        );
    }
    if session_id.is_some() && continue_session {
        return Err("--session-id and --continue cannot be used together.".to_string());
    }
    Ok((spawn, capacity))
}

fn require_remote_control_value<'a>(
    args: &'a [String],
    index: &mut usize,
    flag: &str,
) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index).map(|value| value.as_str()).ok_or_else(|| {
        format!("Unknown argument: {flag}\nRun 'claude remote-control --help' for usage.")
    })
}

fn validate_remote_control_fast_path_args(args: &[String]) -> Result<(), String> {
    let mut spawn: Option<String> = None;
    let mut capacity: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut continue_session = false;
    let mut create_session_override = false;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--help" || arg == "-h" {
            return Ok(());
        } else if arg == "--verbose"
            || arg == "-v"
            || arg == "--sandbox"
            || arg == "--no-sandbox"
            || arg == "--no-create-session-in-dir"
        {
            // Flag-only options.
        } else if arg == "--create-session-in-dir" {
            create_session_override = true;
        } else if arg == "--debug-file" {
            let _ = require_remote_control_value(args, &mut i, arg)?;
        } else if arg.starts_with("--debug-file=") {
            // Value carried by the same argument.
        } else if arg == "--session-timeout" {
            let _ = require_remote_control_value(args, &mut i, arg)?;
        } else if arg.starts_with("--session-timeout=") {
            // Value carried by the same argument.
        } else if arg == "--permission-mode" {
            let _ = require_remote_control_value(args, &mut i, arg)?;
        } else if arg.starts_with("--permission-mode=") {
            // Value carried by the same argument.
        } else if arg == "--name" {
            let _ = require_remote_control_value(args, &mut i, arg)?;
        } else if arg.starts_with("--name=") {
            // Value carried by the same argument.
        } else if arg == "--remote-control-session-name-prefix" {
            let _ = require_remote_control_value(args, &mut i, arg)?;
        } else if arg.starts_with("--remote-control-session-name-prefix=") {
            // Value carried by the same argument.
        } else if arg == "--session-id" {
            let value = require_remote_control_value(args, &mut i, arg)?;
            if value.is_empty() {
                return Err("--session-id requires a value".to_string());
            }
            session_id = Some(value.to_string());
        } else if let Some(value) = arg.strip_prefix("--session-id=") {
            if value.is_empty() {
                return Err("--session-id requires a value".to_string());
            }
            session_id = Some(value.to_string());
        } else if arg == "--continue" || arg == "-c" {
            continue_session = true;
        } else if arg == "--spawn" || arg.starts_with("--spawn=") {
            if spawn.is_some() {
                return Err("--spawn may only be specified once".to_string());
            }
            let raw = if let Some(value) = arg.strip_prefix("--spawn=") {
                value.to_string()
            } else {
                match args.get(i + 1) {
                    Some(value) => {
                        i += 1;
                        value.clone()
                    }
                    None => "<missing>".to_string(),
                }
            };
            spawn = Some(raw);
        } else if arg == "--capacity" || arg.starts_with("--capacity=") {
            if capacity.is_some() {
                return Err("--capacity may only be specified once".to_string());
            }
            let raw = if let Some(value) = arg.strip_prefix("--capacity=") {
                value.to_string()
            } else {
                match args.get(i + 1) {
                    Some(value) => {
                        i += 1;
                        value.clone()
                    }
                    None => "<missing>".to_string(),
                }
            };
            capacity = Some(raw);
        } else {
            return Err(format!(
                "Unknown argument: {arg}\nRun 'claude remote-control --help' for usage."
            ));
        }
        i += 1;
    }

    validate_remote_control_bridge_options(
        spawn.as_deref(),
        capacity.as_deref(),
        session_id.as_deref(),
        continue_session,
        create_session_override,
    )
    .map(|_| ())
}

fn remote_control_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn remote_control_git_branch() -> String {
    git_output(&["branch", "--show-current"]).unwrap_or_else(|| "HEAD".to_string())
}

fn remote_control_git_remote() -> Option<String> {
    git_output(&["remote", "get-url", "origin"])
}

fn remote_control_title(
    explicit_name: Option<&str>,
    prefix: Option<&str>,
    hostname: &str,
) -> Option<String> {
    explicit_name
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .or_else(|| {
            prefix
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("{}-{}", value.trim(), hostname))
        })
}

const BRIDGE_POINTER_TTL_MS: u128 = 4 * 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgePointer {
    session_id: String,
    environment_id: String,
    source: String,
}

fn claude_config_home_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
        .unwrap_or_else(|| PathBuf::from(".claude"))
}

fn bridge_pointer_path(dir: &std::path::Path) -> PathBuf {
    claude_config_home_dir()
        .join("projects")
        .join(claude_core::path_utils::sanitize_path(
            &dir.display().to_string(),
        ))
        .join("bridge-pointer.json")
}

fn write_bridge_pointer(dir: &std::path::Path, pointer: &BridgePointer) {
    let path = bridge_pointer_path(dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(pointer) {
        let _ = std::fs::write(path, text);
    }
}

fn clear_bridge_pointer(dir: &std::path::Path) {
    let _ = std::fs::remove_file(bridge_pointer_path(dir));
}

fn read_bridge_pointer(dir: &std::path::Path) -> Option<(BridgePointer, u128)> {
    let path = bridge_pointer_path(dir);
    let metadata = std::fs::metadata(&path).ok()?;
    let modified = metadata.modified().ok()?;
    let age_ms = std::time::SystemTime::now()
        .duration_since(modified)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let raw = std::fs::read_to_string(&path).ok()?;
    let pointer = serde_json::from_str::<BridgePointer>(&raw).ok()?;
    if pointer.session_id.is_empty() || pointer.environment_id.is_empty() {
        clear_bridge_pointer(dir);
        return None;
    }
    if age_ms > BRIDGE_POINTER_TTL_MS {
        clear_bridge_pointer(dir);
        return None;
    }
    Some((pointer, age_ms))
}

fn remote_control_worktree_paths(dir: &std::path::Path) -> Vec<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["worktree", "list", "--porcelain"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
}

fn read_bridge_pointer_across_worktrees(dir: &std::path::Path) -> Option<(BridgePointer, PathBuf)> {
    if let Some((pointer, _)) = read_bridge_pointer(dir) {
        return Some((pointer, dir.to_path_buf()));
    }
    let worktrees = remote_control_worktree_paths(dir);
    if worktrees.len() <= 1 || worktrees.len() > 50 {
        return None;
    }
    let mut freshest: Option<(BridgePointer, PathBuf, u128)> = None;
    for worktree in worktrees {
        if worktree == dir {
            continue;
        }
        let Some((pointer, age_ms)) = read_bridge_pointer(&worktree) else {
            continue;
        };
        if freshest.as_ref().is_none_or(|(_, _, age)| age_ms < *age) {
            freshest = Some((pointer, worktree, age_ms));
        }
    }
    freshest.map(|(pointer, dir, _)| (pointer, dir))
}

struct RemoteControlCommandOptions<'a> {
    name: Option<&'a String>,
    remote_control_session_name_prefix: Option<&'a String>,
    permission_mode: Option<&'a String>,
    debug_file: Option<&'a PathBuf>,
    sandbox: bool,
    no_sandbox: bool,
    session_timeout: Option<&'a String>,
    verbose: bool,
    spawn: Option<&'a String>,
    capacity: Option<&'a String>,
    session_id: Option<&'a String>,
    continue_session: bool,
    _create_session_in_dir: bool,
    no_create_session_in_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteSessionDoneStatus {
    Completed,
    Failed,
    Interrupted,
}

struct RemoteSessionHandle {
    session_id: String,
    pid: Option<u32>,
    stdin: Option<tokio::process::ChildStdin>,
    done: tokio::task::JoinHandle<RemoteSessionDoneStatus>,
    last_stderr: Arc<tokio::sync::Mutex<Vec<String>>>,
}

struct RemoteSessionSpawnOptions<'a> {
    session_id: &'a str,
    sdk_url: &'a str,
    access_token: &'a str,
    use_ccr_v2: bool,
    worker_epoch: Option<u64>,
    dir: &'a std::path::Path,
    verbose: bool,
    sandbox: bool,
    debug_file: Option<&'a str>,
    permission_mode: Option<&'a str>,
    capacity_wake: Option<claude_core::bridge::capacity_wake::CapacityWake>,
}

fn safe_filename_id(id: &str) -> String {
    id.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn debug_file_for_session(base: Option<&str>, session_id: &str, verbose: bool) -> Option<String> {
    let safe_id = safe_filename_id(session_id);
    if let Some(base) = base.filter(|value| !value.is_empty()) {
        if let Some((stem, ext)) = base.rsplit_once('.') {
            if !stem.is_empty() && !ext.contains(std::path::MAIN_SEPARATOR) {
                return Some(format!("{stem}-{safe_id}.{ext}"));
            }
        }
        return Some(format!("{base}-{safe_id}"));
    }
    if verbose || std::env::var("USER_TYPE").ok().as_deref() == Some("ant") {
        return Some(
            std::env::temp_dir()
                .join("claude")
                .join(format!("bridge-session-{safe_id}.log"))
                .display()
                .to_string(),
        );
    }
    None
}

fn child_stream_json_line_kind(line: &str) -> Option<&'static str> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    match value.get("type").and_then(|v| v.as_str()) {
        Some("assistant") => Some("assistant"),
        Some("result") => Some("result"),
        Some("error") => Some("error"),
        Some("control_request") => Some("control_request"),
        Some("system") => Some("system"),
        _ => None,
    }
}

async fn spawn_remote_session(
    options: RemoteSessionSpawnOptions<'_>,
) -> Result<RemoteSessionHandle> {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let exe = std::env::current_exe()?;
    let debug_file =
        debug_file_for_session(options.debug_file, options.session_id, options.verbose);
    let mut command = tokio::process::Command::new(exe);
    command
        .arg("--print")
        .arg("--sdk-url")
        .arg(options.sdk_url)
        .arg("--session-id")
        .arg(options.session_id)
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--replay-user-messages");
    if options.verbose {
        command.arg("--verbose");
    }
    if let Some(debug_file) = debug_file.as_deref() {
        command.arg("--debug-file").arg(debug_file);
    }
    if let Some(permission_mode) = options.permission_mode {
        command.arg("--permission-mode").arg(permission_mode);
    }

    command
        .current_dir(options.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
        .env("CLAUDE_CODE_ENVIRONMENT_KIND", "bridge")
        .env("CLAUDE_CODE_SESSION_ACCESS_TOKEN", options.access_token)
        .env("CLAUDE_CODE_POST_FOR_SESSION_INGRESS_V2", "1");
    if options.sandbox {
        command.env("CLAUDE_CODE_FORCE_SANDBOX", "1");
    }
    if options.use_ccr_v2 {
        command.env("CLAUDE_CODE_USE_CCR_V2", "1");
        if let Some(epoch) = options.worker_epoch {
            command.env("CLAUDE_CODE_WORKER_EPOCH", epoch.to_string());
        }
    }

    let mut child = command.spawn()?;
    let pid = child.id();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let session_id = options.session_id.to_string();
    let last_stderr = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    if let Some(stdout) = stdout {
        let sid = session_id.clone();
        let verbose = options.verbose;
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if verbose {
                    eprintln!("{line}");
                }
                if let Some(kind) = child_stream_json_line_kind(&line) {
                    eprintln!("Remote session {sid}: {kind}");
                }
            }
        });
    }

    if let Some(stderr) = stderr {
        let last = last_stderr.clone();
        let verbose = options.verbose;
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if verbose {
                    eprintln!("{line}");
                }
                let mut last = last.lock().await;
                if last.len() >= 10 {
                    last.remove(0);
                }
                last.push(line);
            }
        });
    }

    let capacity_wake = options.capacity_wake.clone();
    let done = tokio::spawn(async move {
        let result = async {
            let Ok(status) = child.wait().await else {
                return RemoteSessionDoneStatus::Failed;
            };
            if status.success() {
                return RemoteSessionDoneStatus::Completed;
            }
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if matches!(status.signal(), Some(15) | Some(2)) {
                    return RemoteSessionDoneStatus::Interrupted;
                }
            }
            RemoteSessionDoneStatus::Failed
        }
        .await;
        if let Some(wake) = capacity_wake {
            wake.wake();
        }
        result
    });

    Ok(RemoteSessionHandle {
        session_id,
        pid,
        stdin,
        done,
        last_stderr,
    })
}

async fn terminate_remote_session(handle: &RemoteSessionHandle, force: bool) {
    if let Some(pid) = handle.pid {
        let signal = if force { "-KILL" } else { "-TERM" };
        let _ = tokio::process::Command::new("kill")
            .arg(signal)
            .arg(pid.to_string())
            .status()
            .await;
    }
}

async fn update_remote_session_token(handle: &mut RemoteSessionHandle, token: &str) {
    use tokio::io::AsyncWriteExt;

    if let Some(stdin) = handle.stdin.as_mut() {
        let payload = serde_json::json!({
            "type": "update_environment_variables",
            "variables": {"CLAUDE_CODE_SESSION_ACCESS_TOKEN": token},
        });
        let _ = stdin.write_all(payload.to_string().as_bytes()).await;
        let _ = stdin.write_all(b"\n").await;
        let _ = stdin.flush().await;
    }
}

const REMOTE_TOKEN_REFRESH_BUFFER: std::time::Duration = std::time::Duration::from_secs(5 * 60);

fn remote_token_refresh_delay(
    token: &str,
    now: std::time::SystemTime,
) -> Option<std::time::Duration> {
    let expiry = claude_core::bridge::jwt_utils::decode_jwt_expiry(token)?;
    let expiry =
        std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(expiry as u64))?;
    let refresh_at = expiry
        .checked_sub(REMOTE_TOKEN_REFRESH_BUFFER)
        .unwrap_or(std::time::UNIX_EPOCH);
    Some(
        refresh_at
            .duration_since(now)
            .unwrap_or(std::time::Duration::ZERO),
    )
}

fn schedule_remote_token_refresh(
    tasks: &mut HashMap<String, tokio::task::JoinHandle<()>>,
    tx: &tokio::sync::mpsc::UnboundedSender<String>,
    session_id: &str,
    token: &str,
) {
    let Some(delay) = remote_token_refresh_delay(token, std::time::SystemTime::now()) else {
        return;
    };
    if let Some(task) = tasks.remove(session_id) {
        task.abort();
    }
    let session_id_owned = session_id.to_string();
    let tx = tx.clone();
    let task_session_id = session_id_owned.clone();
    let task = tokio::spawn(async move {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let _ = tx.send(task_session_id);
    });
    tasks.insert(session_id_owned, task);
}

async fn stop_work_with_retry(
    api: &claude_core::bridge::api::BridgeApiClient,
    environment_id: &str,
    work_id: &str,
    force: bool,
) {
    for attempt in 0..3 {
        match api.stop_work(environment_id, work_id, force).await {
            Ok(()) => return,
            Err(err) if attempt < 2 => {
                eprintln!("Failed to stop remote work {work_id}, retrying: {err}");
                tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            }
            Err(err) => {
                eprintln!("Failed to stop remote work {work_id}: {err}");
            }
        }
    }
}

async fn sleep_until_shutdown(
    shutdown: &tokio_util::sync::CancellationToken,
    duration: std::time::Duration,
) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => false,
        _ = tokio::time::sleep(duration) => true,
    }
}

async fn sleep_for_remote_capacity(
    wake: &claude_core::bridge::capacity_wake::CapacityWake,
    duration: std::time::Duration,
) -> Option<claude_core::bridge::capacity_wake::CapacityWakeReason> {
    wake.sleep_or_wake(duration).await
}

async fn run_remote_control_command(options: RemoteControlCommandOptions<'_>) -> Result<()> {
    let (spawn_mode_raw, capacity) = match validate_remote_control_bridge_options(
        options.spawn.map(String::as_str),
        options.capacity.map(String::as_str),
        options.session_id.map(String::as_str),
        options.continue_session,
        options._create_session_in_dir,
    ) {
        Ok(validated) => validated,
        Err(message) => {
            eprintln!("Error: {message}");
            std::process::exit(1);
        }
    };
    let spawn_mode = match spawn_mode_raw.unwrap_or("same-dir") {
        "single-session" => claude_core::bridge::api::SpawnMode::SingleSession,
        "worktree" => claude_core::bridge::api::SpawnMode::Worktree,
        _ => claude_core::bridge::api::SpawnMode::SameDir,
    };
    let max_sessions = if options.session_id.is_some()
        || options.continue_session
        || matches!(
            spawn_mode,
            claude_core::bridge::api::SpawnMode::SingleSession
        ) {
        1
    } else {
        capacity.unwrap_or(32)
    };

    let tokens = claude_core::auth::storage::load_tokens().await?;
    let Some(tokens) = tokens.filter(|tokens| !tokens.access_token.trim().is_empty()) else {
        eprintln!("{}", claude_core::bridge::api::BRIDGE_LOGIN_INSTRUCTION);
        std::process::exit(1);
    };

    let global = claude_core::config::global::load_global_config()?;
    let Some(organization_uuid) = global
        .oauth_account
        .as_ref()
        .and_then(|account| account.organization_uuid.clone())
        .filter(|value| !value.is_empty())
    else {
        eprintln!(
            "Unable to determine your organization for Remote Control eligibility. Run `claude login` to refresh your account information."
        );
        std::process::exit(1);
    };

    let oauth = claude_core::constants::oauth::get_oauth_config()
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let http = claude_core::proxy::build_proxy_client_from_env()?;
    let api = claude_core::bridge::api::BridgeApiClient::new(
        http.clone(),
        oauth.base_api_url.clone(),
        Some(tokens.access_token.clone()),
        env!("CARGO_PKG_VERSION"),
    )
    .with_trusted_device_token(
        claude_core::bridge::trusted_device::get_trusted_device_token().await,
    );

    let cwd = std::env::current_dir()?;
    let mut resume_pointer_dir: Option<PathBuf> = None;
    let resume_session_id = if options.continue_session {
        let Some((pointer, pointer_dir)) = read_bridge_pointer_across_worktrees(&cwd) else {
            eprintln!("Error: No resumable Remote Control session found for this directory.");
            std::process::exit(1);
        };
        resume_pointer_dir = Some(pointer_dir);
        Some(pointer.session_id)
    } else {
        options.session_id.cloned()
    };
    let hostname = remote_control_hostname();
    let branch = remote_control_git_branch();
    let git_repo_url = remote_control_git_remote();
    let bridge_id = uuid::Uuid::new_v4().to_string();
    let environment_id = uuid::Uuid::new_v4().to_string();
    let title = remote_control_title(
        options.name.map(String::as_str),
        options
            .remote_control_session_name_prefix
            .map(String::as_str),
        &hostname,
    );
    let session_timeout_ms = options
        .session_timeout
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1000));
    let pre_create_session = resume_session_id.is_none() && !options.no_create_session_in_dir;
    let mut reuse_environment_id = None;
    if let Some(resume_session_id) = resume_session_id.as_deref() {
        let session = api
            .get_bridge_session(resume_session_id, &organization_uuid)
            .await
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        let Some(session) = session else {
            if let Some(dir) = resume_pointer_dir.as_deref() {
                clear_bridge_pointer(dir);
            }
            eprintln!(
                "Error: Session {resume_session_id} not found. It may have been archived or expired, or your login may have lapsed (run `claude /login`)."
            );
            std::process::exit(1);
        };
        let Some(environment_id) = session.environment_id.filter(|value| !value.is_empty()) else {
            if let Some(dir) = resume_pointer_dir.as_deref() {
                clear_bridge_pointer(dir);
            }
            eprintln!(
                "Error: Session {resume_session_id} has no environment_id. It may never have been attached to a bridge."
            );
            std::process::exit(1);
        };
        reuse_environment_id = Some(environment_id);
    }

    let config = claude_core::bridge::api::BridgeRuntimeConfig {
        dir: cwd.display().to_string(),
        machine_name: hostname.clone(),
        branch: branch.clone(),
        git_repo_url: git_repo_url.clone(),
        max_sessions,
        spawn_mode,
        verbose: options.verbose,
        sandbox: options.sandbox && !options.no_sandbox,
        bridge_id,
        worker_type: "claude_code".to_string(),
        environment_id,
        reuse_environment_id: reuse_environment_id.clone(),
        api_base_url: oauth.base_api_url.clone(),
        session_ingress_url: oauth.base_api_url.clone(),
        debug_file: options.debug_file.map(|path| path.display().to_string()),
        session_timeout_ms,
    };

    let (registered_environment_id, environment_secret) = api
        .register_bridge_environment(&config)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    let mut session_id = None;
    if let Some(resume_session_id) = resume_session_id.as_deref() {
        if reuse_environment_id.as_deref() != Some(registered_environment_id.as_str()) {
            eprintln!(
                "Warning: Could not resume session {resume_session_id} - its environment has expired. Creating a fresh session instead."
            );
            session_id = api
                .create_bridge_session(&claude_core::bridge::api::CreateBridgeSessionRequest {
                    environment_id: registered_environment_id.clone(),
                    organization_uuid: organization_uuid.clone(),
                    title: title.clone(),
                    events: Vec::new(),
                    git_repo_url,
                    branch,
                    model: default_main_loop_model_setting().await,
                    permission_mode: options.permission_mode.cloned(),
                })
                .await
                .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        } else {
            let infra_id =
                claude_core::bridge::session_id_compat::to_infra_session_id(resume_session_id);
            let mut candidates = vec![resume_session_id.to_string()];
            if infra_id != resume_session_id {
                candidates.push(infra_id);
            }
            let mut reconnected = false;
            let mut last_error = None;
            for candidate in candidates {
                match api
                    .reconnect_session(&registered_environment_id, &candidate)
                    .await
                {
                    Ok(()) => {
                        reconnected = true;
                        break;
                    }
                    Err(err) => last_error = Some(err.to_string()),
                }
            }
            if !reconnected {
                eprintln!(
                    "Error: Failed to reconnect session {resume_session_id}: {}\nThe session may still be resumable - try running the same command again.",
                    last_error.unwrap_or_else(|| "unknown error".to_string())
                );
                std::process::exit(1);
            }
            session_id = Some(resume_session_id.to_string());
        }
    } else if pre_create_session {
        session_id = api
            .create_bridge_session(&claude_core::bridge::api::CreateBridgeSessionRequest {
                environment_id: registered_environment_id.clone(),
                organization_uuid: organization_uuid.clone(),
                title: title.clone(),
                events: Vec::new(),
                git_repo_url,
                branch,
                model: default_main_loop_model_setting().await,
                permission_mode: options.permission_mode.cloned(),
            })
            .await
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    }
    if matches!(
        config.spawn_mode,
        claude_core::bridge::api::SpawnMode::SingleSession
    ) {
        if let Some(session_id) = session_id.as_deref() {
            write_bridge_pointer(
                &cwd,
                &BridgePointer {
                    session_id: session_id.to_string(),
                    environment_id: registered_environment_id.clone(),
                    source: "standalone".to_string(),
                },
            );
        }
    }

    println!("Remote Control is active.");
    println!("Environment: {registered_environment_id}");
    if let Some(session_id) = session_id.as_deref() {
        let compat_id =
            claude_core::bridge::session_id_compat::to_compat_session_id(session_id, true);
        println!(
            "Code in CLI or at {}",
            claude_core::constants::product::get_remote_session_url(
                &compat_id,
                Some(&config.session_ingress_url),
            )
        );
    } else {
        println!("No session was pre-created because --no-create-session-in-dir was set.");
    }
    println!("Press Ctrl-C to stop.");

    let mut active_sessions: HashMap<String, RemoteSessionHandle> = HashMap::new();
    let mut session_work_ids: HashMap<String, String> = HashMap::new();
    let mut session_tokens: HashMap<String, String> = HashMap::new();
    let mut completed_work_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut v2_sessions: std::collections::HashSet<String> = std::collections::HashSet::new();
    let (token_refresh_tx, mut token_refresh_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut token_refresh_tasks: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut known_sessions: Vec<String> = session_id.clone().into_iter().collect();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let shutdown_signal = shutdown.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        shutdown_signal.cancel();
    });
    let capacity_wake = claude_core::bridge::capacity_wake::CapacityWake::new(shutdown.clone());

    while !shutdown.is_cancelled() {
        let finished = active_sessions
            .iter()
            .filter_map(|(session_id, handle)| {
                handle.done.is_finished().then_some(session_id.clone())
            })
            .collect::<Vec<_>>();
        for finished_id in finished {
            if let Some(handle) = active_sessions.remove(&finished_id) {
                let work_id = session_work_ids.remove(&finished_id);
                session_tokens.remove(&finished_id);
                v2_sessions.remove(&finished_id);
                if let Some(task) = token_refresh_tasks.remove(&finished_id) {
                    task.abort();
                }
                let stderr = handle.last_stderr.clone();
                let status = handle.done.await.unwrap_or(RemoteSessionDoneStatus::Failed);
                match status {
                    RemoteSessionDoneStatus::Completed => {
                        println!("Remote session {} completed.", handle.session_id);
                    }
                    RemoteSessionDoneStatus::Interrupted => {
                        println!("Remote session {} interrupted.", handle.session_id);
                    }
                    RemoteSessionDoneStatus::Failed => {
                        let stderr = stderr.lock().await.join("\n");
                        eprintln!(
                            "Remote session {} failed{}",
                            handle.session_id,
                            if stderr.is_empty() {
                                String::new()
                            } else {
                                format!(": {stderr}")
                            }
                        );
                    }
                }
                if status != RemoteSessionDoneStatus::Interrupted {
                    if let Some(work_id) = work_id {
                        completed_work_ids.insert(work_id.clone());
                        stop_work_with_retry(&api, &registered_environment_id, &work_id, false)
                            .await;
                    }
                    let compat_id = claude_core::bridge::session_id_compat::to_compat_session_id(
                        &finished_id,
                        true,
                    );
                    let _ = api.archive_session(&compat_id).await;
                }
            }
        }

        let poll_config = claude_core::bridge::poll_config::get_poll_interval_config();
        let poll = api.poll_for_work(
            &registered_environment_id,
            &environment_secret,
            Some(poll_config.reclaim_older_than_ms),
        );
        tokio::select! {
            _ = shutdown.cancelled() => {
                println!("Remote Control shutting down.");
                break;
            }
            maybe_session_id = token_refresh_rx.recv() => {
                let Some(refresh_session_id) = maybe_session_id else {
                    continue;
                };
                if !active_sessions.contains_key(&refresh_session_id) {
                    continue;
                }
                if v2_sessions.contains(&refresh_session_id) {
                    if let Err(err) = api
                        .reconnect_session(&registered_environment_id, &refresh_session_id)
                        .await
                    {
                        eprintln!(
                            "Failed to refresh remote session {refresh_session_id} token via reconnect: {err}"
                        );
                    }
                } else if let Some(handle) = active_sessions.get_mut(&refresh_session_id) {
                    match claude_core::auth::resolve::resolve_stored_oauth_token(true).await {
                        Ok(Some(oauth_token)) => {
                            update_remote_session_token(handle, &oauth_token).await;
                        }
                        Ok(None) => {
                            eprintln!(
                                "Failed to refresh remote session {refresh_session_id} token: no OAuth token available"
                            );
                        }
                        Err(err) => {
                            eprintln!(
                                "Failed to refresh remote session {refresh_session_id} token: {err}"
                            );
                        }
                    }
                }
            }
            result = poll => {
                match result {
                    Ok(Some(work)) => {
                        if completed_work_ids.contains(&work.id) {
                            if options.verbose {
                                eprintln!("Skipping already-completed remote work {}.", work.id);
                            }
                            if active_sessions.len() >= config.max_sessions as usize {
                                let at_capacity_ms =
                                    poll_config.multisession_poll_interval_ms_at_capacity;
                                if poll_config.non_exclusive_heartbeat_interval_ms > 0 {
                                    for (session_id, work_id) in session_work_ids.clone() {
                                        if let Some(token) = session_tokens.get(&session_id) {
                                            let _ = api
                                                .heartbeat_work(
                                                    &registered_environment_id,
                                                    &work_id,
                                                    token,
                                                )
                                                .await;
                                        }
                                    }
                                    let _ = sleep_for_remote_capacity(
                                        &capacity_wake,
                                        std::time::Duration::from_millis(
                                            poll_config.non_exclusive_heartbeat_interval_ms,
                                        ),
                                    )
                                    .await;
                                } else if at_capacity_ms > 0 {
                                    let _ = sleep_for_remote_capacity(
                                        &capacity_wake,
                                        std::time::Duration::from_millis(at_capacity_ms),
                                    )
                                    .await;
                                }
                            } else {
                                let _ = sleep_until_shutdown(
                                    &shutdown,
                                    std::time::Duration::from_secs(1),
                                )
                                .await;
                            }
                            continue;
                        }

                        let secret = match claude_core::bridge::work_secret::decode_work_secret(&work.secret) {
                            Ok(secret) => secret,
                            Err(err) => {
                                eprintln!("Failed to decode remote work secret for {}: {err}", work.id);
                                completed_work_ids.insert(work.id.clone());
                                stop_work_with_retry(&api, &registered_environment_id, &work.id, false).await;
                                continue;
                            }
                        };

                        let ack = async {
                            let _ = api
                                .acknowledge_work(
                                    &registered_environment_id,
                                    &work.id,
                                    &secret.session_ingress_token,
                                )
                                .await;
                        };

                        match work.data.kind.as_str() {
                            "healthcheck" => {
                                ack.await;
                                if options.verbose {
                                    println!("Healthcheck received.");
                                }
                            }
                            "session" => {
                                let remote_session_id = work.data.id.clone();
                                if let Err(err) = claude_core::bridge::api::validate_bridge_id(
                                    &remote_session_id,
                                    "session_id",
                                ) {
                                    ack.await;
                                    eprintln!("Invalid remote session id {remote_session_id}: {err}");
                                    continue;
                                }

                                if let Some(existing) = active_sessions.get_mut(&remote_session_id) {
                                    update_remote_session_token(existing, &secret.session_ingress_token).await;
                                    session_tokens.insert(remote_session_id.clone(), secret.session_ingress_token.clone());
                                    session_work_ids.insert(remote_session_id.clone(), work.id.clone());
                                    schedule_remote_token_refresh(
                                        &mut token_refresh_tasks,
                                        &token_refresh_tx,
                                        &remote_session_id,
                                        &secret.session_ingress_token,
                                    );
                                    ack.await;
                                    continue;
                                }

                                if active_sessions.len() >= config.max_sessions as usize {
                                    if options.verbose {
                                        eprintln!(
                                            "Remote Control at capacity ({}/{}), leaving work {} pending.",
                                            active_sessions.len(),
                                            config.max_sessions,
                                            work.id
                                        );
                                    }
                                    let at_capacity_ms = poll_config
                                        .multisession_poll_interval_ms_at_capacity;
                                    if poll_config.non_exclusive_heartbeat_interval_ms > 0 {
                                        for (session_id, work_id) in session_work_ids.clone() {
                                            if let Some(token) = session_tokens.get(&session_id) {
                                                let _ = api
                                                    .heartbeat_work(
                                                        &registered_environment_id,
                                                        &work_id,
                                                        token,
                                                    )
                                                    .await;
                                            }
                                        }
                                        let _ = sleep_for_remote_capacity(
                                            &capacity_wake,
                                            std::time::Duration::from_millis(
                                                poll_config
                                                    .non_exclusive_heartbeat_interval_ms,
                                            ),
                                        )
                                        .await;
                                    } else if at_capacity_ms > 0 {
                                        let _ = sleep_for_remote_capacity(
                                            &capacity_wake,
                                            std::time::Duration::from_millis(at_capacity_ms),
                                        )
                                        .await;
                                    }
                                    continue;
                                }

                                ack.await;

                                let use_ccr_v2 = secret
                                    .extra
                                    .get("use_code_sessions")
                                    .and_then(|value| value.as_bool())
                                    .unwrap_or(false)
                                    || claude_core::errors_util::is_env_truthy("CLAUDE_BRIDGE_USE_CCR_V2");
                                let mut worker_epoch = None;
                                let sdk_url = if use_ccr_v2 {
                                    let url = claude_core::bridge::work_secret::build_ccr_v2_sdk_url(
                                        &config.api_base_url,
                                        &remote_session_id,
                                    );
                                    match claude_core::bridge::work_secret::register_worker(
                                        &http,
                                        &url,
                                        &secret.session_ingress_token,
                                    )
                                    .await
                                    {
                                        Ok(epoch) => {
                                            worker_epoch = Some(epoch);
                                            url
                                        }
                                        Err(err) => {
                                            eprintln!(
                                                "Failed to register remote worker for {remote_session_id}: {err}"
                                            );
                                            completed_work_ids.insert(work.id.clone());
                                            stop_work_with_retry(
                                                &api,
                                                &registered_environment_id,
                                                &work.id,
                                                false,
                                            )
                                            .await;
                                            continue;
                                        }
                                    }
                                } else {
                                    claude_core::bridge::work_secret::build_sdk_url(
                                        &config.session_ingress_url,
                                        &remote_session_id,
                                    )
                                };

                                match spawn_remote_session(RemoteSessionSpawnOptions {
                                    session_id: &remote_session_id,
                                    sdk_url: &sdk_url,
                                    access_token: &secret.session_ingress_token,
                                    use_ccr_v2,
                                    worker_epoch,
                                    dir: &cwd,
                                    verbose: options.verbose,
                                    sandbox: config.sandbox,
                                    debug_file: config.debug_file.as_deref(),
                                    permission_mode: options.permission_mode.map(String::as_str),
                                    capacity_wake: Some(capacity_wake.clone()),
                                })
                                .await
                                {
                                    Ok(handle) => {
                                        println!("Remote session {remote_session_id} attached.");
                                        if !known_sessions.iter().any(|id| id == &remote_session_id) {
                                            known_sessions.push(remote_session_id.clone());
                                        }
                                        if use_ccr_v2 {
                                            v2_sessions.insert(remote_session_id.clone());
                                        }
                                        schedule_remote_token_refresh(
                                            &mut token_refresh_tasks,
                                            &token_refresh_tx,
                                            &remote_session_id,
                                            &secret.session_ingress_token,
                                        );
                                        session_work_ids.insert(remote_session_id.clone(), work.id.clone());
                                        session_tokens.insert(remote_session_id.clone(), secret.session_ingress_token);
                                        active_sessions.insert(remote_session_id, handle);
                                    }
                                    Err(err) => {
                                        eprintln!("Failed to spawn remote session {remote_session_id}: {err}");
                                        completed_work_ids.insert(work.id.clone());
                                        stop_work_with_retry(
                                            &api,
                                            &registered_environment_id,
                                            &work.id,
                                            false,
                                        )
                                        .await;
                                    }
                                }
                            }
                            other => {
                                ack.await;
                                if options.verbose {
                                    eprintln!("Ignoring unknown remote work type: {other}");
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        if active_sessions.len() >= config.max_sessions as usize {
                            let at_capacity_ms =
                                poll_config.multisession_poll_interval_ms_at_capacity;
                            if poll_config.non_exclusive_heartbeat_interval_ms > 0 {
                                let poll_deadline = (at_capacity_ms > 0).then(|| {
                                    tokio::time::Instant::now()
                                        + std::time::Duration::from_millis(at_capacity_ms)
                                });
                                while !shutdown.is_cancelled()
                                    && active_sessions.len() >= config.max_sessions as usize
                                    && poll_deadline.is_none_or(|deadline| {
                                        tokio::time::Instant::now() < deadline
                                    })
                                {
                                    let hb_config =
                                        claude_core::bridge::poll_config::get_poll_interval_config();
                                    if hb_config.non_exclusive_heartbeat_interval_ms == 0 {
                                        break;
                                    }
                                    for (session_id, work_id) in session_work_ids.clone() {
                                        if let Some(token) = session_tokens.get(&session_id) {
                                            let _ = api
                                                .heartbeat_work(
                                                    &registered_environment_id,
                                                    &work_id,
                                                    token,
                                                )
                                                .await;
                                        }
                                    }
                                    if sleep_for_remote_capacity(
                                        &capacity_wake,
                                        std::time::Duration::from_millis(
                                            hb_config.non_exclusive_heartbeat_interval_ms,
                                        ),
                                    )
                                    .await
                                    .is_some()
                                    {
                                        break;
                                    }
                                }
                            } else if at_capacity_ms > 0 {
                                let _ = sleep_for_remote_capacity(
                                    &capacity_wake,
                                    std::time::Duration::from_millis(at_capacity_ms),
                                )
                                .await;
                            }
                        } else {
                            let interval = if active_sessions.is_empty() {
                                poll_config.multisession_poll_interval_ms_not_at_capacity
                            } else {
                                poll_config.multisession_poll_interval_ms_partial_capacity
                            };
                            let _ = sleep_until_shutdown(
                                &shutdown,
                                std::time::Duration::from_millis(interval),
                            )
                            .await;
                        }
                    }
                    Err(err) => {
                        eprintln!("Remote Control poll failed: {err}");
                        let _ = sleep_until_shutdown(
                            &shutdown,
                            std::time::Duration::from_secs(5),
                        )
                        .await;
                    }
                }
            }
        }
    }

    for (_, handle) in active_sessions.iter() {
        terminate_remote_session(handle, false).await;
    }
    for (_, task) in token_refresh_tasks {
        task.abort();
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    for (_, handle) in active_sessions.iter() {
        terminate_remote_session(handle, true).await;
    }
    for (_, work_id) in session_work_ids {
        stop_work_with_retry(&api, &registered_environment_id, &work_id, true).await;
    }
    for session_id in known_sessions {
        let compat_id =
            claude_core::bridge::session_id_compat::to_compat_session_id(&session_id, true);
        let _ = api.archive_session(&compat_id).await;
    }
    clear_bridge_pointer(&cwd);
    let _ = api.deregister_environment(&registered_environment_id).await;
    Ok(())
}

fn maybe_handle_remote_control_fast_path_args() {
    let mut args = std::env::args().skip(1);
    let Some(first) = args.next() else {
        return;
    };
    if !is_remote_control_command(&first) {
        return;
    }
    let rest = args.collect::<Vec<_>>();
    if rest.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{}", remote_control_help_text());
        std::process::exit(0);
    }
    if let Err(message) = validate_remote_control_fast_path_args(&rest) {
        eprintln!("Error: {message}");
        std::process::exit(1);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    maybe_handle_remote_control_fast_path_args();
    let (rewritten_args, pending_direct_connect, pending_direct_skip_permissions) =
        rewrite_direct_connect_args(std::env::args())?;
    let mut cli = Cli::parse_from(rewritten_args);
    if cli.bare {
        std::env::set_var("CLAUDE_CODE_SIMPLE", "1");
    }
    if cli.input_format == InputFormat::StreamJson && cli.output_format != OutputFormat::StreamJson
    {
        eprintln!("Error: --input-format=stream-json requires output-format=stream-json.");
        std::process::exit(1);
    }
    if cli.replay_user_messages
        && (cli.input_format != InputFormat::StreamJson
            || cli.output_format != OutputFormat::StreamJson)
    {
        eprintln!(
            "Error: --replay-user-messages requires both --input-format=stream-json and --output-format=stream-json."
        );
        std::process::exit(1);
    }
    let include_partial_messages = cli.include_partial_messages
        || claude_core::errors_util::is_env_truthy("CLAUDE_CODE_INCLUDE_PARTIAL_MESSAGES");
    if include_partial_messages && (!cli.print || cli.output_format != OutputFormat::StreamJson) {
        eprintln!(
            "Error: --include-partial-messages requires --print and --output-format=stream-json."
        );
        std::process::exit(1);
    }
    if matches!(cli.task_budget, Some(0)) {
        eprintln!("error: invalid value '0' for '--task-budget <TASK_BUDGET>': --task-budget must be a positive integer");
        std::process::exit(2);
    }
    if let Some(max_budget_usd) = cli.max_budget_usd {
        if !max_budget_usd.is_finite() || max_budget_usd <= 0.0 {
            eprintln!(
                "error: invalid value '{max_budget_usd}' for '--max-budget-usd <MAX_BUDGET_USD>': --max-budget-usd must be a positive number greater than 0"
            );
            std::process::exit(2);
        }
    }
    let mut prompt_arg = cli.prompt.clone();
    let mut prompt_content: Option<Value> = None;
    let mut prompt_delivery_event_id: Option<String> = None;
    let mut initial_remote_control_requests = Vec::new();
    let mut restored_remote_worker_metadata: Option<Value> = None;
    let mut remote_sdk_rx = None;
    if cli.print && prompt_arg.is_none() {
        if let Some(sdk_url) = cli.sdk_url.clone() {
            let remote = read_remote_sdk_seed(&sdk_url).await?;
            let seed = remote.seed;
            remote_sdk_rx = Some(remote.rx);
            if prompt_arg.is_none() {
                prompt_arg = seed.prompt;
                prompt_content = seed.prompt_content;
                prompt_delivery_event_id = seed.prompt_delivery_event_id;
            }
            initial_remote_control_requests = seed.control_requests;
            restored_remote_worker_metadata = seed.restored_worker_metadata;
            if seed.system_prompt.is_some() {
                cli.system_prompt = seed.system_prompt;
                cli.system_prompt_file = None;
            }
            if seed.append_system_prompt.is_some() {
                cli.append_system_prompt = seed.append_system_prompt;
                cli.append_system_prompt_file = None;
            }
            if seed.json_schema.is_some() {
                cli.json_schema = seed.json_schema;
            }
        } else {
            use std::io::Read;
            let mut stdin = String::new();
            std::io::stdin().read_to_string(&mut stdin)?;
            match cli.input_format {
                InputFormat::Text => {
                    let stdin = stdin.trim_end().to_string();
                    if !stdin.is_empty() {
                        prompt_arg = Some(stdin);
                    }
                }
                InputFormat::StreamJson => {
                    let seed = parse_stream_json_stdin(&stdin)?;
                    if prompt_arg.is_none() {
                        prompt_arg = seed.prompt;
                    }
                    initial_remote_control_requests = seed.control_requests;
                    restored_remote_worker_metadata = seed.restored_worker_metadata;
                    if seed.system_prompt.is_some() {
                        cli.system_prompt = seed.system_prompt;
                        cli.system_prompt_file = None;
                    }
                    if seed.append_system_prompt.is_some() {
                        cli.append_system_prompt = seed.append_system_prompt;
                        cli.append_system_prompt_file = None;
                    }
                    if seed.json_schema.is_some() {
                        cli.json_schema = seed.json_schema;
                    }
                }
            }
        }
    }

    if cli.print && cli.sdk_url.is_some() && prompt_arg.is_none() {
        prompt_arg = Some(String::new());
    }

    // Set working directory if specified
    if let Some(dir) = &cli.working_dir {
        std::env::set_current_dir(dir)?;
    }
    if let Some(metadata) = restored_remote_worker_metadata.as_ref() {
        apply_restored_worker_metadata_to_cli(&mut cli, metadata);
    }
    let is_interactive_session = prompt_arg.is_none();
    initialize_entrypoint(is_interactive_session);

    // Initialize tracing. Stream-json stdout must remain valid JSONL; TS does
    // not interleave normal log lines with stream-json records.
    let trace_filter = if cli.output_format == OutputFormat::StreamJson && prompt_arg.is_some() {
        "off"
    } else if cli.verbose {
        "debug"
    } else {
        "error"
    };
    tracing_subscriber::fmt()
        .with_env_filter(trace_filter)
        .init();

    // Handle subcommands
    match &cli.command {
        Some(SubCommand::Login {
            email,
            sso,
            console,
            claudeai,
        }) => {
            let opts = claude_core::auth::login::LoginOptions {
                email: email.clone(),
                sso: *sso,
                use_console: *console,
                use_claude_ai: *claudeai || !*console, // Default to Claude.ai if neither flag
            };
            claude_core::auth::login::login_with_options(opts).await?;
            return Ok(());
        }
        Some(SubCommand::Logout) => {
            claude_core::bridge::trusted_device::clear_trusted_device_token().await;
            // Delete tokens from secure storage (keychain + file)
            claude_core::auth::storage::delete_tokens().await.ok();
            claude_core::auth::storage::delete_managed_api_key()
                .await
                .ok();
            // Clear account info from global config
            claude_core::config::global::save_global_config(|mut config| {
                config.oauth_account = None;
                config.primary_api_key = None;
                config
            })
            .ok();
            println!("Successfully logged out from your Anthropic account.");
            return Ok(());
        }
        Some(SubCommand::Config) => {
            let cwd = std::env::current_dir()?;
            let root = claude_core::config::paths::detect_project_root(&cwd);
            println!("Project root: {}", root.display());
            println!(
                "Config dir: {}",
                claude_core::config::paths::claude_dir()?.display()
            );
            return Ok(());
        }
        Some(SubCommand::Mcp { command }) => {
            handle_mcp_command(command).await?;
            return Ok(());
        }
        Some(SubCommand::Server { port }) => {
            // Start the IDE bridge server
            let config = claude_core::bridge::types::BridgeConfig {
                port: *port,
                host: "127.0.0.1".to_string(),
                ide: claude_core::bridge::types::IdeType::Other("cli".to_string()),
            };
            let server = claude_core::bridge::server::BridgeServer::new(config);
            tracing::info!("Starting IDE bridge server...");
            server.start().await?;
            return Ok(());
        }
        Some(SubCommand::RemoteControl {
            name,
            remote_control_session_name_prefix,
            permission_mode,
            debug_file,
            sandbox,
            no_sandbox,
            session_timeout,
            verbose,
            spawn,
            capacity,
            session_id,
            continue_session,
            create_session_in_dir,
            no_create_session_in_dir,
        }) => {
            run_remote_control_command(RemoteControlCommandOptions {
                name: name.as_ref(),
                remote_control_session_name_prefix: remote_control_session_name_prefix.as_ref(),
                permission_mode: permission_mode.as_ref(),
                debug_file: debug_file.as_ref(),
                sandbox: *sandbox,
                no_sandbox: *no_sandbox,
                session_timeout: session_timeout.as_ref(),
                verbose: *verbose,
                spawn: spawn.as_ref(),
                capacity: capacity.as_ref(),
                session_id: session_id.as_ref(),
                continue_session: *continue_session,
                _create_session_in_dir: *create_session_in_dir,
                no_create_session_in_dir: *no_create_session_in_dir,
            })
            .await?;
            return Ok(());
        }
        Some(SubCommand::Open {
            cc_url,
            print,
            output_format,
        }) => {
            let parsed = parse_direct_connect_url(cc_url)?;
            let session = claude_core::bridge::direct_connect::create_direct_connect_session(
                &reqwest::Client::new(),
                &parsed.server_url,
                parsed.auth_token.as_deref(),
                &std::env::current_dir()?.display().to_string(),
                cli.dangerously_skip_permissions || pending_direct_skip_permissions,
            )
            .await
            .map_err(|err| anyhow::anyhow!("{err}"))?;
            if let Some(work_dir) = session.work_dir.as_deref() {
                std::env::set_current_dir(work_dir)?;
            }
            let prompt = print
                .as_ref()
                .and_then(|value| value.as_ref())
                .cloned()
                .unwrap_or_default();
            run_direct_connect_headless(session.config, prompt, output_format.clone()).await?;
            return Ok(());
        }
        None => {}
    }

    if let Some(parsed) = pending_direct_connect {
        let session = claude_core::bridge::direct_connect::create_direct_connect_session(
            &reqwest::Client::new(),
            &parsed.server_url,
            parsed.auth_token.as_deref(),
            &std::env::current_dir()?.display().to_string(),
            cli.dangerously_skip_permissions || pending_direct_skip_permissions,
        )
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))?;
        if let Some(work_dir) = session.work_dir.as_deref() {
            std::env::set_current_dir(work_dir)?;
        }
        let mut app = claude_tui::app::App::new()?;
        app.set_direct_connect_config(session.config);
        app.run_direct_connect_only().await?;
        return Ok(());
    }

    // Resolve authentication (reads keychain, refreshes OAuth tokens)
    let auth = match claude_core::auth::resolve::resolve_auth().await {
        Ok(auth) => auth,
        Err(e) => {
            eprintln!();
            eprintln!("  \x1b[1mWelcome to Claude Code!\x1b[0m");
            eprintln!();
            eprintln!("  {}", e);
            eprintln!();
            eprintln!("  To get started:");
            eprintln!("  1. Run \x1b[1mclaude-rs login\x1b[0m");
            eprintln!("  2. Or set: \x1b[1mexport ANTHROPIC_API_KEY=sk-ant-...\x1b[0m");
            eprintln!();
            std::process::exit(1);
        }
    };

    // Load settings
    let cwd = std::env::current_dir()?;
    let project_root = claude_core::config::paths::detect_project_root(&cwd);

    // Prime the undercover repo-class cache early so `is_undercover()`
    // can resolve against a concrete classification before the first
    // tool description or commit prompt is rendered. Matches TS
    // setup.ts which calls isInternalModelRepo() at startup. Ant-only:
    // for other user types the reader short-circuits so the subprocess
    // cost is wasted — gate here. Fire-and-forget: `prime_repo_class
    // _from_remote` is idempotent and the conservative-safe default
    // (undercover ON when unprimed) covers the race if anything reads
    // the cache before this task completes.
    if claude_core::user_type::is_ant() {
        let prime_root = project_root.clone();
        tokio::task::spawn_blocking(move || {
            claude_core::undercover::prime_repo_class_from_remote(&prime_root);
        });
    }

    run_startup_migrations(&project_root).await;

    // Load settings from the same source buckets as TS (`--setting-sources`).
    // Policy and flag settings are always layered separately in TS; the Rust
    // permission loader applies the policy overlay, and `--settings` is merged
    // immediately below.
    let setting_sources = parse_setting_sources_flag(&cli.setting_sources)?;
    let mut settings = load_settings_from_sources(&project_root, &setting_sources);
    let mut raw_settings =
        claude_core::permissions::load_raw_settings_value_with_plugin_hooks_for_sources(
            &project_root,
            &setting_sources,
        );
    let mut permission_settings =
        claude_core::permissions::load_permission_settings_value_for_sources(
            &project_root,
            &setting_sources,
        );
    if let Some((overlay, overlay_value)) = load_settings_overlay(&cli.settings)? {
        settings = settings.merge(&overlay);
        merge_json_values(&mut raw_settings, overlay_value.clone());
        merge_json_values(&mut permission_settings, overlay_value);
    }
    let mut runtime_agents = load_plugin_agents(&project_root);
    runtime_agents.extend(load_markdown_agents(&cwd));
    let cli_agents = cli
        .agents
        .as_deref()
        .map(parse_cli_agents_json)
        .unwrap_or_default();
    runtime_agents.extend(cli_agents);
    claude_tools::agent_tool::set_runtime_agents(runtime_agents);
    let main_thread_agent = cli
        .agent
        .as_ref()
        .or(settings.agent.as_ref())
        .and_then(|agent_type| {
            claude_tools::agent_tool::active_agent_definitions()
                .into_iter()
                .find(|agent| &agent.agent_type == agent_type)
        });
    if let Some(initial_prompt) = main_thread_agent
        .as_ref()
        .and_then(|agent| agent.initial_prompt.as_ref())
    {
        prompt_arg = Some(match prompt_arg {
            Some(prompt) if !prompt.is_empty() => format!("{initial_prompt}\n\n{prompt}"),
            _ => initial_prompt.clone(),
        });
    }

    // Build tool registry
    let mut tools =
        claude_tools::build_default_registry_with_options(claude_tools::RegistryOptions {
            is_non_interactive_session: prompt_arg.is_some(),
        });
    if let Some(schema_text) = &cli.json_schema {
        let schema: serde_json::Value = serde_json::from_str(schema_text)
            .map_err(|err| anyhow::anyhow!("Invalid --json-schema JSON: {err}"))?;
        let tool = claude_tools::synthetic_output::JsonSchemaSyntheticOutputTool::new(schema)?;
        tools.register(Arc::new(tool));
    }
    let base_permission_tool_names = tools
        .all()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();

    // Determine permission mode and build the full permission context before
    // any prompt-visible tool list is finalized. TS routes `getTools()` and
    // `assembleToolPool()` through `filterToolsByDenyRules(permissionContext)`,
    // so CLI/disk/policy deny rules must all be visible here.
    let permission_mode = if cli.dangerously_skip_permissions {
        claude_core::permissions::types::PermissionMode::BypassPermissions
    } else if let Some(mode_str) = &cli.permission_mode {
        claude_core::permissions::types::PermissionMode::from_string(mode_str)
    } else if let Ok(mode_str) = std::env::var("CLAUDE_PERMISSION_MODE") {
        claude_core::permissions::types::PermissionMode::from_string(&mode_str)
    } else if let Some(mode) =
        claude_core::permissions::permission_mode_from_settings_value(&permission_settings)
    {
        mode
    } else {
        claude_core::permissions::types::PermissionMode::Default
    };
    let permission_rules_from_disk =
        claude_core::permissions::load_permission_rules_from_disk_by_source(&project_root);
    let additional_dirs =
        claude_core::permissions::permission_additional_directories_from_settings_value(
            &permission_settings,
        );
    let mut disallowed_tools_cli = cli.disallowed_tools.clone();
    disallowed_tools_cli.extend(base_tool_denials_from_cli_tools(
        &cli.tools,
        &base_permission_tool_names,
    ));
    let mut add_dirs = additional_dirs;
    add_dirs.extend(cli.add_dirs.clone());
    let initial_permission_context = claude_core::permissions::initialize_tool_permission_context(
        &cli.allowed_tools,
        &disallowed_tools_cli,
        permission_mode.clone(),
        cli.dangerously_skip_permissions,
        &add_dirs,
        &permission_rules_from_disk,
        cwd.clone(),
    )
    .tool_permission_context;

    filter_registry_by_cli_tools(&mut tools, &cli.tools);
    claude_tools::filter_registry_by_permission_context(&mut tools, &initial_permission_context);

    // Register bundled skills (simplify, stuck, remember, …).
    // Each skill's registrar applies its own TS-parity gate
    // (user-type, feature flag) so calling this unconditionally
    // at startup is safe. Matches TS setup.ts which calls each
    // registerXxxSkill() at startup and lets the skill choose
    // whether to register.
    claude_tools::bundled_skills::register_bundled_skills();

    // --- MCP server wiring ---
    // Connect to MCP servers configured in settings.mcpServers
    let mcp_manager = Arc::new(RwLock::new(claude_core::mcp::manager::McpManager::new()));
    let (dynamic_mcp_configs, dynamic_mcp_order) = load_dynamic_mcp_configs(&cli.mcp_config)?;
    let mut mcp_server_settings = if cli.strict_mcp_config {
        std::collections::HashMap::new()
    } else {
        settings.mcp_servers.clone()
    };
    let (plugin_mcp_servers, plugin_mcp_order) = if cli.strict_mcp_config || cli.bare {
        (std::collections::HashMap::new(), Vec::new())
    } else {
        load_enabled_plugin_mcp_servers(&project_root)
    };
    mcp_server_settings.extend(plugin_mcp_servers);
    let mut configs = std::collections::HashMap::new();
    let mut mcp_server_order = plugin_mcp_order;
    let mut plugin_mcp_server_names = Vec::new();
    for (name, entry) in &mcp_server_settings {
        if !mcp_server_order.contains(name) {
            mcp_server_order.push(name.clone());
        }
        let scoped = claude_core::mcp::types::ScopedMcpServerConfig {
            config: entry.to_mcp_server_config().map_err(|err| {
                anyhow::anyhow!("Error: Invalid MCP configuration:\nsettings: {name}: {err}")
            })?,
            scope: claude_core::mcp::types::ConfigScope::User,
        };
        if name.starts_with("plugin:") {
            plugin_mcp_server_names.push(name.clone());
        }
        configs.insert(name.clone(), scoped);
    }
    let (claude_ai_configs, discovered_claude_ai_original_urls) = if cli.strict_mcp_config {
        (
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        )
    } else {
        let ClaudeAiMcpDiscovery {
            configs: discovered_claude_ai_configs,
            order: discovered_claude_ai_order,
            original_urls: discovered_claude_ai_original_urls,
        } = fetch_claude_ai_mcp_configs_if_eligible().await;
        let claude_ai_configs = dedup_claude_ai_mcp_servers(discovered_claude_ai_configs, &configs);
        for name in discovered_claude_ai_order {
            if claude_ai_configs.contains_key(&name) && !mcp_server_order.contains(&name) {
                mcp_server_order.push(name);
            }
        }
        (claude_ai_configs, discovered_claude_ai_original_urls)
    };
    let claude_ai_shadow_servers = claude_ai_configs
        .iter()
        .map(|(name, config)| ShadowMcpServer {
            name: name.clone(),
            url: discovered_claude_ai_original_urls
                .get(name)
                .cloned()
                .or_else(|| mcp_config_url(config)),
            needs_auth: claude_ai_server_uses_auth_shadow(name, config),
            include_tools: true,
        })
        .collect::<Vec<_>>();
    configs.extend(claude_ai_configs);
    for name in dynamic_mcp_order {
        if !mcp_server_order.contains(&name) {
            mcp_server_order.push(name);
        }
    }
    configs.extend(dynamic_mcp_configs);
    let mut claude_ai_shadow_tools = Vec::new();
    claude_ai_shadow_tools.extend(mcp_contract_shadow_tools(
        plugin_mcp_server_names
            .into_iter()
            .map(|name| ShadowMcpServer {
                name,
                url: None,
                needs_auth: false,
                include_tools: true,
            }),
    ));
    if !configs.is_empty() {
        tracing::info!(
            count = configs.len(),
            "Connecting to configured MCP servers"
        );

        if is_interactive_session {
            let manager = mcp_manager.clone();
            let claude_ai_shadow_servers = claude_ai_shadow_servers.clone();
            tokio::spawn(async move {
                let mgr = manager.read().await;
                let connections = mgr.connect_all_respecting_auth_cache(configs).await;
                let connected = connections
                    .iter()
                    .filter(|conn| {
                        matches!(
                            conn.status,
                            claude_core::mcp::types::McpConnectionStatus::Connected { .. }
                        )
                    })
                    .map(|conn| conn.name.clone())
                    .collect::<std::collections::HashSet<_>>();
                let needs_auth = connections
                    .iter()
                    .filter(|conn| {
                        matches!(
                            conn.status,
                            claude_core::mcp::types::McpConnectionStatus::NeedsAuth
                        )
                    })
                    .map(|conn| conn.name.clone())
                    .collect::<std::collections::HashSet<_>>();
                let auth_shadow_tools = mcp_contract_shadow_tools(
                    claude_ai_shadow_servers.into_iter().map(|mut server| {
                        let is_connected = connected.contains(&server.name);
                        server.needs_auth = needs_auth.contains(&server.name)
                            || (server.needs_auth && !is_connected);
                        server.include_tools = true;
                        server
                    }),
                );
                if !auth_shadow_tools.is_empty() {
                    mgr.add_tool_definitions(auth_shadow_tools).await;
                }
                drop(mgr);
                for conn in &connections {
                    match &conn.status {
                        claude_core::mcp::types::McpConnectionStatus::Connected { .. } => {
                            tracing::info!(server = %conn.name, "MCP server connected");
                        }
                        claude_core::mcp::types::McpConnectionStatus::Failed { error } => {
                            tracing::warn!(
                                server = %conn.name,
                                error = ?error,
                                "MCP server failed to connect"
                            );
                        }
                        _ => {}
                    }
                }
            });
        } else {
            let expected_names = configs.keys().cloned().collect::<Vec<_>>();
            let manager = mcp_manager.clone();
            let _connect_handle = tokio::spawn(async move {
                let mgr = manager.read().await;
                let connections = mgr.connect_all_respecting_auth_cache(configs).await;
                drop(mgr);
                for conn in &connections {
                    match &conn.status {
                        claude_core::mcp::types::McpConnectionStatus::Connected { .. } => {
                            tracing::info!(server = %conn.name, "MCP server connected");
                        }
                        claude_core::mcp::types::McpConnectionStatus::Failed { error } => {
                            tracing::warn!(
                                server = %conn.name,
                                error = ?error,
                                "MCP server failed to connect"
                            );
                        }
                        _ => {}
                    }
                }
            });
            wait_for_headless_mcp_prewait(&mcp_manager, &expected_names).await;
            let mgr = mcp_manager.read().await;
            let connections = mgr.connections().await;
            let connected = connections
                .iter()
                .filter(|conn| {
                    matches!(
                        conn.status,
                        claude_core::mcp::types::McpConnectionStatus::Connected { .. }
                    )
                })
                .map(|conn| conn.name.clone())
                .collect::<std::collections::HashSet<_>>();
            let needs_auth = connections
                .iter()
                .filter(|conn| {
                    matches!(
                        conn.status,
                        claude_core::mcp::types::McpConnectionStatus::NeedsAuth
                    )
                })
                .map(|conn| conn.name.clone())
                .collect::<std::collections::HashSet<_>>();
            claude_ai_shadow_tools.extend(mcp_contract_shadow_tools(
                claude_ai_shadow_servers.into_iter().map(|mut server| {
                    let is_connected = connected.contains(&server.name);
                    server.needs_auth =
                        needs_auth.contains(&server.name) || (server.needs_auth && !is_connected);
                    server.include_tools = true;
                    server
                }),
            ));
            drop(mgr);
        }

        if !claude_ai_shadow_tools.is_empty() {
            let mgr = mcp_manager.read().await;
            mgr.add_tool_definitions(claude_ai_shadow_tools).await;
        }

        // Register immediately available MCP/shadow tools into the registry.
        claude_tools::register_mcp_tools(&mut tools, mcp_manager.clone()).await;
        claude_tools::register_mcp_resource_tools_if_supported(&mut tools, mcp_manager.clone())
            .await;
    }
    filter_registry_by_cli_tools(&mut tools, &cli.tools);
    claude_tools::filter_registry_by_permission_context(&mut tools, &initial_permission_context);
    let permission_prompt_tool = if let Some(name) = &cli.permission_prompt_tool {
        if prompt_arg.is_none() {
            eprintln!("Error: --permission-prompt-tool can only be used with --print");
            std::process::exit(1);
        }
        let Some(tool) = tools.remove(name) else {
            let available_mcp_tools = tools
                .all()
                .iter()
                .filter(|tool| tool.name().starts_with("mcp__"))
                .map(|tool| tool.name().to_string())
                .collect::<Vec<_>>();
            eprintln!(
                "Error: MCP tool {} (passed via --permission-prompt-tool) not found. Available MCP tools: {}",
                name,
                if available_mcp_tools.is_empty() {
                    "none".to_string()
                } else {
                    available_mcp_tools.join(", ")
                }
            );
            std::process::exit(1);
        };
        if !tool.name().starts_with("mcp__") {
            eprintln!(
                "Error: tool {} (passed via --permission-prompt-tool) must be an MCP tool",
                name
            );
            std::process::exit(1);
        }
        Some(tool)
    } else {
        None
    };
    claude_tools::register_tool_search_snapshot(&mut tools);
    filter_registry_by_cli_tools(&mut tools, &cli.tools);
    claude_tools::filter_registry_by_permission_context(&mut tools, &initial_permission_context);

    // --- Skill discovery ---
    let mut skill_additional_dirs =
        claude_core::permissions::permission_additional_directories_from_settings_value(
            &permission_settings,
        );
    skill_additional_dirs.extend(cli.add_dirs.clone());
    let skill_additional_dirs = skill_additional_dirs
        .iter()
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    let discovered_skills = claude_core::plugins::skill::discover_skills_with_additional(
        &project_root,
        &skill_additional_dirs,
    );
    let registered_skills = claude_tools::skill_tool::list_skills();
    let stream_skill_names = stream_json_skill_names(&registered_skills, &discovered_skills);
    let mut skills = Vec::new();
    {
        let mut seen = std::collections::HashSet::new();
        for skill in &registered_skills {
            if skill.prompt_phase == claude_tools::skill_tool::SkillPromptPhase::StaticCommand {
                continue;
            }
            if skill.disable_model_invocation {
                continue;
            }
            if !seen.insert(skill.name.clone()) {
                continue;
            }
            skills.push(claude_core::plugins::types::Skill {
                name: skill.name.clone(),
                description: skill.description.clone(),
                content: skill.content.clone(),
                source: claude_core::plugins::types::SkillSource::Builtin,
                argument_hint: None,
                when_to_use: None,
                paths: Vec::new(),
                allowed_tools: Vec::new(),
                user_invocable: skill.user_invocable,
                disable_model_invocation: skill.disable_model_invocation,
                is_plugin_command: false,
            });
        }
        for skill in discovered_skills.iter().cloned() {
            if skill.disable_model_invocation {
                continue;
            }
            if seen.insert(skill.name.clone()) {
                skills.push(skill);
            }
        }
        for skill in &registered_skills {
            if skill.prompt_phase != claude_tools::skill_tool::SkillPromptPhase::StaticCommand {
                continue;
            }
            if !seen.insert(skill.name.clone()) {
                continue;
            }
            skills.push(claude_core::plugins::types::Skill {
                name: skill.name.clone(),
                description: skill.description.clone(),
                content: skill.content.clone(),
                source: claude_core::plugins::types::SkillSource::Builtin,
                argument_hint: None,
                when_to_use: None,
                paths: Vec::new(),
                allowed_tools: Vec::new(),
                user_invocable: skill.user_invocable,
                disable_model_invocation: skill.disable_model_invocation,
                is_plugin_command: false,
            });
        }
    }
    if !skills.is_empty() {
        tracing::info!(count = skills.len(), "Discovered skills");
    }
    for skill in &skills {
        claude_tools::skill_tool::register_discovered_skill(skill);
    }

    let agent_model = main_thread_agent.as_ref().and_then(|agent| {
        agent.model.as_ref().and_then(|model| {
            if model == "inherit" {
                None
            } else {
                Some(model.clone())
            }
        })
    });
    let configured_model = match cli.model.or(agent_model).or_else(|| settings.model.clone()) {
        Some(model) => model,
        None => default_main_loop_model_setting().await,
    };
    let model = normalize_model_name(&configured_model);
    let fallback_model = match cli
        .fallback_model
        .clone()
        .filter(|value| !value.trim().is_empty())
    {
        Some(value) if value == "default" => Some(default_main_loop_model_setting().await),
        Some(value) => Some(normalize_model_name(&value)),
        None => None,
    };
    if fallback_model.as_deref() == Some(model.as_str()) {
        eprintln!("Error: Fallback model cannot be the same as the main model. Please specify a different model for --fallback-model.");
        std::process::exit(1);
    }

    tracing::info!(
        "claude-rs initialized: model={}, tools={}, mcp_servers={}, skills={}, project={}",
        model,
        tools.all().len(),
        settings.mcp_servers.len(),
        skills.len(),
        project_root.display(),
    );

    let mut custom_system_prompt = resolve_prompt_file_option(
        &cli.system_prompt,
        &cli.system_prompt_file,
        "Cannot use both --system-prompt and --system-prompt-file. Please use only one.",
        "System prompt file not found",
        "Error reading system prompt file",
    )?;
    if custom_system_prompt.is_none()
        && prompt_arg.is_some()
        && main_thread_agent
            .as_ref()
            .is_some_and(|agent| agent.source != claude_tools::agent_tool::AgentSource::BuiltIn)
    {
        custom_system_prompt = main_thread_agent
            .as_ref()
            .and_then(|agent| agent.system_prompt.clone());
    }
    let append_system_prompt = resolve_prompt_file_option(
        &cli.append_system_prompt,
        &cli.append_system_prompt_file,
        "Cannot use both --append-system-prompt and --append-system-prompt-file. Please use only one.",
        "Append system prompt file not found",
        "Error reading append system prompt file",
    )?;

    // Build system prompt
    let tool_descriptions: Vec<(String, String)> = tools
        .all()
        .iter()
        .map(|t| (t.name().to_string(), t.description()))
        .collect();
    let system_prompt: Vec<claude_core::types::content::ContentBlock> =
        if let Some(prompt) = custom_system_prompt {
            vec![claude_core::types::content::ContentBlock::Text { text: prompt }]
        } else {
            let system_prompt_values = claude_core::context::system_prompt::build_system_prompt(
                &project_root,
                &tool_descriptions,
                &model,
            )
            .await?;

            // Convert Vec<Value> to Vec<ContentBlock> for the engine
            system_prompt_values
                .into_iter()
                .filter_map(|v| {
                    v.get("text").and_then(|t| t.as_str()).map(|text| {
                        claude_core::types::content::ContentBlock::Text {
                            text: text.to_string(),
                        }
                    })
                })
                .collect()
        };

    // Create API client
    let model_display = model.clone();
    let account_uuid = if matches!(&auth, claude_core::api::client::AuthMethod::OAuthToken(_)) {
        claude_core::config::global::load_global_config()
            .ok()
            .and_then(|config| config.oauth_account.map(|account| account.account_uuid))
            .unwrap_or_default()
    } else {
        String::new()
    };
    if cli.resume.is_some() && cli.session_id.is_some() {
        eprintln!("Error: --session-id can only be used with --resume when forking the session.");
        std::process::exit(1);
    }
    let resolved_resume_session_id = if let Some(ref session_id_raw) = cli.resume {
        Some(if session_id_raw.eq_ignore_ascii_case("last") {
            let sessions = claude_core::session::manager::SessionManager::list_sessions()?;
            match sessions.first() {
                Some(info) => {
                    tracing::info!("Resolved --resume last to session '{}'", info.id);
                    info.id.clone()
                }
                None => {
                    eprintln!("No previous sessions found to resume.");
                    std::process::exit(1);
                }
            }
        } else {
            session_id_raw.clone()
        })
    } else {
        None
    };
    let api_config = claude_core::api::client::ApiConfig {
        max_tokens: resolve_max_output_tokens(&model, &settings),
        session_id: cli
            .session_id
            .clone()
            .or_else(|| resolved_resume_session_id.clone())
            .unwrap_or_else(|| claude_core::api::client::get_session_id().clone()),
        account_uuid,
        effort: resolve_effort_for_api(cli.effort.as_deref(), &settings),
        task_budget_total: cli.task_budget,
        fallback_model: fallback_model.clone(),
        workload: cli
            .workload
            .clone()
            .filter(|value| !value.trim().is_empty()),
        sdk_betas: filter_allowed_sdk_betas(
            &cli.betas,
            matches!(&auth, claude_core::api::client::AuthMethod::OAuthToken(_)),
        ),
        model: model.clone(),
        ..Default::default()
    };
    // Install a Haiku-backed secondary model so WebFetchTool and other tools
    // that need cheap post-processing can actually call the API. Without this
    // the prompt param on WebFetch is preserved in the result payload but no
    // summarisation happens.
    {
        let haiku = std::sync::Arc::new(claude_core::secondary_model::HaikuSecondaryModel::new(
            auth.clone(),
            api_config.base_url.clone(),
            api_config.session_id.clone(),
            model.clone(),
        ));
        claude_core::secondary_model::set_global(haiku);
    }

    // Install a HookRunner so TaskCreateTool (and future hook sites) can fire
    // user-configured hooks. We serialise the typed Settings to a JSON Value
    // so HookRunner::from_settings can pick the "hooks" subtree out of it.
    // If the user has no hooks configured the runner is a no-op.
    let hook_runner = {
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let session_id = api_config.session_id.clone();
        let settings_value = raw_settings.clone();
        let runner = std::sync::Arc::new(claude_core::hooks::HookRunner::from_settings(
            &settings_value,
            cwd,
            session_id,
            String::new(),
        ));
        claude_core::hooks::set_global_runner(runner.clone());
        runner
    };

    let api_session_id = api_config.session_id.clone();
    claude_core::hooks::clear_hook_event_state();
    claude_core::hooks::set_all_hook_events_enabled(
        cli.include_hook_events || claude_core::errors_util::is_env_truthy("CLAUDE_CODE_REMOTE"),
    );
    if cli.output_format == OutputFormat::StreamJson {
        let hook_session_id = api_session_id.clone();
        claude_core::hooks::register_hook_event_handler(Some(std::sync::Arc::new(move |event| {
            emit_stream_json_hook_execution_event(event, &hook_session_id);
        })));
        let rate_limit_session_id = api_session_id.clone();
        claude_core::rate_limits::register_status_listener(std::sync::Arc::new(move |limits| {
            emit_stream_json(stream_json_rate_limit_event(
                &rate_limit_session_id,
                &limits,
            ));
        }));
    }
    let api_client = claude_core::api::client::ApiClient::new(api_config, auth.clone());

    // Create cancellation token
    let cancel = tokio_util::sync::CancellationToken::new();

    // Get tool definitions for the engine
    let tool_definition_context = claude_tools::registry::ToolDefinitionContext {
        permission_context: &initial_permission_context,
        available_tools: tools
            .all()
            .iter()
            .map(|tool| tool.name().to_string())
            .collect(),
        allowed_agent_types: std::env::var("CLAUDE_CODE_ALLOWED_AGENT_TYPES")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|values| !values.is_empty()),
    };
    let tool_defs = tools.tool_definitions_with_context(Some(&tool_definition_context));
    let stream_json_tool_names: Vec<String> = tool_defs
        .iter()
        .map(|tool| {
            if tool.name == "Agent" {
                "Task".to_string()
            } else {
                tool.name.clone()
            }
        })
        .collect();

    // Create query engine
    let mut query_engine = claude_core::query::engine::QueryEngine::new(
        api_client,
        system_prompt,
        tool_defs,
        cancel.clone(),
    );
    if let Ok(storage) = claude_core::session::storage::SessionStorage::new(&api_session_id) {
        query_engine.set_transcript_storage(storage);
    }
    if let Some(max) = cli.max_turns {
        query_engine.set_max_turns(max);
    }
    let mut resumed_history_has_user_context = false;
    let mut ccr_v2_resume_attempted = false;

    // Handle --resume: load transcript from a previous session.
    // Supports `--resume <session-id>` or `--resume last` to pick the most recent.
    if let Some(resolved_id) = resolved_resume_session_id.as_deref() {
        if claude_core::errors_util::is_env_truthy("CLAUDE_CODE_USE_CCR_V2") {
            if let Some(sdk_url) = cli.sdk_url.as_deref() {
                ccr_v2_resume_attempted = true;
                let _ = hydrate_remote_ccr_internal_events(sdk_url, resolved_id).await?;
            }
        }
        let session_mgr = claude_core::session::manager::SessionManager::resume(resolved_id)?;
        let transcript = session_mgr.storage().load_transcript()?;
        if transcript.is_empty() {
            if !ccr_v2_resume_attempted {
                eprintln!(
                    "Warning: session '{}' has no transcript to resume",
                    resolved_id
                );
            }
        } else {
            tracing::info!(
                "Resuming session '{}' with {} messages",
                resolved_id,
                transcript.len()
            );
            eprintln!(
                "Resuming session {} ({} messages)",
                resolved_id,
                transcript.len()
            );
            query_engine.load_messages(transcript);
            resumed_history_has_user_context = query_engine.has_user_context_in_history();
            query_engine.add_resume_continuation_markers();
        }
    }

    let session_start = hook_runner
        .run_hooks(
            &claude_core::hooks::types::HookEvent::SessionStart,
            serde_json::json!({
                "source": if cli.resume.is_some() { "resume" } else { "startup" },
            }),
            None,
            None,
            None,
            None,
        )
        .await;
    if !resumed_history_has_user_context {
        for context in session_start.additional_contexts {
            query_engine.append_user_context_block(format!(
                "<system-reminder>\nSessionStart hook additional context: {}\n</system-reminder>",
                context
            ));
        }
    }
    let session_start_initial_user_message = session_start.initial_user_message;

    // Add MCP server instructions as request-time user context, matching TS.
    if !resumed_history_has_user_context {
        let mgr = mcp_manager.read().await;
        let connections = mgr.connections().await;
        let mut instructions_parts: Vec<String> = Vec::new();
        let mut connected_instruction_blocks = Vec::new();
        for conn in &connections {
            if let claude_core::mcp::types::McpConnectionStatus::Connected {
                instructions: Some(ref instr),
                ..
            } = conn.status
            {
                connected_instruction_blocks
                    .push((conn.name.clone(), format!("## {}\n{}", conn.name, instr)));
            }
        }
        connected_instruction_blocks.sort_by(|a, b| a.0.cmp(&b.0));
        instructions_parts.extend(
            connected_instruction_blocks
                .into_iter()
                .map(|(_, block)| block),
        );
        if !instructions_parts.is_empty() {
            query_engine.append_user_context_block(format!(
                "<system-reminder>\n# MCP Server Instructions\n\nThe following MCP servers have provided instructions for how to use their tools and resources:\n\n{}\n</system-reminder>",
                instructions_parts.join("\n\n")
            ));
        }
    }

    // Add skill descriptions as request-time user context, matching TS.
    if !resumed_history_has_user_context && !skills.is_empty() {
        query_engine.append_user_context_block(skills_reminder_block(&skills));
    }

    if !resumed_history_has_user_context {
        if let Some(context) =
            claude_core::context::system_prompt::build_project_user_context_block_with_additional(
                &project_root,
                &skill_additional_dirs,
            )
        {
            query_engine.append_user_context_block(context);
        }
    }

    // TS getSystemContext() appends gitStatus to the system prompt, gated by
    // CLAUDE_CODE_REMOTE and includeGitInstructions.
    if !claude_core::errors_util::is_env_truthy("CLAUDE_CODE_REMOTE")
        && claude_core::context::git_settings::should_include_git_instructions(&project_root)
    {
        if let Ok(Some(git_status)) =
            claude_core::context::git::get_git_context(&project_root).await
        {
            query_engine.append_system_context("gitStatus", git_status);
        }
    }

    // Append --append-system-prompt text or --append-system-prompt-file content.
    if let Some(extra) = append_system_prompt {
        query_engine.append_system_prompt(extra);
    }

    // Handle non-interactive prompt mode
    if let Some(prompt) = prompt_arg {
        // If using OAuth proxy, delegate to real claude binary
        use claude_core::hooks::{
            get_global_runner, resolve_hook_permission_decision, run_post_tool_use_failure_hooks,
            run_post_tool_use_hooks, run_pre_tool_use_hooks, ResolvedPermission,
        };
        use claude_core::permissions::evaluator::{evaluate_permission, SimpleToolPermissions};
        use claude_core::permissions::types::PermissionDecision;
        use claude_core::query::engine::TurnResult;
        use claude_core::types::events::StreamEvent;
        use claude_tools::ToolUseContext;
        use std::path::PathBuf;
        use tokio::sync::mpsc;

        let mut cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        // Remember the original directory so ExitWorktree can restore it.
        let original_cwd = cwd.clone();
        let read_file_state = std::sync::Arc::new(std::sync::Mutex::new(
            claude_tools::registry::ReadFileState::new(),
        ));
        let mut perm_ctx = initial_permission_context.clone();
        let mut active_model = model.clone();
        let mut active_model_display = model_display.clone();

        let mut pending_remote_prompt = RemotePrompt {
            content: prompt_content
                .take()
                .unwrap_or_else(|| serde_json::json!(prompt)),
            delivery_event_id: prompt_delivery_event_id.take(),
        };
        let mut add_session_start_message = true;
        let mut queued_remote_prompts = std::collections::VecDeque::new();
        let mut queued_remote_control_requests =
            std::collections::VecDeque::from(initial_remote_control_requests);
        loop {
            while let Some(control_request) = queued_remote_control_requests.pop_front() {
                apply_remote_control_request(
                    control_request,
                    &mut query_engine,
                    &mut perm_ctx,
                    &mut active_model,
                    &mut active_model_display,
                )
                .await?;
            }
            let current_remote_prompt = pending_remote_prompt;
            if let Some(event_id) = current_remote_prompt.delivery_event_id.as_deref() {
                report_remote_ccr_delivery(event_id, "processing");
            }
            let current_delivery_event_id = current_remote_prompt.delivery_event_id.clone();
            let prompt_content = current_remote_prompt.content;
            let prompt = sdk_content_to_prompt(&prompt_content).unwrap_or_default();

            // Check if prompt is a skill invocation (e.g. "/commit fix typo")
            let mut effective_prompt = prompt.clone();
            let mut effective_content = prompt_content.clone();
            for skill in &skills {
                if let Some(args) = claude_core::plugins::skill::match_skill(&prompt, skill) {
                    let mut content = skill.content.clone();
                    if !args.is_empty() {
                        content.push_str(&format!("\n\nArguments: {}", args));
                    }
                    effective_prompt = content;
                    effective_content = serde_json::json!(effective_prompt);
                    break;
                }
            }

            let user_prompt_submit = hook_runner
                .run_hooks(
                    &claude_core::hooks::types::HookEvent::UserPromptSubmit,
                    serde_json::json!({ "prompt": effective_prompt.clone() }),
                    Some(permission_mode_hook_name(&perm_ctx.mode)),
                    None,
                    None,
                    None,
                )
                .await;
            if !user_prompt_submit.blocking_errors.is_empty() {
                let blocking = user_prompt_submit
                    .blocking_errors
                    .iter()
                    .map(claude_core::hooks::get_user_prompt_submit_hook_blocking_message)
                    .collect::<Vec<_>>()
                    .join("\n");
                let message = format!("{blocking}\n\nOriginal prompt: {effective_prompt}");
                match cli.output_format {
                    OutputFormat::Text => println!("{message}"),
                    OutputFormat::Json => println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "type": "result",
                            "subtype": "error",
                            "is_error": true,
                            "errors": [message],
                        }))?
                    ),
                    OutputFormat::StreamJson => emit_stream_json(serde_json::json!({
                        "type": "result",
                        "subtype": "error",
                        "is_error": true,
                        "errors": [message],
                        "session_id": api_session_id,
                    })),
                }
                let mgr = mcp_manager.read().await;
                mgr.disconnect_all().await;
                return Ok(());
            }
            if user_prompt_submit.prevent_continuation {
                let message = user_prompt_submit
                    .stop_reason
                    .unwrap_or_else(|| "Operation stopped by hook".to_string());
                match cli.output_format {
                    OutputFormat::Text => println!("{message}"),
                    OutputFormat::Json => println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({
                            "type": "result",
                            "subtype": "error",
                            "is_error": true,
                            "errors": [message],
                        }))?
                    ),
                    OutputFormat::StreamJson => emit_stream_json(serde_json::json!({
                        "type": "result",
                        "subtype": "error",
                        "is_error": true,
                        "errors": [message],
                        "session_id": api_session_id,
                    })),
                }
                let mgr = mcp_manager.read().await;
                mgr.disconnect_all().await;
                return Ok(());
            }
            for context in user_prompt_submit.additional_contexts {
                query_engine.append_user_context_block(format!(
                "<system-reminder>\nUserPromptSubmit hook additional context: {}\n</system-reminder>",
                context
            ));
            }

            if cli.output_format == OutputFormat::StreamJson {
                let (mcp_servers, mcp_prompt_commands) = {
                    let mgr = mcp_manager.read().await;
                    (
                        stream_json_mcp_servers_in_order(
                            mgr.connections().await,
                            &mcp_server_order,
                        ),
                        mgr.prompt_command_names_in_order(&mcp_server_order).await,
                    )
                };
                emit_stream_json(stream_json_init_event(StreamJsonInitMeta {
                    cwd: &cwd,
                    session_id: &api_session_id,
                    tool_names: stream_json_tool_names.clone(),
                    mcp_servers,
                    model_display: &active_model_display,
                    permission_mode: &perm_ctx.mode,
                    registered_skills: &registered_skills,
                    discovered_skills: &discovered_skills,
                    stream_skill_names: &stream_skill_names,
                    mcp_prompt_commands: &mcp_prompt_commands,
                    output_style: settings.output_style.as_deref(),
                    auth: &auth,
                }));
            }

            if add_session_start_message {
                if let Some(initial) = &session_start_initial_user_message {
                    query_engine.add_user_message(initial);
                }
                add_session_start_message = false;
            }
            query_engine.add_user_content(&effective_content);

            // Run the agentic loop: prompt -> run_turn -> ToolUse* -> Done
            let mut final_text = String::new();
            let mut structured_output: Option<serde_json::Value> = None;
            let mut structured_output_retry_count: u32 = 0;
            let max_structured_output_retries = std::env::var("MAX_STRUCTURED_OUTPUT_RETRIES")
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(5);
            let session_id = api_session_id.clone();
            let started_at = std::time::Instant::now();
            let mut latest_usage: Option<claude_core::types::usage::Usage> = None;
            let mut total_usage: Option<claude_core::types::usage::Usage> = None;
            let mut num_turns: u32 = 0;
            let terminal_outcome = loop {
                let (stream_tx, mut stream_rx) = mpsc::channel::<StreamEvent>(128);
                let output_format = cli.output_format.clone();
                let stream_session_id = session_id.clone();

                // Spawn a task to print streamed text to stdout
                let print_handle = tokio::spawn(async move {
                    let mut state = StreamJsonPrintState::default();
                    while let Some(ev) = stream_rx.recv().await {
                        match &output_format {
                            OutputFormat::Text => {
                                if let StreamEvent::TextDelta { text } = ev {
                                    state.text.push_str(&text);
                                }
                            }
                            OutputFormat::Json => {
                                if let StreamEvent::TextDelta { text: delta } = ev {
                                    state.text.push_str(&delta);
                                }
                            }
                            OutputFormat::StreamJson => {
                                if let StreamEvent::TextDelta { text: delta } = &ev {
                                    state.text.push_str(delta);
                                }
                                if let StreamEvent::UsageUpdate(usage) = &ev {
                                    merge_stream_usage(&mut state.latest_usage, usage.clone());
                                }
                                for value in stream_event_to_stream_json_events(
                                    &ev,
                                    &stream_session_id,
                                    include_partial_messages,
                                ) {
                                    emit_stream_json(value);
                                }
                            }
                        }
                    }
                    state
                });

                num_turns += 1;
                if cli.output_format == OutputFormat::StreamJson && include_partial_messages {
                    emit_stream_json(stream_json_status_event(&session_id, Some("requesting")));
                }
                let result = query_engine.run_turn(&stream_tx).await?;
                drop(stream_tx);
                if let Ok(state) = print_handle.await {
                    final_text.push_str(&state.text);
                    if let Some(usage) = state.latest_usage {
                        accumulate_stream_usage(&mut total_usage, &usage);
                        latest_usage = Some(usage);
                    }
                }
                if let Some(max_budget_usd) = cli.max_budget_usd {
                    if total_cost_for_usage(&active_model, total_usage.as_ref()) >= max_budget_usd {
                        break PrintTerminalOutcome::MaxBudgetUsd { max_budget_usd };
                    }
                }

                match result {
                    TurnResult::Done(stop_reason) => {
                        if cli.json_schema.is_some() && structured_output.is_none() {
                            if structured_output_retry_count >= max_structured_output_retries {
                                break PrintTerminalOutcome::StructuredOutputRetries {
                                    max_retries: max_structured_output_retries,
                                };
                            }
                            structured_output_retry_count += 1;
                            query_engine.add_user_message(
                            "You MUST call the StructuredOutput tool to complete this request. \
                             Call this tool now.",
                        );
                            continue;
                        }
                        let stop_reason = serde_json::to_string(&stop_reason)
                            .ok()
                            .and_then(|value| serde_json::from_str::<String>(&value).ok())
                            .unwrap_or_else(|| "end_turn".to_string());
                        break PrintTerminalOutcome::Completed { stop_reason };
                    }
                    TurnResult::MaxTurns {
                        max_turns,
                        turn_count,
                    } => {
                        break PrintTerminalOutcome::MaxTurns {
                            max_turns,
                            turn_count,
                        };
                    }
                    TurnResult::ContinueRecovery => {
                        // max_tokens recovery — run again immediately
                        continue;
                    }
                    TurnResult::Continue => {
                        continue;
                    }
                    TurnResult::ToolUse(tool_uses) => {
                        // Execute each tool, check permissions, feed results back
                        for tool_info in &tool_uses {
                            if cli.output_format == OutputFormat::StreamJson {
                                emit_stream_json(stream_json_assistant_tool_use_event(
                                    tool_info,
                                    &session_id,
                                    &active_model,
                                    latest_usage.as_ref(),
                                ));
                            }
                            let mut tool_input = tool_info.input.clone();
                            let mut forced_permission: Option<Result<(), String>> = None;
                            if let Some(runner) = get_global_runner() {
                                let pre = run_pre_tool_use_hooks(
                                    &runner,
                                    &tool_info.name,
                                    &tool_info.id,
                                    &tool_input,
                                    Some(permission_mode_hook_name(&perm_ctx.mode)),
                                    None,
                                    None,
                                )
                                .await;
                                for context in &pre.additional_contexts {
                                    query_engine.append_user_context_block(context.clone());
                                }
                                if let Some(message) =
                                    hook_blocking_errors_text(&pre.blocking_errors)
                                        .or_else(|| pre.denial_message.clone())
                                {
                                    forced_permission = Some(Err(message));
                                } else if pre.prevent_continuation {
                                    forced_permission =
                                        Some(Err(pre.stop_reason.unwrap_or_else(|| {
                                            "PreToolUse hook stopped tool execution".to_string()
                                        })));
                                } else {
                                    let resolved = resolve_hook_permission_decision(
                                    &pre,
                                    &tool_input,
                                    |candidate_input| {
                                        let tool_name = tool_info.name.clone();
                                        let perm_ctx = perm_ctx.clone();
                                        let tools = tools.clone();
                                        let candidate_input = candidate_input.clone();
                                        async move {
                                            let decision = if let Some(tool) =
                                                tools.get(&tool_name)
                                            {
                                                let tool_perms =
                                                    claude_tools::registry::ExecutorToolPermissions::new(
                                                        tool,
                                                        candidate_input.clone(),
                                                    );
                                                evaluate_permission(
                                                    &tool_perms,
                                                    &candidate_input,
                                                    &perm_ctx,
                                                )
                                            } else {
                                                let tool_perms = SimpleToolPermissions::new(
                                                    &tool_name,
                                                    false,
                                                );
                                                evaluate_permission(
                                                    &tool_perms,
                                                    &candidate_input,
                                                    &perm_ctx,
                                                )
                                            };
                                            permission_decision_to_rule_check(&decision)
                                        }
                                    },
                                )
                                .await;
                                    match resolved {
                                        ResolvedPermission::Allow { updated_input }
                                        | ResolvedPermission::NormalFlow { updated_input } => {
                                            tool_input = merge_hook_updated_input(
                                                &tool_input,
                                                &updated_input,
                                            );
                                        }
                                        ResolvedPermission::Deny { message } => {
                                            forced_permission =
                                                Some(Err(message.unwrap_or_else(|| {
                                                    "PreToolUse hook denied this tool".to_string()
                                                })));
                                        }
                                        ResolvedPermission::RequiresUserConfirmation {
                                            updated_input,
                                            force_decision,
                                        } => {
                                            tool_input = merge_hook_updated_input(
                                                &tool_input,
                                                &updated_input,
                                            );
                                            forced_permission = Some(Err(force_decision
                                                .unwrap_or_else(|| {
                                                    "Tool requires user confirmation".to_string()
                                                })));
                                        }
                                    }
                                }
                            }
                            let decision = if let Some(forced) = forced_permission {
                                match forced {
                                Ok(()) => PermissionDecision::allow(),
                                Err(message) => PermissionDecision::deny(
                                    message,
                                    claude_core::permissions::types::PermissionDecisionReason::Hook {
                                        hook_name: format!("PreToolUse:{}", tool_info.name),
                                        hook_source: None,
                                        reason: None,
                                    },
                                ),
                            }
                            } else if let Some(tool) = tools.get(&tool_info.name) {
                                let tool_perms =
                                    claude_tools::registry::ExecutorToolPermissions::new(
                                        tool,
                                        tool_input.clone(),
                                    );
                                evaluate_permission(&tool_perms, &tool_input, &perm_ctx)
                            } else {
                                let tool_perms = SimpleToolPermissions::new(&tool_info.name, false);
                                evaluate_permission(&tool_perms, &tool_input, &perm_ctx)
                            };

                            let (mut result_text, mut is_error, mut result_json) = match decision {
                                PermissionDecision::Ask(ask) => {
                                    if let Some(rx) = remote_sdk_rx.as_mut() {
                                        let request_id = uuid::Uuid::new_v4().to_string();
                                        emit_stream_json(remote_permission_request(
                                            &request_id,
                                            &tool_info.name,
                                            &tool_info.id,
                                            &tool_input,
                                            &ask.message,
                                            &ask.suggestions,
                                            &ask.blocked_path,
                                        ));
                                        match wait_remote_permission_response(
                                            rx,
                                            &request_id,
                                            &mut queued_remote_prompts,
                                            &mut queued_remote_control_requests,
                                        )
                                        .await
                                        .map(|value| parse_remote_permission_response(&value))
                                        {
                                            Some(Ok((
                                                true,
                                                updated_input,
                                                updated_permissions,
                                                _,
                                            ))) => {
                                                if let Some(updates) = updated_permissions {
                                                    perm_ctx = claude_core::permissions::evaluator::apply_permission_updates(
                                                    perm_ctx,
                                                    &updates,
                                                );
                                                    if let Err(err) = claude_core::permissions::evaluator::persist_permission_updates(
                                                    &updates,
                                                    &cwd,
                                                ) {
                                                    tracing::warn!(
                                                        error = %err,
                                                        "failed to persist remote permission updates"
                                                    );
                                                }
                                                }
                                                if let Some(updated_input) = updated_input {
                                                    tool_input = updated_input;
                                                }
                                                let executor = tools.get(&tool_info.name);
                                                match executor {
                                                    Some(exec) => {
                                                        let ctx = ToolUseContext::new(
                                                            cwd.clone(),
                                                            read_file_state.clone(),
                                                            perm_ctx.mode.clone(),
                                                            perm_ctx.clone(),
                                                            std::sync::Arc::new({
                                                                let mut options =
                                                                    claude_core::tool_use_context_options::ToolUseContextOptions::minimal(&active_model);
                                                                options.session_id =
                                                                    Some(api_session_id.clone());
                                                                options
                                                            }),
                                                            std::sync::Arc::new(
                                                                claude_core::tool_host::NullToolHost,
                                                            ),
                                                        );
                                                        match exec
                                                            .call(
                                                                &tool_input,
                                                                &ctx,
                                                                cancel.clone(),
                                                                None,
                                                            )
                                                            .await
                                                        {
                                                            Ok(data) => {
                                                                if !data.is_error {
                                                                    match tool_info.name.as_str() {
                                                                        "EnterWorktree" => {
                                                                            if let Some(path) = data
                                                                                .data
                                                                                ["worktreePath"]
                                                                                .as_str()
                                                                            {
                                                                                let old_cwd =
                                                                                    cwd.clone();
                                                                                let new_cwd =
                                                                                    PathBuf::from(
                                                                                        path,
                                                                                    );
                                                                                claude_core::hooks::fire_cwd_changed(
                                                                                &old_cwd.display().to_string(),
                                                                                &new_cwd.display().to_string(),
                                                                            )
                                                                            .await;
                                                                                cwd = new_cwd;
                                                                                tracing::info!(
                                                                                "Session cwd switched to worktree: {}",
                                                                                path
                                                                            );
                                                                            }
                                                                        }
                                                                        "ExitWorktree" => {
                                                                            let old_cwd =
                                                                                cwd.clone();
                                                                            let new_cwd =
                                                                                original_cwd
                                                                                    .clone();
                                                                            claude_core::hooks::fire_cwd_changed(
                                                                            &old_cwd.display().to_string(),
                                                                            &new_cwd.display().to_string(),
                                                                        )
                                                                        .await;
                                                                            cwd = new_cwd;
                                                                            tracing::info!(
                                                                            "Session cwd restored to: {}",
                                                                            original_cwd.display()
                                                                        );
                                                                        }
                                                                        _ => {}
                                                                    }
                                                                }
                                                                let text = format_tool_result_for_model(
                                                                    &tool_info.name,
                                                                    &data.data,
                                                                );
                                                                (text, data.is_error, data.data)
                                                            }
                                                            Err(e) => {
                                                                let message = format!("Error: {}", e);
                                                                (
                                                                    message.clone(),
                                                                    true,
                                                                    serde_json::json!({"error": message}),
                                                                )
                                                            }
                                                        }
                                                    }
                                                    None => {
                                                        let message =
                                                            claude_core::tool_result_format::unknown_tool_error_text(
                                                                &tool_info.name,
                                                            );
                                                        (
                                                            claude_core::tool_result_format::unknown_tool_error_content(
                                                                &tool_info.name,
                                                            ),
                                                            true,
                                                            serde_json::json!({"error": message}),
                                                        )
                                                    }
                                                }
                                            }
                                            Some(Ok((false, _, _, message))) => (
                                                format!("Permission denied: {}", message),
                                                true,
                                                serde_json::json!({"error": message}),
                                            ),
                                            Some(Err(err)) => {
                                                let message = err.to_string();
                                                (
                                                    format!("Permission prompt error: {}", message),
                                                    true,
                                                    serde_json::json!({"error": message}),
                                                )
                                            }
                                            None => (
                                                "Permission denied: remote permission stream closed"
                                                    .to_string(),
                                                true,
                                                serde_json::json!({"error": "remote permission stream closed"}),
                                            ),
                                        }
                                    } else if let Some(permission_prompt_tool) =
                                        &permission_prompt_tool
                                    {
                                        match run_permission_prompt_tool(
                                            permission_prompt_tool,
                                            &tool_info.name,
                                            &tool_info.id,
                                            &tool_input,
                                            &cwd,
                                            read_file_state.clone(),
                                            perm_ctx.mode.clone(),
                                            &active_model,
                                            cancel.clone(),
                                        )
                                        .await
                                        {
                                            Ok(output) if output.behavior == "allow" => {
                                                if let Some(updates) = output.updated_permissions {
                                                    perm_ctx = claude_core::permissions::evaluator::apply_permission_updates(
                                                    perm_ctx,
                                                    &updates,
                                                );
                                                    if let Err(err) = claude_core::permissions::evaluator::persist_permission_updates(
                                                    &updates,
                                                    &cwd,
                                                ) {
                                                    tracing::warn!(
                                                        error = %err,
                                                        "failed to persist permission prompt tool updates"
                                                    );
                                                }
                                                }
                                                let updated_input = output
                                                    .updated_input
                                                    .unwrap_or_else(|| tool_input.clone());
                                                tool_input = if updated_input
                                                    .as_object()
                                                    .is_some_and(|obj| obj.is_empty())
                                                {
                                                    tool_input
                                                } else {
                                                    updated_input
                                                };
                                                let executor = tools.get(&tool_info.name);
                                                match executor {
                                                    Some(exec) => {
                                                        let ctx = ToolUseContext::new(
                                                        cwd.clone(),
                                                        read_file_state.clone(),
                                                        perm_ctx.mode.clone(),
                                                        perm_ctx.clone(),
                                                        std::sync::Arc::new({
                                                            let mut options =
                                                                claude_core::tool_use_context_options::ToolUseContextOptions::minimal(&active_model);
                                                            options.session_id =
                                                                Some(api_session_id.clone());
                                                            options
                                                        }),
                                                        std::sync::Arc::new(
                                                            claude_core::tool_host::NullToolHost,
                                                        ),
                                                    );
                                                        match exec
                                                            .call(
                                                                &tool_input,
                                                                &ctx,
                                                                cancel.clone(),
                                                                None,
                                                            )
                                                            .await
                                                        {
                                                            Ok(data) => {
                                                                if !data.is_error {
                                                                    match tool_info.name.as_str() {
                                                                        "EnterWorktree" => {
                                                                            if let Some(path) = data
                                                                                .data
                                                                                ["worktreePath"]
                                                                                .as_str()
                                                                            {
                                                                                let old_cwd =
                                                                                    cwd.clone();
                                                                                let new_cwd =
                                                                                    PathBuf::from(
                                                                                        path,
                                                                                    );
                                                                                claude_core::hooks::fire_cwd_changed(
                                                                                &old_cwd.display().to_string(),
                                                                                &new_cwd.display().to_string(),
                                                                            )
                                                                            .await;
                                                                                cwd = new_cwd;
                                                                                tracing::info!(
                                                                                "Session cwd switched to worktree: {}",
                                                                                path
                                                                            );
                                                                            }
                                                                        }
                                                                        "ExitWorktree" => {
                                                                            let old_cwd =
                                                                                cwd.clone();
                                                                            let new_cwd =
                                                                                original_cwd
                                                                                    .clone();
                                                                            claude_core::hooks::fire_cwd_changed(
                                                                            &old_cwd.display().to_string(),
                                                                            &new_cwd.display().to_string(),
                                                                        )
                                                                        .await;
                                                                            cwd = new_cwd;
                                                                            tracing::info!(
                                                                            "Session cwd restored to: {}",
                                                                            original_cwd.display()
                                                                        );
                                                                        }
                                                                        _ => {}
                                                                    }
                                                                }
                                                                let text =
                                                                    format_tool_result_for_model(
                                                                        &tool_info.name,
                                                                        &data.data,
                                                                    );
                                                                (text, data.is_error, data.data)
                                                            }
                                                            Err(e) => {
                                                                let message =
                                                                    format!("Error: {}", e);
                                                                (
                                                                    message.clone(),
                                                                    true,
                                                                    serde_json::json!({"error": message}),
                                                                )
                                                            }
                                                        }
                                                    }
                                                    None => {
                                                        let message =
                                                        claude_core::tool_result_format::unknown_tool_error_text(
                                                            &tool_info.name,
                                                        );
                                                        (
                                                        claude_core::tool_result_format::unknown_tool_error_content(
                                                            &tool_info.name,
                                                        ),
                                                        true,
                                                        serde_json::json!({"error": message}),
                                                    )
                                                    }
                                                }
                                            }
                                            Ok(output) => {
                                                let message = output.message.unwrap_or_else(|| {
                                                    "Permission denied by permission prompt tool"
                                                        .to_string()
                                                });
                                                (
                                                    format!("Permission denied: {}", message),
                                                    true,
                                                    serde_json::json!({"error": message}),
                                                )
                                            }
                                            Err(err) => {
                                                let message = err.to_string();
                                                (
                                                    format!(
                                                        "Permission prompt tool error: {}",
                                                        message
                                                    ),
                                                    true,
                                                    serde_json::json!({"error": message}),
                                                )
                                            }
                                        }
                                    } else {
                                        // In non-interactive / headless mode, Ask decisions are DENIED
                                        // (matching TS headless behavior). Auto-allowing would bypass
                                        // permission semantics when running unattended.
                                        tracing::warn!(
                                            tool = %tool_info.name,
                                            reason = %ask.message,
                                            "Non-interactive mode: denying tool requiring user confirmation"
                                        );
                                        (
                                            format!(
                                                "Permission denied (non-interactive): {}",
                                                ask.message
                                            ),
                                            true,
                                            serde_json::json!({"error": ask.message}),
                                        )
                                    }
                                }
                                PermissionDecision::Allow(_) => {
                                    let executor = tools.get(&tool_info.name);
                                    match executor {
                                        Some(exec) => {
                                            // Explicit construction (not `for_test`) so the session's
                                            // live `model` reaches `ctx.options.main_loop_model` —
                                            // consumed by e.g. the command adapter at
                                            // `claude-core/src/command_adapter.rs:114`.
                                            let ctx = ToolUseContext::new(
                                                cwd.clone(),
                                                read_file_state.clone(),
                                                perm_ctx.mode.clone(),
                                                perm_ctx.clone(),
                                                std::sync::Arc::new({
                                                    let mut options =
                                                    claude_core::tool_use_context_options::ToolUseContextOptions::minimal(&active_model);
                                                    options.session_id =
                                                        Some(api_session_id.clone());
                                                    options
                                                }),
                                                std::sync::Arc::new(
                                                    claude_core::tool_host::NullToolHost,
                                                ),
                                            );
                                            match exec
                                                .call(&tool_input, &ctx, cancel.clone(), None)
                                                .await
                                            {
                                                Ok(data) => {
                                                    // Update cwd when entering/exiting a worktree.
                                                    if !data.is_error {
                                                        match tool_info.name.as_str() {
                                                            "EnterWorktree" => {
                                                                if let Some(path) = data.data
                                                                    ["worktreePath"]
                                                                    .as_str()
                                                                {
                                                                    let old_cwd = cwd.clone();
                                                                    let new_cwd =
                                                                        PathBuf::from(path);
                                                                    claude_core::hooks::fire_cwd_changed(
                                                                    &old_cwd.display().to_string(),
                                                                    &new_cwd.display().to_string(),
                                                                )
                                                                .await;
                                                                    cwd = new_cwd;
                                                                    tracing::info!(
                                                                    "Session cwd switched to worktree: {}",
                                                                    path
                                                                );
                                                                }
                                                            }
                                                            "ExitWorktree" => {
                                                                let old_cwd = cwd.clone();
                                                                let new_cwd = original_cwd.clone();
                                                                claude_core::hooks::fire_cwd_changed(
                                                                &old_cwd.display().to_string(),
                                                                &new_cwd.display().to_string(),
                                                            )
                                                            .await;
                                                                cwd = new_cwd;
                                                                tracing::info!(
                                                                    "Session cwd restored to: {}",
                                                                    original_cwd.display()
                                                                );
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                    let text = format_tool_result_for_model(
                                                        &tool_info.name,
                                                        &data.data,
                                                    );
                                                    (text, data.is_error, data.data)
                                                }
                                                Err(e) => {
                                                    let message = format!("Error: {}", e);
                                                    (
                                                        message.clone(),
                                                        true,
                                                        serde_json::json!({"error": message}),
                                                    )
                                                }
                                            }
                                        }
                                        None => {
                                            let message =
                                            claude_core::tool_result_format::unknown_tool_error_text(
                                                &tool_info.name,
                                            );
                                            (
                                            claude_core::tool_result_format::unknown_tool_error_content(
                                                &tool_info.name,
                                            ),
                                            true,
                                            serde_json::json!({"error": message}),
                                        )
                                        }
                                    }
                                }
                                PermissionDecision::Deny(deny) => (
                                    format!("Permission denied: {}", deny.message),
                                    true,
                                    serde_json::json!({"error": deny.message}),
                                ),
                            };

                            if let Some(runner) = get_global_runner() {
                                if is_error {
                                    let failure = run_post_tool_use_failure_hooks(
                                        &runner,
                                        &tool_info.name,
                                        &tool_info.id,
                                        &tool_input,
                                        &result_text,
                                        None,
                                        Some(permission_mode_hook_name(&perm_ctx.mode)),
                                        None,
                                        None,
                                    )
                                    .await;
                                    for context in &failure.additional_contexts {
                                        query_engine.append_user_context_block(context.clone());
                                    }
                                    if let Some(message) =
                                        hook_blocking_errors_text(&failure.blocking_errors)
                                    {
                                        result_text = message;
                                        is_error = true;
                                    }
                                } else {
                                    let post = run_post_tool_use_hooks(
                                        &runner,
                                        &tool_info.name,
                                        &tool_info.id,
                                        &tool_input,
                                        &result_json,
                                        Some(permission_mode_hook_name(&perm_ctx.mode)),
                                        None,
                                        None,
                                    )
                                    .await;
                                    for context in &post.additional_contexts {
                                        query_engine.append_user_context_block(context.clone());
                                    }
                                    if let Some(updated) = post.updated_mcp_tool_output {
                                        result_json = updated;
                                        result_text = result_json
                                            .as_str()
                                            .unwrap_or(&result_json.to_string())
                                            .to_string();
                                    }
                                    if let Some(message) =
                                        hook_blocking_errors_text(&post.blocking_errors)
                                    {
                                        result_text = message;
                                        is_error = true;
                                    } else if post.prevent_continuation {
                                        result_text = post.stop_reason.unwrap_or_else(|| {
                                            "PostToolUse hook stopped continuation".to_string()
                                        });
                                        is_error = true;
                                    }
                                }
                            }

                            let mut pending_skill_reminder = None;
                            if !is_error {
                                let touched_paths =
                                    dynamic_skill_file_paths(&tool_info.name, &tool_input);
                                if !touched_paths.is_empty() {
                                    let skill_dirs =
                                        claude_core::plugins::skill::discover_skill_dirs_for_paths(
                                            &touched_paths,
                                            &cwd,
                                        );
                                    let mut newly_available =
                                        claude_core::plugins::skill::add_skill_directories(
                                            &skill_dirs,
                                        );
                                    newly_available.extend(
                                    claude_core::plugins::skill::activate_conditional_skills_for_paths(
                                        &touched_paths,
                                        &cwd,
                                    ),
                                );
                                    if !newly_available.is_empty() {
                                        let mut seen = skills
                                            .iter()
                                            .map(|skill| skill.name.clone())
                                            .collect::<std::collections::HashSet<_>>();
                                        let mut unique_new = Vec::new();
                                        for skill in newly_available {
                                            if !skill.disable_model_invocation
                                                && seen.insert(skill.name.clone())
                                            {
                                                claude_tools::skill_tool::register_discovered_skill(
                                                    &skill,
                                                );
                                                unique_new.push(skill.clone());
                                                skills.push(skill);
                                            }
                                        }
                                        if !unique_new.is_empty() {
                                            pending_skill_reminder =
                                                Some(skills_reminder_block(&unique_new));
                                        }
                                    }
                                }
                            }

                            let result_content = if is_error {
                                serde_json::Value::String(result_text.clone())
                            } else {
                                format_tool_result_content_for_model(&tool_info.name, &result_json)
                            };
                            if !is_error && tool_info.name == "StructuredOutput" {
                                structured_output = result_json.get("structured_output").cloned();
                            }

                            if cli.output_format == OutputFormat::StreamJson {
                                emit_stream_json(stream_json_user_tool_result_event(
                                    vec![serde_json::json!({
                                    "type": "tool_result",
                                    "tool_use_id": tool_info.id,
                                    "content": result_content.clone(),
                                    "is_error": is_error,
                                    })],
                                    vec![result_json.clone()],
                                    &session_id,
                                ));
                            }
                            let max_result_size_chars = tools
                                .get(&tool_info.name)
                                .map(|tool| tool.max_result_size_chars())
                                .unwrap_or(100_000);
                            query_engine.add_tool_result_content_with_error_field_and_name(
                                &tool_info.id,
                                Some(&tool_info.name),
                                Some(max_result_size_chars),
                                result_content,
                                is_error,
                                is_error || tool_info.name == "Bash",
                            );
                            if let Some(reminder) = pending_skill_reminder {
                                query_engine.add_user_context_message(reminder);
                            }
                        }
                        if let Some(max_turns) = cli.max_turns.filter(|max_turns| *max_turns > 0) {
                            if num_turns >= max_turns {
                                break PrintTerminalOutcome::MaxTurns {
                                    max_turns,
                                    turn_count: num_turns + 1,
                                };
                            }
                        }
                        // Continue the loop to call run_turn again with the tool results
                    }
                }
            };

            if cli.output_format == OutputFormat::Json {
                println!(
                    "{}",
                    match terminal_outcome {
                        PrintTerminalOutcome::Completed { .. } => {
                            let mut value = serde_json::json!({
                            "type": "result",
                            "subtype": "success",
                            "is_error": false,
                            "result": final_text,
                            });
                            if let Some(ref structured) = structured_output {
                                value["structured_output"] = structured.clone();
                            }
                            serde_json::to_string(&value)?
                        }
                        PrintTerminalOutcome::MaxTurns { max_turns, .. } =>
                            serde_json::to_string(&serde_json::json!({
                                "type": "result",
                                "subtype": "error_max_turns",
                                "is_error": true,
                                "errors": [format!("Reached maximum number of turns ({max_turns})")],
                            }))?,
                        PrintTerminalOutcome::MaxBudgetUsd { max_budget_usd } =>
                            serde_json::to_string(&serde_json::json!({
                                "type": "result",
                                "subtype": "error_max_budget_usd",
                                "is_error": true,
                                "errors": [format!("Reached maximum budget (${max_budget_usd})")],
                            }))?,
                        PrintTerminalOutcome::StructuredOutputRetries { max_retries } =>
                            serde_json::to_string(&serde_json::json!({
                                "type": "result",
                                "subtype": "error_max_structured_output_retries",
                                "is_error": true,
                                "errors": [format!("Failed to produce valid structured output after {max_retries} retries")],
                            }))?,
                    }
                );
            } else if cli.output_format == OutputFormat::StreamJson {
                let total_cost_usd = total_cost_for_usage(&model, total_usage.as_ref());
                let context_window =
                    if model_display.contains("[1M]") || model_display.contains("[1m]") {
                        1_000_000
                    } else {
                        claude_core::compact::compactor::default_context_window()
                    };
                let duration_ms = started_at.elapsed().as_millis();
                let meta_num_turns = match terminal_outcome {
                    PrintTerminalOutcome::Completed { .. } => num_turns,
                    PrintTerminalOutcome::MaxTurns { turn_count, .. } => turn_count,
                    PrintTerminalOutcome::MaxBudgetUsd { .. } => num_turns,
                    PrintTerminalOutcome::StructuredOutputRetries { .. } => num_turns,
                };
                let meta = StreamJsonResultMeta {
                    duration_ms,
                    num_turns: meta_num_turns,
                    stop_reason: match &terminal_outcome {
                        PrintTerminalOutcome::Completed { stop_reason } => stop_reason,
                        PrintTerminalOutcome::MaxTurns { .. } => "tool_use",
                        PrintTerminalOutcome::MaxBudgetUsd { .. } => "end_turn",
                        PrintTerminalOutcome::StructuredOutputRetries { .. } => "end_turn",
                    },
                    total_usage: total_usage.as_ref(),
                    latest_usage: latest_usage.as_ref(),
                    model_display: &model_display,
                    max_tokens: resolve_max_output_tokens(&model, &settings),
                    context_window,
                    total_cost_usd,
                };
                match terminal_outcome {
                    PrintTerminalOutcome::Completed { .. } => {
                        let mut event =
                            stream_json_result_event_with_meta(&final_text, &session_id, meta);
                        if let Some(ref structured) = structured_output {
                            event["structured_output"] = structured.clone();
                        }
                        emit_stream_json(event);
                    }
                    PrintTerminalOutcome::MaxTurns { max_turns, .. } => {
                        emit_stream_json(stream_json_max_turns_event_with_meta(
                            max_turns,
                            &session_id,
                            meta,
                        ));
                    }
                    PrintTerminalOutcome::MaxBudgetUsd { max_budget_usd } => {
                        emit_stream_json(stream_json_max_budget_usd_event_with_meta(
                            max_budget_usd,
                            &session_id,
                            meta,
                        ));
                    }
                    PrintTerminalOutcome::StructuredOutputRetries { max_retries } => {
                        let mut event =
                            stream_json_max_turns_event_with_meta(max_retries, &session_id, meta);
                        event["subtype"] = serde_json::json!("error_max_structured_output_retries");
                        event["terminal_reason"] =
                            serde_json::json!("max_structured_output_retries");
                        event["errors"] = serde_json::json!([format!(
                            "Failed to produce valid structured output after {max_retries} retries"
                        )]);
                        emit_stream_json(event);
                    }
                }
            } else {
                match terminal_outcome {
                    PrintTerminalOutcome::Completed { .. } => {
                        if !final_text.is_empty() {
                            println!("{final_text}");
                        }
                    }
                    PrintTerminalOutcome::MaxTurns { max_turns, .. } => {
                        println!("Error: Reached max turns ({max_turns})");
                    }
                    PrintTerminalOutcome::MaxBudgetUsd { max_budget_usd } => {
                        println!("Error: Exceeded USD budget ({max_budget_usd})");
                    }
                    PrintTerminalOutcome::StructuredOutputRetries { max_retries } => {
                        println!(
                        "Error: Failed to produce valid structured output after {max_retries} retries"
                    );
                    }
                }
            }

            if let Some(event_id) = current_delivery_event_id.as_deref() {
                report_remote_ccr_delivery(event_id, "processed");
            }

            // Gracefully disconnect MCP servers
            let Some(rx) = remote_sdk_rx.as_mut() else {
                let mgr = mcp_manager.read().await;
                mgr.disconnect_all().await;

                return Ok(());
            };
            if let Some(next_prompt) = queued_remote_prompts.pop_front() {
                pending_remote_prompt = next_prompt;
                continue;
            }
            loop {
                match rx.recv().await {
                    Some(RemoteSdkInbound::User(next_prompt)) => {
                        pending_remote_prompt = next_prompt;
                        break;
                    }
                    Some(RemoteSdkInbound::ControlResponse(_)) => {
                        continue;
                    }
                    Some(RemoteSdkInbound::ControlRequest(control_request)) => {
                        apply_remote_control_request(
                            control_request,
                            &mut query_engine,
                            &mut perm_ctx,
                            &mut active_model,
                            &mut active_model_display,
                        )
                        .await?;
                        continue;
                    }
                    Some(RemoteSdkInbound::EndSession) | None => {
                        let mgr = mcp_manager.read().await;
                        mgr.disconnect_all().await;
                        return Ok(());
                    }
                }
            }
            continue;
        }
    }

    // Interactive TUI mode
    let mut app = claude_tui::app::App::new()?;
    app.set_model_name(&model_display);
    app.set_skills(skills);
    app.run_with_engine(
        query_engine,
        tools,
        cancel.clone(),
        initial_permission_context,
        api_session_id.clone(),
    )
    .await?;

    // Gracefully disconnect MCP servers
    let mgr = mcp_manager.read().await;
    mgr.disconnect_all().await;

    Ok(())
}

fn initialize_entrypoint(is_interactive_session: bool) {
    if std::env::var_os("USER_TYPE").is_none() {
        std::env::set_var("USER_TYPE", "external");
    }
    if std::env::var_os("CLAUDE_CODE_ENTRYPOINT").is_none() {
        std::env::set_var(
            "CLAUDE_CODE_ENTRYPOINT",
            if is_interactive_session {
                "cli"
            } else {
                "sdk-cli"
            },
        );
    }
}

#[cfg(test)]
mod remote_control_tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn remote_control_help_matches_ts_fast_path_shape() {
        let help = remote_control_help_text();
        assert!(help.contains("Remote Control - Connect your local environment to claude.ai/code"));
        assert!(help.contains("USAGE\n  claude remote-control [options]"));
        assert!(help.contains("--name <name>"));
        assert!(help.contains("--spawn <mode>"));
        assert!(help.contains("--[no-]create-session-in-dir"));
        assert!(help.contains("claude.ai/code (https://claude.ai/code)"));
    }

    #[test]
    fn remote_control_subcommand_accepts_ts_bridge_options() {
        let cli = Cli::parse_from([
            "claude-rs",
            "remote-control",
            "--name",
            "demo",
            "--remote-control-session-name-prefix",
            "host",
            "--permission-mode",
            "default",
            "--debug-file",
            "/tmp/bridge.log",
            "--sandbox",
            "--session-timeout=15",
            "--verbose",
            "--spawn",
            "same-dir",
            "--capacity",
            "2",
            "--no-create-session-in-dir",
        ]);

        match cli.command {
            Some(SubCommand::RemoteControl {
                name,
                remote_control_session_name_prefix,
                permission_mode,
                debug_file,
                sandbox,
                no_sandbox,
                session_timeout,
                verbose,
                spawn,
                capacity,
                session_id,
                continue_session,
                create_session_in_dir,
                no_create_session_in_dir,
            }) => {
                assert_eq!(name.as_deref(), Some("demo"));
                assert_eq!(remote_control_session_name_prefix.as_deref(), Some("host"));
                assert_eq!(permission_mode.as_deref(), Some("default"));
                assert_eq!(
                    debug_file.as_deref(),
                    Some(std::path::Path::new("/tmp/bridge.log"))
                );
                assert!(sandbox);
                assert!(!no_sandbox);
                assert_eq!(session_timeout.as_deref(), Some("15"));
                assert!(verbose);
                assert_eq!(spawn.as_deref(), Some("same-dir"));
                assert_eq!(capacity.as_deref(), Some("2"));
                assert_eq!(session_id, None);
                assert!(!continue_session);
                assert!(!create_session_in_dir);
                assert!(no_create_session_in_dir);
            }
            _ => panic!("expected remote-control command"),
        }
    }

    #[test]
    fn mcp_xaa_subcommands_match_ts_shape() {
        let cli = Cli::parse_from([
            "claude-rs",
            "mcp",
            "xaa",
            "setup",
            "--issuer",
            "https://idp.example.com",
            "--client-id",
            "client",
            "--client-secret",
            "--callback-port",
            "8080",
        ]);
        match cli.command {
            Some(SubCommand::Mcp {
                command:
                    McpCommand::Xaa {
                        command:
                            McpXaaCommand::Setup {
                                issuer,
                                client_id,
                                client_secret,
                                callback_port,
                            },
                    },
            }) => {
                assert_eq!(issuer, "https://idp.example.com");
                assert_eq!(client_id, "client");
                assert!(client_secret);
                assert_eq!(callback_port, Some(8080));
            }
            _ => panic!("expected mcp xaa setup command"),
        }

        let cli = Cli::parse_from([
            "claude-rs",
            "mcp",
            "xaa",
            "login",
            "--force",
            "--id-token",
            "jwt",
        ]);
        match cli.command {
            Some(SubCommand::Mcp {
                command:
                    McpCommand::Xaa {
                        command: McpXaaCommand::Login { force, id_token },
                    },
            }) => {
                assert!(force);
                assert_eq!(id_token.as_deref(), Some("jwt"));
            }
            _ => panic!("expected mcp xaa login command"),
        }
    }

    #[test]
    fn remote_control_fast_path_argument_validation_matches_ts() {
        assert!(validate_remote_control_fast_path_args(&[
            "--name".into(),
            "demo".into(),
            "--debug-file=/tmp/bridge.log".into(),
            "--sandbox".into(),
            "--session-timeout".into(),
            "15".into(),
            "--permission-mode=default".into(),
            "--remote-control-session-name-prefix=host".into(),
            "--spawn=same-dir".into(),
            "--capacity".into(),
            "2".into(),
            "--create-session-in-dir".into(),
        ])
        .is_ok());

        assert_eq!(
            validate_remote_control_fast_path_args(&["foo".into()]).unwrap_err(),
            "Unknown argument: foo\nRun 'claude remote-control --help' for usage."
        );
        assert_eq!(
            validate_remote_control_fast_path_args(&["--spawn".into()]).unwrap_err(),
            "--spawn requires one of: session, same-dir, worktree (got: <missing>)"
        );
        assert_eq!(
            validate_remote_control_fast_path_args(&["--capacity".into()]).unwrap_err(),
            "--capacity requires a positive integer (got: <missing>)"
        );
        assert_eq!(
            validate_remote_control_fast_path_args(&[
                "--spawn=session".into(),
                "--spawn=worktree".into()
            ])
            .unwrap_err(),
            "--spawn may only be specified once"
        );
        assert_eq!(
            validate_remote_control_fast_path_args(&["--capacity=1".into(), "--capacity=2".into()])
                .unwrap_err(),
            "--capacity may only be specified once"
        );
        assert_eq!(
            validate_remote_control_fast_path_args(&["--spawn=session".into(), "--capacity=2".into()])
                .unwrap_err(),
            "--capacity cannot be used with --spawn=session (single-session mode has fixed capacity 1)."
        );
    }

    #[test]
    fn remote_control_bridge_option_validation_matches_ts() {
        assert_eq!(
            validate_remote_control_bridge_options(Some("session"), None, None, false, false)
                .unwrap(),
            (Some("single-session"), None)
        );
        assert_eq!(
            validate_remote_control_bridge_options(Some("same-dir"), Some("2"), None, false, false)
                .unwrap(),
            (Some("same-dir"), Some(2))
        );
        assert_eq!(
            validate_remote_control_bridge_options(
                Some("worktree"),
                Some("32"),
                None,
                false,
                false
            )
            .unwrap(),
            (Some("worktree"), Some(32))
        );

        assert_eq!(
            validate_remote_control_bridge_options(Some("bad"), None, None, false, false)
                .unwrap_err(),
            "--spawn requires one of: session, same-dir, worktree (got: bad)"
        );
        assert_eq!(
            validate_remote_control_bridge_options(None, Some("0"), None, false, false)
                .unwrap_err(),
            "--capacity requires a positive integer (got: 0)"
        );
        assert_eq!(
            validate_remote_control_bridge_options(Some("session"), Some("2"), None, false, false)
                .unwrap_err(),
            "--capacity cannot be used with --spawn=session (single-session mode has fixed capacity 1)."
        );
        assert_eq!(
            validate_remote_control_bridge_options(None, None, Some("session_abc"), false, false)
                .unwrap(),
            (None, None)
        );
        assert_eq!(
            validate_remote_control_bridge_options(
                Some("same-dir"),
                None,
                Some("session_abc"),
                false,
                false
            )
            .unwrap_err(),
            "--session-id and --continue cannot be used with --spawn, --capacity, or --create-session-in-dir."
        );
        assert_eq!(
            validate_remote_control_bridge_options(None, None, None, true, false).unwrap(),
            (None, None)
        );
        assert_eq!(
            validate_remote_control_bridge_options(None, None, Some("session_abc"), true, false)
                .unwrap_err(),
            "--session-id and --continue cannot be used together."
        );
    }

    #[test]
    fn remote_session_child_flags_are_accepted_like_ts_runner() {
        let cli = Cli::parse_from([
            "claude-rs",
            "--print",
            "--sdk-url",
            "wss://api.example.com/v1/session_ingress/ws/session_abc",
            "--session-id",
            "session_abc",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--replay-user-messages",
            "--debug-file",
            "/tmp/bridge-session.log",
            "--permission-mode",
            "default",
        ]);

        assert!(cli.print);
        assert_eq!(
            cli.sdk_url.as_deref(),
            Some("wss://api.example.com/v1/session_ingress/ws/session_abc")
        );
        assert_eq!(cli.session_id.as_deref(), Some("session_abc"));
        assert_eq!(cli.input_format, InputFormat::StreamJson);
        assert_eq!(cli.output_format, OutputFormat::StreamJson);
        assert!(cli.replay_user_messages);
        assert_eq!(
            cli.debug_file.as_deref(),
            Some(std::path::Path::new("/tmp/bridge-session.log"))
        );
        assert_eq!(cli.permission_mode.as_deref(), Some("default"));
    }

    #[test]
    fn remote_session_filename_and_debug_path_match_ts_shape() {
        assert_eq!(safe_filename_id("session/../abc:123"), "session____abc_123");
        assert_eq!(
            debug_file_for_session(Some("/tmp/bridge.log"), "session/abc", false).as_deref(),
            Some("/tmp/bridge-session_abc.log")
        );
        assert_eq!(
            debug_file_for_session(Some("/tmp/bridge"), "session/abc", false).as_deref(),
            Some("/tmp/bridge-session_abc")
        );
        assert!(debug_file_for_session(None, "session/abc", false).is_none());
    }

    #[test]
    fn remote_hybrid_post_url_matches_ts_conversion() {
        assert_eq!(
            convert_ws_url_to_session_ingress_post_url(
                "wss://api.example.com/v2/session_ingress/ws/session-123"
            )
            .unwrap(),
            "https://api.example.com/v2/session_ingress/session/session-123/events"
        );
        assert_eq!(
            convert_ws_url_to_session_ingress_post_url(
                "ws://127.0.0.1:8787/v1/session_ingress/ws/session-123?debug=1"
            )
            .unwrap(),
            "http://127.0.0.1:8787/v1/session_ingress/session/session-123/events?debug=1"
        );
    }

    #[test]
    fn remote_ccr_sse_url_matches_ts_transport_utils() {
        assert_eq!(
            ccr_v2_sse_url("https://api.example.com/v1/code/sessions/session-123").unwrap(),
            "https://api.example.com/v1/code/sessions/session-123/worker/events/stream"
        );
    }

    #[test]
    fn remote_ccr_heartbeat_url_matches_ts_transport() {
        let config = RemotePostConfig::CcrV2 {
            session_url: "https://api.example.com/v1/code/sessions/session-123".to_string(),
            events_url: "https://api.example.com/v1/code/sessions/session-123/worker/events"
                .to_string(),
            delivery_url:
                "https://api.example.com/v1/code/sessions/session-123/worker/events/delivery"
                    .to_string(),
            internal_events_url:
                "https://api.example.com/v1/code/sessions/session-123/worker/internal-events"
                    .to_string(),
            worker_url: "https://api.example.com/v1/code/sessions/session-123/worker".to_string(),
            worker_epoch: 7,
        };
        assert_eq!(
            ccr_heartbeat_url(&config).unwrap(),
            "https://api.example.com/v1/code/sessions/session-123/worker/heartbeat"
        );
    }

    #[test]
    fn remote_ccr_delivery_body_matches_ts_transport() {
        let updates = vec![
            CcrDeliveryUpdate {
                event_id: "event-1".to_string(),
                status: "received",
            },
            CcrDeliveryUpdate {
                event_id: "event-2".to_string(),
                status: "processed",
            },
        ];
        assert_eq!(
            ccr_delivery_body(7, &updates),
            serde_json::json!({
                "worker_epoch": 7,
                "updates": [
                    {"event_id": "event-1", "status": "received"},
                    {"event_id": "event-2", "status": "processed"},
                ],
            })
        );
    }

    #[test]
    fn remote_ccr_internal_events_body_matches_ts_transport() {
        let events = vec![
            InternalTranscriptEvent {
                payload: serde_json::json!({
                    "type": "user",
                    "message": {"content": "hi"},
                    "uuid": "uuid-1",
                }),
                is_compaction: false,
                agent_id: None,
            },
            InternalTranscriptEvent {
                payload: serde_json::json!({
                    "type": "compact_boundary",
                    "uuid": "uuid-2",
                }),
                is_compaction: true,
                agent_id: Some("agent-1".to_string()),
            },
        ];

        assert_eq!(
            ccr_internal_events_body(7, &events),
            serde_json::json!({
                "worker_epoch": 7,
                "events": [
                    {
                        "payload": {
                            "type": "user",
                            "message": {"content": "hi"},
                            "uuid": "uuid-1",
                        },
                    },
                    {
                        "payload": {
                            "type": "compact_boundary",
                            "uuid": "uuid-2",
                        },
                        "is_compaction": true,
                        "agent_id": "agent-1",
                    },
                ],
            })
        );
    }

    #[test]
    fn remote_ccr_internal_events_read_url_matches_ts_transport() {
        assert_eq!(
            ccr_internal_events_read_url(
                "https://api.example.com/v1/code/sessions/session-123?ignored=1",
                false,
                None,
            )
            .unwrap(),
            "https://api.example.com/v1/code/sessions/session-123/worker/internal-events"
        );
        assert_eq!(
            ccr_internal_events_read_url(
                "https://api.example.com/v1/code/sessions/session-123",
                true,
                Some("cursor-1"),
            )
            .unwrap(),
            "https://api.example.com/v1/code/sessions/session-123/worker/internal-events?subagents=true&cursor=cursor-1"
        );
    }

    #[test]
    fn remote_ccr_worker_read_url_matches_ts_transport() {
        assert_eq!(
            ccr_worker_read_url("https://api.example.com/v1/code/sessions/session-123?ignored=1")
                .unwrap(),
            "https://api.example.com/v1/code/sessions/session-123/worker"
        );
    }

    #[test]
    fn remote_ccr_subagent_internal_events_group_by_agent_id() {
        let grouped = group_ccr_subagent_internal_events(vec![
            CcrInternalEvent {
                agent_id: Some("agent-1".to_string()),
                payload: serde_json::json!({"uuid": "1"}),
            },
            CcrInternalEvent {
                agent_id: None,
                payload: serde_json::json!({"uuid": "ignored"}),
            },
            CcrInternalEvent {
                agent_id: Some("agent-2".to_string()),
                payload: serde_json::json!({"uuid": "2"}),
            },
            CcrInternalEvent {
                agent_id: Some("agent-1".to_string()),
                payload: serde_json::json!({"uuid": "3"}),
            },
        ]);

        assert_eq!(grouped.len(), 2);
        assert_eq!(
            grouped.get("agent-1").unwrap(),
            &vec![
                serde_json::json!({"uuid": "1"}),
                serde_json::json!({"uuid": "3"}),
            ]
        );
        assert_eq!(
            grouped.get("agent-2").unwrap(),
            &vec![serde_json::json!({"uuid": "2"})]
        );
    }

    #[test]
    fn remote_ccr_stream_text_deltas_coalesce_like_ts_transport() {
        let mut state = RemoteStreamAccumulator::default();
        let events = vec![
            serde_json::json!({
                "type": "stream_event",
                "uuid": "start-1",
                "session_id": "session-1",
                "parent_tool_use_id": null,
                "event": {
                    "type": "message_start",
                    "message": {"id": "msg-1"}
                }
            }),
            serde_json::json!({
                "type": "stream_event",
                "uuid": "delta-1",
                "session_id": "session-1",
                "parent_tool_use_id": null,
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "hel"}
                }
            }),
            serde_json::json!({
                "type": "stream_event",
                "uuid": "delta-2",
                "session_id": "session-1",
                "parent_tool_use_id": null,
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "lo"}
                }
            }),
        ];

        let out = accumulate_remote_stream_events(events, &mut state);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["event"]["type"], "message_start");
        assert_eq!(out[1]["uuid"], "delta-1");
        assert_eq!(out[1]["event"]["delta"]["text"], "hello");

        let more = accumulate_remote_stream_events(
            vec![serde_json::json!({
                "type": "stream_event",
                "uuid": "delta-3",
                "session_id": "session-1",
                "parent_tool_use_id": null,
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "!"}
                }
            })],
            &mut state,
        );
        assert_eq!(more[0]["event"]["delta"]["text"], "hello!");

        clear_remote_stream_accumulator_for_assistant(
            &mut state,
            &serde_json::json!({
                "type": "assistant",
                "session_id": "session-1",
                "parent_tool_use_id": null,
                "message": {"id": "msg-1"}
            }),
        );
        assert!(state.by_message.is_empty());
        assert!(state.scope_to_message.is_empty());
    }

    #[test]
    fn remote_ccr_sse_parser_preserves_event_id_for_delivery_ack() {
        let mut buffer = "event: client_event\ndata: {\"event_id\":\"event-1\",\"sequence_num\":1,\"event_type\":\"user\",\"source\":\"web\",\"payload\":{\"type\":\"user\",\"message\":{\"content\":\"hi\"}},\"created_at\":\"now\"}\n\n".to_string();
        let events = parse_sse_payloads(&mut buffer);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id.as_deref(), Some("event-1"));
        assert_eq!(events[0].payload.as_ref().unwrap()["type"], "user");
    }

    #[test]
    fn remote_ccr_worker_state_matches_ts_event_transitions() {
        assert_eq!(
            ccr_worker_state_for_event(&serde_json::json!({
                "type": "user",
            }))
            .unwrap()["worker_status"],
            "running"
        );
        assert_eq!(
            ccr_worker_state_for_event(&serde_json::json!({
                "type": "result",
            }))
            .unwrap()["worker_status"],
            "idle"
        );
        assert_eq!(
            ccr_worker_state_for_event(&serde_json::json!({
                "type": "control_request",
                "request_id": "req-1",
                "request": {
                    "subtype": "can_use_tool",
                    "tool_name": "Bash",
                    "decision_reason": "Bash needs permission"
                }
            }))
            .unwrap(),
            serde_json::json!({
                "worker_status": "requires_action",
                "requires_action_details": {
                    "tool_name": "Bash",
                    "action_description": "Bash needs permission",
                    "request_id": "req-1",
                },
            })
        );
    }

    #[test]
    fn remote_ccr_worker_patch_mirrors_pending_action_metadata() {
        assert_eq!(
            ccr_worker_patch_for_event(&serde_json::json!({
                "type": "control_request",
                "request_id": "req-1",
                "request": {
                    "subtype": "can_use_tool",
                    "tool_name": "Bash",
                    "decision_reason": "Bash needs permission"
                }
            }))
            .unwrap()["external_metadata"]["pending_action"]["request_id"],
            "req-1"
        );
        assert_eq!(
            ccr_worker_patch_for_event(&serde_json::json!({
                "type": "result",
            }))
            .unwrap()["external_metadata"],
            serde_json::json!({"pending_action": null})
        );
    }

    #[test]
    fn remote_external_permission_mode_matches_ts_config() {
        use claude_core::permissions::types::PermissionMode;

        assert_eq!(
            to_external_permission_mode(&PermissionMode::Default),
            "default"
        );
        assert_eq!(
            to_external_permission_mode(&PermissionMode::AcceptEdits),
            "acceptEdits"
        );
        assert_eq!(
            to_external_permission_mode(&PermissionMode::Auto),
            "default"
        );
        assert_eq!(to_external_permission_mode(&PermissionMode::Plan), "plan");
        assert_eq!(
            to_external_permission_mode(&PermissionMode::BypassPermissions),
            "bypassPermissions"
        );
        assert_eq!(
            to_external_permission_mode(&PermissionMode::DontAsk),
            "dontAsk"
        );
        assert_eq!(
            to_external_permission_mode(&PermissionMode::Bubble),
            "default"
        );
    }

    #[test]
    fn remote_token_refresh_delay_matches_ts_buffer() {
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let token = "sk-ant-si-eyJhbGciOiJub25lIn0.eyJleHAiOjE2MDB9.c2ln";
        assert_eq!(
            remote_token_refresh_delay(&token, now),
            Some(std::time::Duration::from_secs(300))
        );

        let within_buffer = "sk-ant-si-eyJhbGciOiJub25lIn0.eyJleHAiOjExMDB9.c2ln";
        assert_eq!(
            remote_token_refresh_delay(&within_buffer, now),
            Some(std::time::Duration::ZERO)
        );

        assert_eq!(remote_token_refresh_delay("not-a-jwt", now), None);
    }

    #[test]
    fn stream_json_seed_preserves_runtime_control_requests() {
        let seed = parse_stream_json_stdin(
            r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"set_model","model":"sonnet"}}"#,
        )
        .unwrap();
        assert_eq!(seed.control_requests.len(), 1);
        assert_eq!(
            seed.control_requests[0]["request"]["subtype"],
            serde_json::json!("set_model")
        );
    }

    #[test]
    fn restored_worker_metadata_applies_model_and_permission_mode_like_ts() {
        let mut cli = Cli::parse_from(["claude-rs", "--print", "hi"]);
        apply_restored_worker_metadata_to_cli(
            &mut cli,
            &serde_json::json!({
                "model": "claude-opus-4-7[1m]",
                "permission_mode": "acceptEdits",
                "pending_action": null,
            }),
        );
        assert_eq!(cli.model.as_deref(), Some("claude-opus-4-7[1m]"));
        assert_eq!(cli.permission_mode.as_deref(), Some("acceptEdits"));
    }

    #[test]
    fn remote_control_request_routes_to_main_loop_queue() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        handle_remote_sdk_non_user_line(
            &serde_json::json!({
                "type": "control_request",
                "request_id": "req-1",
                "request": {"subtype": "set_permission_mode", "mode": "plan"}
            }),
            Some(&tx),
        )
        .unwrap();
        match rx.try_recv().unwrap() {
            RemoteSdkInbound::ControlRequest(value) => {
                assert_eq!(
                    value["request"]["subtype"],
                    serde_json::json!("set_permission_mode")
                );
            }
            _ => panic!("expected control request"),
        }
    }

    #[test]
    fn remote_control_response_success_shape_matches_structured_io() {
        assert_eq!(
            remote_control_response_success("req-1", serde_json::json!({})),
            serde_json::json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": "req-1",
                    "response": {},
                },
            })
        );
    }

    #[test]
    fn direct_connect_rewrite_matches_ts_cc_url_flow() {
        let args = vec![
            "claude-rs".to_string(),
            "cc://connect?serverUrl=http%3A%2F%2F127.0.0.1%3A4141&authToken=tok".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        let (rewritten, parsed, skip) = rewrite_direct_connect_args(args).unwrap();
        assert_eq!(rewritten, vec!["claude-rs"]);
        assert!(skip);
        let parsed = parsed.unwrap();
        assert_eq!(parsed.server_url, "http://127.0.0.1:4141");
        assert_eq!(parsed.auth_token.as_deref(), Some("tok"));
    }

    #[test]
    fn direct_connect_print_rewrites_to_internal_open_subcommand() {
        let args = vec![
            "claude-rs".to_string(),
            "-p".to_string(),
            "hello".to_string(),
            "cc://connect?serverUrl=http%3A%2F%2F127.0.0.1%3A4141".to_string(),
        ];
        let (rewritten, parsed, skip) = rewrite_direct_connect_args(args).unwrap();
        assert!(!skip);
        assert!(parsed.is_some());
        assert_eq!(rewritten[1], "open");
        assert!(rewritten[2].starts_with("cc://"));
        assert_eq!(rewritten[3], "-p");
        assert_eq!(rewritten[4], "hello");
    }

    #[test]
    fn remote_permission_request_shape_matches_sdk_can_use_tool() {
        assert_eq!(
            remote_permission_request(
                "req-1",
                "Bash",
                "toolu_1",
                &serde_json::json!({"command": "cargo build"}),
                "Bash needs permission",
                &None,
                &None,
            ),
            serde_json::json!({
                "type": "control_request",
                "request_id": "req-1",
                "request": {
                    "subtype": "can_use_tool",
                    "tool_name": "Bash",
                    "input": {"command": "cargo build"},
                    "decision_reason": "Bash needs permission",
                    "tool_use_id": "toolu_1",
                },
            })
        );
    }

    #[test]
    fn remote_permission_response_parses_allow_and_deny() {
        let allow = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "req-1",
                "response": {
                    "behavior": "allow",
                    "updatedInput": {"command": "cargo test"}
                }
            }
        });
        let (allowed, updated, updated_permissions, message) =
            parse_remote_permission_response(&allow).unwrap();
        assert!(allowed);
        assert_eq!(updated, Some(serde_json::json!({"command": "cargo test"})));
        assert!(updated_permissions.is_none());
        assert!(message.is_empty());

        let deny = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "req-1",
                "response": {
                    "behavior": "deny",
                    "message": "No"
                }
            }
        });
        let (allowed, updated, updated_permissions, message) =
            parse_remote_permission_response(&deny).unwrap();
        assert!(!allowed);
        assert_eq!(updated, None);
        assert!(updated_permissions.is_none());
        assert_eq!(message, "No");
    }

    #[test]
    fn remote_control_is_hidden_from_root_help_like_ts_commander_registration() {
        let mut command = Cli::command();
        let help = command.render_help().to_string();
        assert!(!help.contains("remote-control"));
        assert!(!help.contains("Connect your local environment for remote-control"));
    }
}
