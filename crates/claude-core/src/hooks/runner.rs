use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::future::join_all;
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};

use super::aggregation::aggregate_hook_results;
use super::matching::{get_matching_hooks, resolve_match_query, MatchedHook};
use super::types::*;

// ============================================================================
// Constants
// ============================================================================

/// Default timeout for tool hook execution: 10 minutes.
const TOOL_HOOK_EXECUTION_TIMEOUT_MS: u64 = 10 * 60 * 1000;

/// Default timeout for SessionEnd hooks: 1.5 seconds.
/// Overridable via CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS env var.
const SESSION_END_HOOK_TIMEOUT_MS_DEFAULT: u64 = 1500;

/// Default timeout for HTTP hooks: 30 seconds.
const DEFAULT_HTTP_HOOK_TIMEOUT_MS: u64 = 30 * 1000;

// ============================================================================
// HookRunner — the main entry point
// ============================================================================

/// The hook execution engine.
///
/// Holds the hooks configuration and provides methods to run hooks
/// for various events with proper matching, execution, and aggregation.
pub struct HookRunner {
    settings: HooksSettings,
    /// The current working directory for hook execution.
    cwd: String,
    /// Session ID.
    session_id: String,
    /// Transcript path for the session.
    transcript_path: String,
}

impl HookRunner {
    /// Create a new HookRunner from parsed HooksSettings.
    pub fn new(
        settings: HooksSettings,
        cwd: String,
        session_id: String,
        transcript_path: String,
    ) -> Self {
        Self {
            settings,
            cwd,
            session_id,
            transcript_path,
        }
    }

    /// Create a HookRunner by parsing a raw serde_json::Value settings blob.
    ///
    /// The value is expected to have a top-level "hooks" key mapping to the
    /// HooksSettings structure.
    pub fn from_settings(
        settings_value: &serde_json::Value,
        cwd: String,
        session_id: String,
        transcript_path: String,
    ) -> Self {
        let hooks_settings = settings_value
            .get("hooks")
            .and_then(|h| serde_json::from_value::<HooksSettings>(h.clone()).ok())
            .unwrap_or_default();
        Self::new(hooks_settings, cwd, session_id, transcript_path)
    }

    /// Update the current working directory (e.g., on CwdChanged).
    pub fn set_cwd(&mut self, cwd: String) {
        self.cwd = cwd;
    }

    /// Get the SessionEnd hook timeout in milliseconds.
    /// Honors the CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS env var.
    pub fn session_end_hook_timeout_ms() -> u64 {
        std::env::var("CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(SESSION_END_HOOK_TIMEOUT_MS_DEFAULT)
    }

    /// Create the base hook input fields shared by all hook events.
    fn create_base_hook_input(
        &self,
        permission_mode: Option<&str>,
        agent_id: Option<&str>,
        agent_type: Option<&str>,
    ) -> serde_json::Value {
        let mut base = serde_json::json!({
            "session_id": self.session_id,
            "transcript_path": self.transcript_path,
            "cwd": self.cwd,
        });
        if let Some(pm) = permission_mode {
            base["permission_mode"] = serde_json::Value::String(pm.to_string());
        }
        if let Some(aid) = agent_id {
            base["agent_id"] = serde_json::Value::String(aid.to_string());
        }
        if let Some(at) = agent_type {
            base["agent_type"] = serde_json::Value::String(at.to_string());
        }
        base
    }

    /// Build the full hook input JSON for a given event with additional fields.
    fn build_hook_input(
        &self,
        event: &HookEvent,
        extra_fields: serde_json::Value,
        permission_mode: Option<&str>,
        agent_id: Option<&str>,
        agent_type: Option<&str>,
    ) -> serde_json::Value {
        let mut input = self.create_base_hook_input(permission_mode, agent_id, agent_type);
        input["hook_event_name"] = serde_json::Value::String(event.as_str().to_string());
        if let serde_json::Value::Object(map) = extra_fields {
            for (k, v) in map {
                input[k] = v;
            }
        }
        input
    }

    // ========================================================================
    // Public API: run_hooks (inside REPL context)
    // ========================================================================

    /// Execute hooks for a given event with the provided input fields.
    ///
    /// This is the primary entry point used inside the REPL loop.
    /// Hooks run in parallel; results are aggregated with proper precedence.
    ///
    /// Returns an `AggregatedHookResult` containing all blocking errors,
    /// permission decisions, additional contexts, etc.
    pub async fn run_hooks(
        &self,
        event: &HookEvent,
        extra_fields: serde_json::Value,
        permission_mode: Option<&str>,
        agent_id: Option<&str>,
        agent_type: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> AggregatedHookResult {
        let hook_input =
            self.build_hook_input(event, extra_fields, permission_mode, agent_id, agent_type);

        let timeout = timeout_ms.unwrap_or(TOOL_HOOK_EXECUTION_TIMEOUT_MS);

        let matching = get_matching_hooks(&self.settings, event, &hook_input);
        if matching.is_empty() {
            return AggregatedHookResult::default();
        }

        let match_query = resolve_match_query(event, &hook_input);
        let hook_name = match &match_query {
            Some(q) => format!("{}:{}", event, q),
            None => event.to_string(),
        };

        debug!("Running {} hooks for {}", matching.len(), hook_name);

        let json_input = match serde_json::to_string(&hook_input) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to stringify hook input for {}: {}", hook_name, e);
                return AggregatedHookResult::default();
            }
        };

        // Run all hooks in parallel
        let batch_start = Instant::now();
        let hook_futures =
            matching
                .into_iter()
                .map(|matched| {
                    let json_input = json_input.clone();
                    let hook_name = hook_name.clone();
                    let cwd = self.cwd.clone();
                    let event = event.clone();
                    async move {
                        exec_hook(&matched, &event, &hook_name, &json_input, &cwd, timeout).await
                    }
                })
                .collect::<Vec<_>>();

        let results: Vec<HookResult> = join_all(hook_futures).await;
        let duration = batch_start.elapsed();

        debug!(
            "Hook batch {} completed in {}ms ({} hooks)",
            hook_name,
            duration.as_millis(),
            results.len()
        );

        aggregate_hook_results(results)
    }

    // ========================================================================
    // Public API: run_hooks_outside_repl
    // ========================================================================

    /// Execute hooks outside of the REPL context (e.g., Notification, SessionEnd).
    ///
    /// Unlike `run_hooks()` which returns aggregated results with messages,
    /// this returns simple success/failure/blocked per hook. Errors are logged
    /// but not surfaced to the model.
    pub async fn run_hooks_outside_repl(
        &self,
        event: &HookEvent,
        extra_fields: serde_json::Value,
        permission_mode: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Vec<HookOutsideReplResult> {
        let hook_input = self.build_hook_input(event, extra_fields, permission_mode, None, None);

        let timeout = timeout_ms.unwrap_or(TOOL_HOOK_EXECUTION_TIMEOUT_MS);

        let matching = get_matching_hooks(&self.settings, event, &hook_input);
        if matching.is_empty() {
            return Vec::new();
        }

        let match_query = resolve_match_query(event, &hook_input);
        let hook_name = match &match_query {
            Some(q) => format!("{}:{}", event, q),
            None => event.to_string(),
        };

        let json_input = match serde_json::to_string(&hook_input) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to stringify hook input for {}: {}", hook_name, e);
                return Vec::new();
            }
        };

        let hook_futures = matching
            .into_iter()
            .map(|matched| {
                let json_input = json_input.clone();
                let hook_name = hook_name.clone();
                let cwd = self.cwd.clone();
                let event = event.clone();
                let command_display = matched.hook.display_text();
                async move {
                    let result =
                        exec_hook(&matched, &event, &hook_name, &json_input, &cwd, timeout).await;
                    HookOutsideReplResult {
                        command: command_display,
                        succeeded: result.outcome == HookOutcome::Success,
                        output: if !result.stdout.is_empty() {
                            result.stdout
                        } else {
                            result.stderr.clone()
                        },
                        blocked: result.blocking_error.is_some(),
                        watch_paths: result.watch_paths,
                        system_message: result.system_message,
                    }
                }
            })
            .collect::<Vec<_>>();

        join_all(hook_futures).await
    }
}

// ============================================================================
// Per-type hook execution
// ============================================================================

/// Execute a single matched hook, dispatching to the appropriate executor.
async fn exec_hook(
    matched: &MatchedHook,
    event: &HookEvent,
    hook_name: &str,
    json_input: &str,
    cwd: &str,
    timeout_ms: u64,
) -> HookResult {
    let start = Instant::now();
    let command_display = matched.hook.display_text();

    // Per-hook timeout overrides the batch timeout
    let effective_timeout_ms = matched
        .hook
        .timeout_secs()
        .map(|s| (s * 1000.0) as u64)
        .unwrap_or(timeout_ms);

    let result = match &matched.hook {
        HookCommand::Command(cmd) => {
            exec_command_hook(cmd, event, hook_name, json_input, cwd, effective_timeout_ms).await
        }
        HookCommand::Prompt(prompt) => {
            exec_prompt_hook(prompt, event, hook_name, json_input, effective_timeout_ms).await
        }
        HookCommand::Http(http) => {
            exec_http_hook(http, event, hook_name, json_input, effective_timeout_ms).await
        }
        HookCommand::Agent(agent) => {
            exec_agent_hook(agent, event, hook_name, json_input, effective_timeout_ms).await
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(mut r) => {
            r.duration_ms = Some(duration_ms);
            r.command_display = command_display;
            r
        }
        Err(e) => {
            warn!("Hook {} failed: {}", hook_name, e);
            HookResult {
                outcome: HookOutcome::NonBlockingError,
                stderr: format!("Failed to run: {}", e),
                duration_ms: Some(duration_ms),
                command_display,
                ..Default::default()
            }
        }
    }
}

// ============================================================================
// Command hook execution (shell with stdout/stderr capture + JSON parse)
// ============================================================================

/// Execute a command-based hook by spawning a shell process.
///
/// The hook receives its input as JSON on stdin (with trailing newline).
/// Output is captured from stdout/stderr. If stdout starts with '{', it
/// is parsed as JSON (SyncHookJsonOutput or AsyncHookJsonOutput). Otherwise
/// treated as plain text.
///
/// Exit codes:
/// - 0: success
/// - 2: blocking error (stderr is the error message)
/// - other non-zero: non-blocking error
async fn exec_command_hook(
    hook: &CommandHook,
    event: &HookEvent,
    hook_name: &str,
    json_input: &str,
    cwd: &str,
    timeout_ms: u64,
) -> Result<HookResult> {
    let command = &hook.command;

    debug!("Executing command hook for {}: {}", hook_name, command);

    // Build the shell command.
    // On all platforms we use "bash -c" by default; PowerShell uses "pwsh -NoProfile -NonInteractive -Command".
    let (shell_program, shell_args) = match hook.shell.as_ref().unwrap_or(&ShellType::Bash) {
        ShellType::Bash => {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
            (shell, vec!["-c".to_string(), command.clone()])
        }
        ShellType::PowerShell => (
            "pwsh".to_string(),
            vec![
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                command.clone(),
            ],
        ),
    };

    let mut child = tokio::process::Command::new(&shell_program)
        .args(&shell_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(cwd)
        .env("CLAUDE_PROJECT_DIR", cwd)
        .spawn()
        .with_context(|| format!("Failed to spawn hook command: {}", command))?;

    // Write JSON input to stdin with trailing newline (matches TS behavior).
    // Take stdin before wait_with_output() which consumes child.
    if let Some(mut stdin) = child.stdin.take() {
        let input_with_newline = format!("{}\n", json_input);
        // Ignore EPIPE — the hook may have closed stdin before we finish writing.
        let _ = stdin.write_all(input_with_newline.as_bytes()).await;
        drop(stdin);
    }

    // Take stdout/stderr handles before waiting so we can still kill the child
    // on timeout (child.wait() borrows, whereas wait_with_output() consumes).
    let mut child_stdout = child.stdout.take();
    let mut child_stderr = child.stderr.take();

    // Wait for the process with timeout, killing the child on timeout to avoid
    // orphaned processes.
    let timeout_duration = Duration::from_millis(timeout_ms);
    let status = tokio::select! {
        result = child.wait() => {
            match result {
                Ok(status) => status,
                Err(e) => {
                    return Ok(HookResult {
                        outcome: HookOutcome::NonBlockingError,
                        stderr: format!("Error executing hook: {}", e),
                        command_display: command.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        _ = tokio::time::sleep(timeout_duration) => {
            // Timeout — kill the child process to avoid orphans.
            let _ = child.kill().await;
            return Ok(HookResult {
                outcome: HookOutcome::NonBlockingError,
                stderr: format!(
                    "Hook timed out after {}ms: {}",
                    timeout_ms, command
                ),
                command_display: command.clone(),
                ..Default::default()
            });
        }
    };

    // Read captured stdout/stderr after the process has exited.
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    if let Some(ref mut out) = child_stdout {
        let _ = tokio::io::AsyncReadExt::read_to_end(out, &mut stdout_bytes).await;
    }
    if let Some(ref mut err) = child_stderr {
        let _ = tokio::io::AsyncReadExt::read_to_end(err, &mut stderr_bytes).await;
    }

    let exit_code = status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
    let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

    debug!(
        "Command hook {} exited with code {}, stdout={} bytes, stderr={} bytes",
        hook_name,
        exit_code,
        stdout.len(),
        stderr.len()
    );

    // Try to parse stdout as JSON.
    let parsed = parse_hook_output(&stdout);

    match parsed {
        ParsedHookOutput::Json(HookJsonOutput::Async(_)) => {
            // Async hooks are backgrounded in the TS impl. In Rust we just
            // return success since we don't have the background registry yet.
            Ok(HookResult {
                outcome: HookOutcome::Success,
                stdout,
                stderr,
                exit_code: Some(exit_code),
                ..Default::default()
            })
        }
        ParsedHookOutput::Json(HookJsonOutput::Sync(json)) => {
            let mut result = process_hook_json_output(&json, command, hook_name, event);
            result.stdout = stdout;
            result.stderr = stderr;
            result.exit_code = Some(exit_code);
            Ok(result)
        }
        ParsedHookOutput::ValidationError(err) => Ok(HookResult {
            outcome: HookOutcome::NonBlockingError,
            stderr: format!("JSON validation failed: {}", err),
            stdout,
            exit_code: Some(1),
            ..Default::default()
        }),
        ParsedHookOutput::PlainText => {
            // Non-JSON output — interpret by exit code
            match exit_code {
                0 => Ok(HookResult {
                    outcome: HookOutcome::Success,
                    stdout,
                    stderr,
                    exit_code: Some(0),
                    ..Default::default()
                }),
                2 => {
                    // Exit code 2 = blocking error. stderr is the message.
                    let error_msg = if stderr.is_empty() {
                        "No stderr output".to_string()
                    } else {
                        stderr.clone()
                    };
                    Ok(HookResult {
                        outcome: HookOutcome::Blocking,
                        blocking_error: Some(HookBlockingError {
                            blocking_error: format!("[{}]: {}", command, error_msg),
                            command: command.clone(),
                        }),
                        stdout,
                        stderr,
                        exit_code: Some(2),
                        ..Default::default()
                    })
                }
                _ => {
                    // Non-zero non-2 exit code = non-blocking error
                    let error_msg = if stderr.trim().is_empty() {
                        "No stderr output".to_string()
                    } else {
                        stderr.trim().to_string()
                    };
                    Ok(HookResult {
                        outcome: HookOutcome::NonBlockingError,
                        stderr: format!("Failed with non-blocking status code: {}", error_msg),
                        stdout,
                        exit_code: Some(exit_code),
                        ..Default::default()
                    })
                }
            }
        }
    }
}

// ============================================================================
// Prompt hook execution
// ============================================================================

/// Execute a prompt-based hook by querying an LLM.
///
/// The prompt text has $ARGUMENTS replaced with the JSON input.
/// This is a simplified implementation — the full TS version calls into the
/// query pipeline. Here we prepare the result structure; the actual LLM call
/// is deferred to the caller's infrastructure.
async fn exec_prompt_hook(
    hook: &PromptHook,
    _event: &HookEvent,
    hook_name: &str,
    json_input: &str,
    _timeout_ms: u64,
) -> Result<HookResult> {
    debug!("Executing prompt hook for {}: {}", hook_name, hook.prompt);

    // Substitute $ARGUMENTS in the prompt with the JSON input.
    let resolved_prompt = hook.prompt.replace("$ARGUMENTS", json_input);

    // In the full implementation this would call into the LLM query pipeline.
    // For now, we return a non-blocking error indicating prompt hooks need
    // the full query infrastructure.
    Ok(HookResult {
        outcome: HookOutcome::NonBlockingError,
        stderr: format!(
            "Prompt hook execution requires LLM query infrastructure. Prompt: {}",
            truncate(&resolved_prompt, 200)
        ),
        command_display: format!("prompt: {}", truncate(&hook.prompt, 60)),
        ..Default::default()
    })
}

// ============================================================================
// HTTP hook execution
// ============================================================================

/// Execute an HTTP-based hook by POSTing the input JSON to the configured URL.
///
/// The response body is parsed as JSON (SyncHookJsonOutput).
/// Header values may reference environment variables using $VAR_NAME syntax;
/// only variables listed in allowedEnvVars are interpolated.
async fn exec_http_hook(
    hook: &HttpHook,
    event: &HookEvent,
    hook_name: &str,
    json_input: &str,
    timeout_ms: u64,
) -> Result<HookResult> {
    use super::ssrf::ssrf_check;

    debug!("Executing HTTP hook for {}: {}", hook_name, hook.url);

    // ── SSRF guard ────────────────────────────────────────────────────────────
    // Resolve the hostname and reject private/link-local ranges before making
    // any network connection. This mirrors TS's ssrfGuardedLookup.
    let parsed_url = url::Url::parse(&hook.url)
        .map_err(|e| anyhow::anyhow!("HTTP hook has invalid URL '{}': {}", hook.url, e))?;
    let host = parsed_url.host_str().unwrap_or("");
    let port = parsed_url.port_or_known_default().unwrap_or(80);

    if let Err(ssrf_msg) = ssrf_check(host, port).await {
        return Ok(HookResult {
            outcome: HookOutcome::NonBlockingError,
            stderr: ssrf_msg,
            ..Default::default()
        });
    }
    // ── end SSRF guard ────────────────────────────────────────────────────────

    let effective_timeout = hook
        .timeout
        .map(|s| (s * 1000.0) as u64)
        .unwrap_or(DEFAULT_HTTP_HOOK_TIMEOUT_MS.min(timeout_ms));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(effective_timeout))
        .build()
        .context("Failed to build HTTP client")?;

    let mut request = client
        .post(&hook.url)
        .header("Content-Type", "application/json")
        .body(json_input.to_string());

    // Interpolate headers with env vars and apply CRLF sanitization.
    if let Some(ref headers) = hook.headers {
        let allowed_vars: std::collections::HashSet<&str> = hook
            .allowed_env_vars
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        for (key, value_template) in headers {
            let resolved = interpolate_env_vars(value_template, &allowed_vars);
            let sanitized = sanitize_header_value(&resolved);
            request = request.header(key, sanitized);
        }
    }

    let response = match request.send().await {
        Ok(resp) => resp,
        Err(e) => {
            if e.is_timeout() {
                return Ok(HookResult {
                    outcome: HookOutcome::NonBlockingError,
                    stderr: format!("HTTP hook timed out: {}", hook.url),
                    ..Default::default()
                });
            }
            return Ok(HookResult {
                outcome: HookOutcome::NonBlockingError,
                stderr: format!("HTTP hook failed: {}", e),
                ..Default::default()
            });
        }
    };

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Ok(HookResult {
            outcome: HookOutcome::NonBlockingError,
            stderr: format!("HTTP {} from {}", status.as_u16(), hook.url),
            stdout: body,
            exit_code: Some(status.as_u16() as i32),
            ..Default::default()
        });
    }

    // Parse response body as JSON.
    let parsed = parse_http_hook_output(&body);

    match parsed {
        ParsedHookOutput::Json(HookJsonOutput::Async(_)) => Ok(HookResult {
            outcome: HookOutcome::Success,
            stdout: body,
            ..Default::default()
        }),
        ParsedHookOutput::Json(HookJsonOutput::Sync(json)) => {
            let mut result = process_hook_json_output(&json, &hook.url, hook_name, event);
            result.stdout = body;
            result.exit_code = Some(status.as_u16() as i32);
            Ok(result)
        }
        ParsedHookOutput::ValidationError(err) => Ok(HookResult {
            outcome: HookOutcome::NonBlockingError,
            stderr: format!("JSON validation failed: {}", err),
            stdout: body,
            ..Default::default()
        }),
        ParsedHookOutput::PlainText => {
            // HTTP hooks must return JSON (unlike command hooks which accept plain text).
            let error_msg = if body.len() > 200 {
                format!("{}...", &body[..200])
            } else {
                body.clone()
            };
            Ok(HookResult {
                outcome: HookOutcome::NonBlockingError,
                stderr: format!(
                    "HTTP hook must return JSON, but got non-JSON response body: {}",
                    error_msg
                ),
                stdout: body,
                ..Default::default()
            })
        }
    }
}

// ============================================================================
// Agent hook execution
// ============================================================================

/// Execute an agent-based hook by running a subagent.
///
/// The prompt has $ARGUMENTS replaced with the JSON input.
/// This is a simplified implementation — the full TS version spawns a
/// subagent with tool access. Here we prepare the result structure.
async fn exec_agent_hook(
    hook: &AgentHook,
    _event: &HookEvent,
    hook_name: &str,
    json_input: &str,
    _timeout_ms: u64,
) -> Result<HookResult> {
    debug!("Executing agent hook for {}: {}", hook_name, hook.prompt);

    let resolved_prompt = hook.prompt.replace("$ARGUMENTS", json_input);

    // In the full implementation this would spawn a subagent.
    Ok(HookResult {
        outcome: HookOutcome::NonBlockingError,
        stderr: format!(
            "Agent hook execution requires subagent infrastructure. Prompt: {}",
            truncate(&resolved_prompt, 200)
        ),
        command_display: format!("agent: {}", truncate(&hook.prompt, 60)),
        ..Default::default()
    })
}

// ============================================================================
// Hook output parsing
// ============================================================================

enum ParsedHookOutput {
    Json(HookJsonOutput),
    PlainText,
    ValidationError(String),
}

/// Parse stdout from a command hook.
///
/// If the output starts with '{', attempt JSON parse. Otherwise treat as plain text.
fn parse_hook_output(stdout: &str) -> ParsedHookOutput {
    let trimmed = stdout.trim();
    if !trimmed.starts_with('{') {
        return ParsedHookOutput::PlainText;
    }

    match try_parse_hook_json(trimmed) {
        Ok(json) => ParsedHookOutput::Json(json),
        Err(err) => ParsedHookOutput::ValidationError(err),
    }
}

/// Parse the HTTP hook response body.
///
/// Empty body is treated as an empty JSON object (success with no fields).
/// Non-JSON responses are validation errors (HTTP hooks must return JSON).
fn parse_http_hook_output(body: &str) -> ParsedHookOutput {
    let trimmed = body.trim();

    if trimmed.is_empty() {
        // Empty body => empty JSON object => success with no special fields.
        return ParsedHookOutput::Json(HookJsonOutput::Sync(SyncHookJsonOutput::default()));
    }

    if !trimmed.starts_with('{') {
        return ParsedHookOutput::PlainText;
    }

    match try_parse_hook_json(trimmed) {
        Ok(json) => ParsedHookOutput::Json(json),
        Err(err) => ParsedHookOutput::ValidationError(err),
    }
}

/// Try to parse a JSON string as either AsyncHookJsonOutput or SyncHookJsonOutput.
fn try_parse_hook_json(json_str: &str) -> std::result::Result<HookJsonOutput, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("JSON parse error: {}", e))?;

    // Check for async response first: {"async": true, ...}
    if parsed.get("async").and_then(|v| v.as_bool()) == Some(true) {
        let async_output: AsyncHookJsonOutput =
            serde_json::from_value(parsed).map_err(|e| format!("Async JSON validation: {}", e))?;
        return Ok(HookJsonOutput::Async(async_output));
    }

    // Parse as sync response.
    let sync_output: SyncHookJsonOutput =
        serde_json::from_value(parsed).map_err(|e| format!("Sync JSON validation: {}", e))?;
    Ok(HookJsonOutput::Sync(sync_output))
}

// ============================================================================
// Process validated sync JSON output into a HookResult
// ============================================================================

/// Process a validated SyncHookJsonOutput into a HookResult.
///
/// This is a direct port of the TypeScript `processHookJSONOutput` function.
/// It handles all the field mappings from the JSON output to the result struct.
fn process_hook_json_output(
    json: &SyncHookJsonOutput,
    command: &str,
    _hook_name: &str,
    _event: &HookEvent,
) -> HookResult {
    let mut result = HookResult::default();

    // Handle continue === false
    if json.should_continue == Some(false) {
        result.prevent_continuation = Some(true);
        if let Some(ref reason) = json.stop_reason {
            result.stop_reason = Some(reason.clone());
        }
    }

    // Handle top-level decision field (approve/block)
    if let Some(ref decision) = json.decision {
        match decision.as_str() {
            "approve" => {
                result.permission_behavior = Some(PermissionBehavior::Allow);
            }
            "block" => {
                result.permission_behavior = Some(PermissionBehavior::Deny);
                result.blocking_error = Some(HookBlockingError {
                    blocking_error: json
                        .reason
                        .as_deref()
                        .unwrap_or("Blocked by hook")
                        .to_string(),
                    command: command.to_string(),
                });
            }
            _ => {
                warn!(
                    "Unknown hook decision type: {}. Valid types are: approve, block",
                    decision
                );
            }
        }
    }

    // Handle systemMessage
    if let Some(ref sys_msg) = json.system_message {
        result.system_message = Some(sys_msg.clone());
    }

    // Handle reason as permission decision reason (when a permission behavior is set)
    if result.permission_behavior.is_some() {
        if let Some(ref reason) = json.reason {
            result.hook_permission_decision_reason = Some(reason.clone());
        }
    }

    // Handle hookSpecificOutput
    if let Some(ref specific) = json.hook_specific_output {
        match specific {
            HookSpecificOutput::PreToolUse {
                permission_decision,
                permission_decision_reason,
                updated_input,
                additional_context,
            } => {
                // Override with more specific permission decision
                if let Some(ref pd) = permission_decision {
                    match pd.as_str() {
                        "allow" => {
                            result.permission_behavior = Some(PermissionBehavior::Allow);
                        }
                        "deny" => {
                            result.permission_behavior = Some(PermissionBehavior::Deny);
                            result.blocking_error = Some(HookBlockingError {
                                blocking_error: permission_decision_reason
                                    .as_deref()
                                    .or(json.reason.as_deref())
                                    .unwrap_or("Blocked by hook")
                                    .to_string(),
                                command: command.to_string(),
                            });
                        }
                        "ask" => {
                            result.permission_behavior = Some(PermissionBehavior::Ask);
                        }
                        _ => {
                            warn!("Unknown hook permissionDecision: {}", pd);
                        }
                    }
                }
                result.hook_permission_decision_reason = permission_decision_reason.clone();
                if let Some(ref ui) = updated_input {
                    result.updated_input = Some(ui.clone());
                }
                result.additional_context = additional_context.clone();
            }
            HookSpecificOutput::UserPromptSubmit { additional_context } => {
                result.additional_context = additional_context.clone();
            }
            HookSpecificOutput::SessionStart {
                additional_context,
                initial_user_message,
                watch_paths,
            } => {
                result.additional_context = additional_context.clone();
                result.initial_user_message = initial_user_message.clone();
                result.watch_paths = watch_paths.clone();
            }
            HookSpecificOutput::Setup { additional_context } => {
                result.additional_context = additional_context.clone();
            }
            HookSpecificOutput::SubagentStart { additional_context } => {
                result.additional_context = additional_context.clone();
            }
            HookSpecificOutput::PostToolUse {
                additional_context,
                updated_mcp_tool_output,
            } => {
                result.additional_context = additional_context.clone();
                if let Some(ref out) = updated_mcp_tool_output {
                    result.updated_mcp_tool_output = Some(out.clone());
                }
            }
            HookSpecificOutput::PostToolUseFailure { additional_context } => {
                result.additional_context = additional_context.clone();
            }
            HookSpecificOutput::PermissionDenied { retry } => {
                result.retry = *retry;
            }
            HookSpecificOutput::Notification { additional_context } => {
                result.additional_context = additional_context.clone();
            }
            HookSpecificOutput::PermissionRequest { decision } => {
                if let Some(ref dec) = decision {
                    match dec {
                        PermissionRequestDecision::Allow {
                            updated_input,
                            updated_permissions,
                        } => {
                            result.permission_behavior = Some(PermissionBehavior::Allow);
                            result.permission_request_result =
                                Some(PermissionRequestResult::Allow {
                                    updated_input: updated_input.clone(),
                                    updated_permissions: updated_permissions.clone(),
                                });
                            if let Some(ref ui) = updated_input {
                                result.updated_input = Some(ui.clone());
                            }
                        }
                        PermissionRequestDecision::Deny { message, interrupt } => {
                            result.permission_behavior = Some(PermissionBehavior::Deny);
                            result.permission_request_result =
                                Some(PermissionRequestResult::Deny {
                                    message: message.clone(),
                                    interrupt: *interrupt,
                                });
                        }
                    }
                }
            }
            HookSpecificOutput::Elicitation { action, content: _ } => {
                if let Some(ref act) = action {
                    if act == "decline" {
                        result.blocking_error = Some(HookBlockingError {
                            blocking_error: json
                                .reason
                                .as_deref()
                                .unwrap_or("Elicitation denied by hook")
                                .to_string(),
                            command: command.to_string(),
                        });
                    }
                }
            }
            HookSpecificOutput::ElicitationResult { action, content: _ } => {
                if let Some(ref act) = action {
                    if act == "decline" {
                        result.blocking_error = Some(HookBlockingError {
                            blocking_error: json
                                .reason
                                .as_deref()
                                .unwrap_or("Elicitation result blocked by hook")
                                .to_string(),
                            command: command.to_string(),
                        });
                    }
                }
            }
            HookSpecificOutput::CwdChanged { watch_paths } => {
                result.watch_paths = watch_paths.clone();
            }
            HookSpecificOutput::FileChanged { watch_paths } => {
                result.watch_paths = watch_paths.clone();
            }
            HookSpecificOutput::WorktreeCreate { worktree_path: _ } => {
                // WorktreeCreate output is handled by the caller
            }
        }
    }

    // Set outcome based on whether we have a blocking error
    if result.blocking_error.is_some() {
        result.outcome = HookOutcome::Blocking;
    } else {
        result.outcome = HookOutcome::Success;
    }

    result
}

// ============================================================================
// Utility functions
// ============================================================================

/// Strip CR, LF, NUL from a header value to prevent CRLF injection
/// (mirrors TS sanitizeHeaderValue).
fn sanitize_header_value(value: &str) -> String {
    value
        .chars()
        .filter(|&c| c != '\r' && c != '\n' && c != '\0')
        .collect()
}

/// Interpolate environment variables in a header value string.
///
/// Only variables listed in `allowed_vars` are resolved. Unresolved
/// references become empty strings. Supports $VAR_NAME and ${VAR_NAME} syntax.
fn interpolate_env_vars(template: &str, allowed_vars: &std::collections::HashSet<&str>) -> String {
    let mut result = template.to_string();

    // Handle ${VAR_NAME} syntax
    let re_braced = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
    result = re_braced
        .replace_all(&result, |caps: &regex::Captures| {
            let var_name = &caps[1];
            if allowed_vars.contains(var_name) {
                std::env::var(var_name).unwrap_or_default()
            } else {
                String::new()
            }
        })
        .to_string();

    // Handle $VAR_NAME syntax (not preceded by \)
    let re_plain = regex::Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    result = re_plain
        .replace_all(&result, |caps: &regex::Captures| {
            let var_name = &caps[1];
            if allowed_vars.contains(var_name) {
                std::env::var(var_name).unwrap_or_default()
            } else {
                String::new()
            }
        })
        .to_string();

    result
}

/// Truncate a string to a maximum length, appending "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", s[..max_len.saturating_sub(3)].trim_end())
    }
}

// ============================================================================
// Formatting helpers (match TypeScript exports)
// ============================================================================

/// Format a PreToolUse hook blocking error message.
pub fn get_pre_tool_hook_blocking_message(
    hook_name: &str,
    blocking_error: &HookBlockingError,
) -> String {
    format!(
        "{} hook error: {}",
        hook_name, blocking_error.blocking_error
    )
}

/// Format a Stop hook blocking error message.
pub fn get_stop_hook_message(blocking_error: &HookBlockingError) -> String {
    format!("Stop hook feedback:\n{}", blocking_error.blocking_error)
}

/// Format a TeammateIdle hook blocking error message.
pub fn get_teammate_idle_hook_message(blocking_error: &HookBlockingError) -> String {
    format!(
        "TeammateIdle hook feedback:\n{}",
        blocking_error.blocking_error
    )
}

/// Format a TaskCreated hook blocking error message.
pub fn get_task_created_hook_message(blocking_error: &HookBlockingError) -> String {
    format!(
        "TaskCreated hook feedback:\n{}",
        blocking_error.blocking_error
    )
}

/// Format a TaskCompleted hook blocking error message.
pub fn get_task_completed_hook_message(blocking_error: &HookBlockingError) -> String {
    format!(
        "TaskCompleted hook feedback:\n{}",
        blocking_error.blocking_error
    )
}

/// Format a UserPromptSubmit hook blocking error message.
pub fn get_user_prompt_submit_hook_blocking_message(blocking_error: &HookBlockingError) -> String {
    format!(
        "UserPromptSubmit operation blocked by hook:\n{}",
        blocking_error.blocking_error
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hook_output_plain_text() {
        let result = parse_hook_output("hello world");
        assert!(matches!(result, ParsedHookOutput::PlainText));
    }

    #[test]
    fn test_parse_hook_output_empty() {
        let result = parse_hook_output("");
        assert!(matches!(result, ParsedHookOutput::PlainText));
    }

    #[test]
    fn test_parse_hook_output_sync_json() {
        let json = r#"{"continue": false, "stopReason": "test"}"#;
        let result = parse_hook_output(json);
        match result {
            ParsedHookOutput::Json(HookJsonOutput::Sync(sync)) => {
                assert_eq!(sync.should_continue, Some(false));
                assert_eq!(sync.stop_reason.as_deref(), Some("test"));
            }
            _ => panic!("Expected sync JSON output"),
        }
    }

    #[test]
    fn test_parse_hook_output_async_json() {
        let json = r#"{"async": true, "asyncTimeout": 5000}"#;
        let result = parse_hook_output(json);
        match result {
            ParsedHookOutput::Json(HookJsonOutput::Async(async_out)) => {
                assert!(async_out.is_async);
                assert_eq!(async_out.async_timeout, Some(5000));
            }
            _ => panic!("Expected async JSON output"),
        }
    }

    #[test]
    fn test_parse_http_hook_output_empty_body() {
        let result = parse_http_hook_output("");
        match result {
            ParsedHookOutput::Json(HookJsonOutput::Sync(sync)) => {
                assert!(sync.decision.is_none());
            }
            _ => panic!("Expected sync JSON from empty body"),
        }
    }

    #[test]
    fn test_process_hook_json_block_decision() {
        let json = SyncHookJsonOutput {
            decision: Some("block".to_string()),
            reason: Some("test reason".to_string()),
            ..Default::default()
        };
        let result =
            process_hook_json_output(&json, "test_cmd", "test_hook", &HookEvent::PreToolUse);
        assert_eq!(result.permission_behavior, Some(PermissionBehavior::Deny));
        assert!(result.blocking_error.is_some());
        assert_eq!(
            result.blocking_error.as_ref().unwrap().blocking_error,
            "test reason"
        );
    }

    #[test]
    fn test_process_hook_json_approve_decision() {
        let json = SyncHookJsonOutput {
            decision: Some("approve".to_string()),
            ..Default::default()
        };
        let result =
            process_hook_json_output(&json, "test_cmd", "test_hook", &HookEvent::PreToolUse);
        assert_eq!(result.permission_behavior, Some(PermissionBehavior::Allow));
        assert!(result.blocking_error.is_none());
    }

    #[test]
    fn test_process_hook_json_continue_false() {
        let json = SyncHookJsonOutput {
            should_continue: Some(false),
            stop_reason: Some("stopping".to_string()),
            ..Default::default()
        };
        let result = process_hook_json_output(&json, "cmd", "hook", &HookEvent::Stop);
        assert_eq!(result.prevent_continuation, Some(true));
        assert_eq!(result.stop_reason.as_deref(), Some("stopping"));
    }

    #[test]
    fn test_interpolate_env_vars() {
        std::env::set_var("TEST_HOOK_VAR", "secret123");
        let allowed: std::collections::HashSet<&str> = ["TEST_HOOK_VAR"].into();
        let result = interpolate_env_vars("Bearer $TEST_HOOK_VAR", &allowed);
        assert_eq!(result, "Bearer secret123");
        std::env::remove_var("TEST_HOOK_VAR");
    }

    #[test]
    fn test_interpolate_env_vars_not_allowed() {
        std::env::set_var("TEST_HOOK_SECRET", "should_not_appear");
        let allowed: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let result = interpolate_env_vars("Bearer $TEST_HOOK_SECRET", &allowed);
        assert_eq!(result, "Bearer ");
        std::env::remove_var("TEST_HOOK_SECRET");
    }

    #[test]
    fn test_interpolate_env_vars_braced() {
        std::env::set_var("TEST_HOOK_BRACED", "val");
        let allowed: std::collections::HashSet<&str> = ["TEST_HOOK_BRACED"].into();
        let result = interpolate_env_vars("x-${TEST_HOOK_BRACED}-y", &allowed);
        assert_eq!(result, "x-val-y");
        std::env::remove_var("TEST_HOOK_BRACED");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a very long string here", 10), "a very...");
    }

    #[test]
    fn test_session_end_timeout_default() {
        // Clear env var to test default
        std::env::remove_var("CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS");
        assert_eq!(HookRunner::session_end_hook_timeout_ms(), 1500);
    }

    #[test]
    fn test_formatting_helpers() {
        let be = HookBlockingError {
            blocking_error: "bad thing".to_string(),
            command: "cmd".to_string(),
        };
        assert_eq!(
            get_pre_tool_hook_blocking_message("PreToolUse:Write", &be),
            "PreToolUse:Write hook error: bad thing"
        );
        assert_eq!(get_stop_hook_message(&be), "Stop hook feedback:\nbad thing");
        assert_eq!(
            get_user_prompt_submit_hook_blocking_message(&be),
            "UserPromptSubmit operation blocked by hook:\nbad thing"
        );
    }
}
