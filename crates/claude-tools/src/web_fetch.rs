use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::permissions::{
    PermissionAllowDecision, PermissionAskDecision, PermissionBehavior, PermissionDecisionReason,
    PermissionDenyDecision, PermissionResult, PermissionRuleSource, PermissionRuleValue,
    PermissionUpdate, ToolPermissionContext,
};
use claude_core::secondary_model;
use claude_core::types::events::ToolResultData;

pub struct WebFetchTool;

const MAX_URL_LENGTH: usize = 2000;
const MAX_HTTP_CONTENT_LENGTH: u64 = 10 * 1024 * 1024;
const FETCH_TIMEOUT_SECS: u64 = 60;
const DOMAIN_CHECK_TIMEOUT_SECS: u64 = 10;
const MAX_REDIRECTS: usize = 10;
const CACHE_TTL_SECS: u64 = 15 * 60;
static DOMAIN_CHECK_CACHE: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));
static URL_CACHE: Lazy<Mutex<HashMap<String, CachedFetch>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
struct CachedFetch {
    inserted_at: std::time::Instant,
    bytes: u64,
    code: u16,
    code_text: String,
    content_type: String,
    content: String,
}

/// Max characters sent to the secondary model.
/// Mirrors TS `MAX_MARKDOWN_LENGTH` in `tools/WebFetchTool/utils.ts`.
const MAX_MARKDOWN_LENGTH: usize = 100_000;

/// Build the secondary-model prompt that wraps fetched content +
/// the user's prompt + quoting guidelines.
/// Ports TS `makeSecondaryModelPrompt()` in `tools/WebFetchTool/prompt.ts`.
fn make_secondary_model_prompt(
    markdown_content: &str,
    user_prompt: &str,
    is_preapproved_domain: bool,
) -> String {
    let guidelines = if is_preapproved_domain {
        "Provide a concise response based on the content above. Include relevant details, code examples, and documentation excerpts as needed."
    } else {
        "Provide a concise response based only on the content above. In your response:\n \
         - Enforce a strict 125-character maximum for quotes from any source document. Open Source Software is ok as long as we respect the license.\n \
         - Use quotation marks for exact language from articles; any language outside of the quotation should never be word-for-word the same.\n \
         - You are not a lawyer and never comment on the legality of your own prompts and responses.\n \
         - Never produce or reproduce exact song lyrics."
    };

    format!(
        "\nWeb page content:\n---\n{markdown}\n---\n\n{prompt}\n\n{guidelines}\n",
        markdown = markdown_content,
        prompt = user_prompt,
        guidelines = guidelines,
    )
}

/// Apply the user's prompt to fetched markdown via the secondary model.
/// Ports TS `applyPromptToMarkdown()` in `tools/WebFetchTool/utils.ts`.
///
/// Returns `None` if no secondary model is registered — callers should then
/// surface the raw markdown and echo the prompt so the primary model can
/// apply it itself.
async fn apply_prompt_to_markdown(
    user_prompt: &str,
    markdown_content: &str,
    is_preapproved_domain: bool,
    cancel: CancellationToken,
) -> Result<Option<String>> {
    let Some(model) = secondary_model::get_global() else {
        return Ok(None);
    };

    let truncated = if markdown_content.len() > MAX_MARKDOWN_LENGTH {
        let mut cut = MAX_MARKDOWN_LENGTH;
        while !markdown_content.is_char_boundary(cut) {
            cut -= 1;
        }
        format!(
            "{}\n\n[Content truncated due to length...]",
            &markdown_content[..cut]
        )
    } else {
        markdown_content.to_string()
    };

    let prompt = make_secondary_model_prompt(&truncated, user_prompt, is_preapproved_domain);
    let summary = model.summarize(&prompt, cancel).await?;
    Ok(Some(summary))
}

fn error_result(msg: impl Into<String>) -> ToolResultData {
    ToolResultData {
        data: json!({ "error": msg.into() }),
        is_error: true,
    }
}

fn validate_url(url: &str) -> bool {
    if url.len() > MAX_URL_LENGTH {
        return false;
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.username() != "" || parsed.password().is_some() {
        return false;
    }
    let Some(hostname) = parsed.host_str() else {
        return false;
    };
    hostname.split('.').count() >= 2
}

fn upgrade_http_to_https(url: &str) -> Result<String> {
    let mut parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() == "http" {
        parsed
            .set_scheme("https")
            .map_err(|_| anyhow::anyhow!("failed to upgrade URL scheme"))?;
    }
    Ok(parsed.to_string())
}

fn is_permitted_redirect(original_url: &str, redirect_url: &str) -> bool {
    let Ok(original) = reqwest::Url::parse(original_url) else {
        return false;
    };
    let Ok(redirect) = reqwest::Url::parse(redirect_url) else {
        return false;
    };
    if redirect.scheme() != original.scheme() {
        return false;
    }
    if redirect.port_or_known_default() != original.port_or_known_default() {
        return false;
    }
    if redirect.username() != "" || redirect.password().is_some() {
        return false;
    }
    let strip_www = |host: &str| host.strip_prefix("www.").unwrap_or(host).to_string();
    match (original.host_str(), redirect.host_str()) {
        (Some(a), Some(b)) => strip_www(a) == strip_www(b),
        _ => false,
    }
}

fn redirect_status_text(status: u16) -> &'static str {
    match status {
        301 => "Moved Permanently",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        _ => "Found",
    }
}

fn redirect_message(original_url: &str, redirect_url: &str, status: u16, prompt: &str) -> String {
    format!(
        "REDIRECT DETECTED: The URL redirects to a different host.\n\n\
Original URL: {original_url}\n\
Redirect URL: {redirect_url}\n\
Status: {status} {status_text}\n\n\
To complete your request, I need to fetch content from the redirected URL. Please use WebFetch again with these parameters:\n\
- url: \"{redirect_url}\"\n\
- prompt: \"{prompt}\"",
        status_text = redirect_status_text(status),
    )
}

async fn fetch_with_permitted_redirects(
    client: &reqwest::Client,
    url: String,
    cancel: &CancellationToken,
    depth: usize,
) -> Result<FetchResponse> {
    if depth > MAX_REDIRECTS {
        anyhow::bail!("Too many redirects (exceeded {MAX_REDIRECTS})");
    }

    let response = tokio::select! {
        _ = cancel.cancelled() => {
            anyhow::bail!("request cancelled");
        }
        res = client
            .get(&url)
            .header(reqwest::header::ACCEPT, "text/markdown, text/html, */*")
            .send() => res?,
    };

    let status = response.status();
    if matches!(status.as_u16(), 301 | 302 | 307 | 308) {
        let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
            anyhow::bail!("Redirect missing Location header");
        };
        let location = location.to_str().unwrap_or_default();
        let redirect_url = reqwest::Url::parse(&url)?.join(location)?.to_string();
        if is_permitted_redirect(&url, &redirect_url) {
            return Box::pin(fetch_with_permitted_redirects(
                client,
                redirect_url,
                cancel,
                depth + 1,
            ))
            .await;
        }
        return Ok(FetchResponse::Redirect {
            original_url: url,
            redirect_url,
            status_code: status.as_u16(),
        });
    }

    let code = status.as_u16();
    let code_text = status.canonical_reason().unwrap_or("Unknown").to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = tokio::select! {
        _ = cancel.cancelled() => {
            anyhow::bail!("request cancelled while reading body");
        }
        res = response.bytes() => res?,
    };
    Ok(FetchResponse::Content {
        body: body.to_vec(),
        code,
        code_text,
        content_type,
    })
}

enum FetchResponse {
    Content {
        body: Vec<u8>,
        code: u16,
        code_text: String,
        content_type: String,
    },
    Redirect {
        original_url: String,
        redirect_url: String,
        status_code: u16,
    },
}

fn web_fetch_rule_content(input: &Value) -> String {
    let Some(url) = input.get("url").and_then(|value| value.as_str()) else {
        return format!("input:{}", input);
    };
    match reqwest::Url::parse(url).ok().and_then(|url| {
        url.host_str()
            .map(|hostname| format!("domain:{}", hostname))
    }) {
        Some(content) => content,
        None => format!("input:{}", input),
    }
}

fn web_fetch_permission_suggestions(rule_content: &str) -> Vec<PermissionUpdate> {
    vec![PermissionUpdate::AddRules {
        destination: PermissionRuleSource::LocalSettings,
        rules: vec![PermissionRuleValue {
            tool_name: "WebFetch".to_string(),
            rule_content: Some(rule_content.to_string()),
        }],
        behavior: PermissionBehavior::Allow,
    }]
}

fn get_cached_url(url: &str) -> Option<CachedFetch> {
    let mut cache = URL_CACHE.lock().ok()?;
    let entry = cache.get(url)?;
    if entry.inserted_at.elapsed() > std::time::Duration::from_secs(CACHE_TTL_SECS) {
        cache.remove(url);
        return None;
    }
    Some(entry.clone())
}

fn put_cached_url(
    url: &str,
    bytes: u64,
    code: u16,
    code_text: &str,
    content_type: &str,
    content: &str,
) {
    if let Ok(mut cache) = URL_CACHE.lock() {
        cache.insert(
            url.to_string(),
            CachedFetch {
                inserted_at: std::time::Instant::now(),
                bytes,
                code,
                code_text: code_text.to_string(),
                content_type: content_type.to_string(),
                content: content.to_string(),
            },
        );
    }
}

enum DomainCheckResult {
    Allowed,
    Blocked,
    CheckFailed,
}

fn skip_web_fetch_preflight(project_root: &std::path::Path) -> bool {
    let user = dirs::home_dir()
        .map(|home| {
            claude_core::config::settings::Settings::load_from_file(
                &home.join(".claude").join("settings.json"),
            )
        })
        .unwrap_or_default();
    let project = claude_core::config::settings::Settings::load_from_file(
        &project_root.join(".claude").join("settings.json"),
    );
    user.merge(&project).skip_web_fetch_preflight == Some(true)
}

async fn check_domain_blocklist(client: &reqwest::Client, domain: &str) -> DomainCheckResult {
    if DOMAIN_CHECK_CACHE
        .lock()
        .map(|cache| cache.contains(domain))
        .unwrap_or(false)
    {
        return DomainCheckResult::Allowed;
    }

    let mut url =
        reqwest::Url::parse("https://api.anthropic.com/api/web/domain_info").expect("valid URL");
    url.query_pairs_mut().append_pair("domain", domain);
    let response = match client
        .get(url)
        .timeout(std::time::Duration::from_secs(DOMAIN_CHECK_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_error) => return DomainCheckResult::CheckFailed,
    };

    if response.status().as_u16() != 200 {
        return DomainCheckResult::CheckFailed;
    }

    match response.json::<Value>().await {
        Ok(value) if value.get("can_fetch").and_then(Value::as_bool) == Some(true) => {
            if let Ok(mut cache) = DOMAIN_CHECK_CACHE.lock() {
                cache.insert(domain.to_string());
            }
            DomainCheckResult::Allowed
        }
        Ok(_) => DomainCheckResult::Blocked,
        Err(_error) => DomainCheckResult::CheckFailed,
    }
}

/// Strip HTML tags from a string, returning plain text.
fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut tag_buf = String::new();

    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '<' {
            in_tag = true;
            tag_buf.clear();
            i += 1;
            continue;
        }
        if in_tag {
            if c == '>' {
                in_tag = false;
                let tag_lower = tag_buf.trim().to_lowercase();
                if tag_lower.starts_with("script") {
                    in_script = true;
                } else if tag_lower == "/script" {
                    in_script = false;
                } else if tag_lower.starts_with("style") {
                    in_style = true;
                } else if tag_lower == "/style" {
                    in_style = false;
                }
                // Add whitespace for block-level elements
                let block_tags = [
                    "div",
                    "p",
                    "br",
                    "h1",
                    "h2",
                    "h3",
                    "h4",
                    "h5",
                    "h6",
                    "li",
                    "tr",
                    "td",
                    "th",
                    "blockquote",
                    "section",
                    "article",
                    "header",
                    "footer",
                    "nav",
                    "main",
                ];
                for bt in &block_tags {
                    if tag_lower.starts_with(bt) || tag_lower == format!("/{}", bt) {
                        result.push('\n');
                        break;
                    }
                }
                tag_buf.clear();
            } else {
                tag_buf.push(c);
            }
            i += 1;
            continue;
        }
        if !in_script && !in_style {
            result.push(c);
        }
        i += 1;
    }

    // Decode common HTML entities
    let result = result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&apos;", "'");

    // Collapse multiple blank lines
    let mut out = String::new();
    let mut consecutive_newlines = 0usize;
    for c in result.chars() {
        if c == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                out.push(c);
            }
        } else {
            consecutive_newlines = 0;
            out.push(c);
        }
    }

    out.trim().to_string()
}

/// Verbatim port of TS WebFetchTool/prompt.ts DESCRIPTION.
pub const WEB_FETCH_PROMPT: &str = include_str!("prompts/web_fetch.md");

#[async_trait]
impl ToolExecutor for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn description(&self) -> String {
        WEB_FETCH_PROMPT.to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "prompt": {
                    "type": "string",
                    "description": "Context or instructions for how to process the fetched content"
                }
            },
            "required": ["url", "prompt"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn check_permissions(
        &self,
        input: &Value,
        context: &ToolPermissionContext,
    ) -> PermissionResult {
        if let Some(url) = input.get("url").and_then(|value| value.as_str()) {
            if let Ok(parsed) = reqwest::Url::parse(url) {
                if crate::web_fetch_preapproved::is_preapproved_host(
                    parsed.host_str().unwrap_or(""),
                    parsed.path(),
                ) {
                    return PermissionResult::Allow(PermissionAllowDecision {
                        updated_input: Some(input.clone()),
                        user_modified: None,
                        decision_reason: Some(PermissionDecisionReason::Other {
                            reason: "Preapproved host".to_string(),
                        }),
                        tool_use_id: None,
                        accept_feedback: None,
                    });
                }
            }
        }

        let rule_content = web_fetch_rule_content(input);
        let permission_tool =
            claude_core::permissions::evaluator::SimpleToolPermissions::new("WebFetch", true);
        let tool = &permission_tool as &dyn claude_core::permissions::evaluator::ToolPermissions;
        if let Some(rule) = claude_core::permissions::evaluator::get_rule_by_contents_for_tool(
            context,
            tool,
            &PermissionBehavior::Deny,
        )
        .get(&rule_content)
        .cloned()
        {
            return PermissionResult::Deny(PermissionDenyDecision {
                message: format!("WebFetch denied access to {rule_content}."),
                decision_reason: PermissionDecisionReason::Rule { rule },
                tool_use_id: None,
            });
        }
        if let Some(rule) = claude_core::permissions::evaluator::get_rule_by_contents_for_tool(
            context,
            tool,
            &PermissionBehavior::Ask,
        )
        .get(&rule_content)
        .cloned()
        {
            return PermissionResult::Ask(PermissionAskDecision {
                message:
                    "Claude requested permissions to use WebFetch, but you haven't granted it yet."
                        .to_string(),
                updated_input: None,
                decision_reason: Some(PermissionDecisionReason::Rule { rule }),
                suggestions: Some(web_fetch_permission_suggestions(&rule_content)),
                blocked_path: None,
                is_bash_security_check_for_misparsing: None,
            });
        }
        if let Some(rule) = claude_core::permissions::evaluator::get_rule_by_contents_for_tool(
            context,
            tool,
            &PermissionBehavior::Allow,
        )
        .get(&rule_content)
        .cloned()
        {
            return PermissionResult::Allow(PermissionAllowDecision {
                updated_input: Some(input.clone()),
                user_modified: None,
                decision_reason: Some(PermissionDecisionReason::Rule { rule }),
                tool_use_id: None,
                accept_feedback: None,
            });
        }

        PermissionResult::Ask(PermissionAskDecision {
            message:
                "Claude requested permissions to use WebFetch, but you haven't granted it yet."
                    .to_string(),
            updated_input: None,
            decision_reason: None,
            suggestions: Some(web_fetch_permission_suggestions(&rule_content)),
            blocked_path: None,
            is_bash_security_check_for_misparsing: None,
        })
    }

    async fn call(
        &self,
        input: &Value,
        _ctx: &ToolUseContext,
        cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let url = match input["url"].as_str() {
            Some(u) => u.to_string(),
            None => return Ok(error_result("missing required parameter: url")),
        };
        let prompt = match input["prompt"].as_str() {
            Some(p) => p.to_string(),
            None => return Ok(error_result("missing required parameter: prompt")),
        };
        if !validate_url(&url) {
            return Ok(error_result("Invalid URL"));
        }

        let start = std::time::Instant::now();
        let (bytes, code, code_text, content_type, result_text) = if let Some(cached) =
            get_cached_url(&url)
        {
            (
                cached.bytes,
                cached.code,
                cached.code_text,
                cached.content_type,
                cached.content,
            )
        } else {
            let upgraded_url = match upgrade_http_to_https(&url) {
                Ok(url) => url,
                Err(_) => return Ok(error_result("Invalid URL")),
            };

            let client = match reqwest::Client::builder()
                .user_agent(claude_core::user_agent::get_web_fetch_user_agent())
                .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
                .redirect(reqwest::redirect::Policy::none())
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    return Ok(error_result(format!("failed to build HTTP client: {}", e)));
                }
            };

            if !skip_web_fetch_preflight(&_ctx.working_directory) {
                let domain = reqwest::Url::parse(&upgraded_url)
                    .ok()
                    .and_then(|url| url.host_str().map(ToString::to_string));
                if let Some(domain) = domain {
                    match check_domain_blocklist(&client, &domain).await {
                        DomainCheckResult::Allowed => {}
                        DomainCheckResult::Blocked => {
                            return Ok(error_result(format!(
                                "Claude Code is unable to fetch from {domain}"
                            )));
                        }
                        DomainCheckResult::CheckFailed => {
                            return Ok(error_result(format!(
                                    "Unable to verify if domain {domain} is safe to fetch. This may be due to network restrictions or enterprise security policies blocking claude.ai."
                                )));
                        }
                    }
                }
            }

            let response =
                match fetch_with_permitted_redirects(&client, upgraded_url, &cancel, 0).await {
                    Ok(response) => response,
                    Err(e) => return Ok(error_result(format!("HTTP request failed: {}", e))),
                };

            let (body, code, code_text, content_type) = match response {
                FetchResponse::Content {
                    body,
                    code,
                    code_text,
                    content_type,
                } => (body, code, code_text, content_type),
                FetchResponse::Redirect {
                    original_url,
                    redirect_url,
                    status_code,
                } => {
                    let message =
                        redirect_message(&original_url, &redirect_url, status_code, &prompt);
                    return Ok(ToolResultData {
                        data: json!({
                            "bytes": message.len() as u64,
                            "code": status_code,
                            "codeText": redirect_status_text(status_code),
                            "result": message,
                            "durationMs": start.elapsed().as_millis() as u64,
                            "url": url,
                        }),
                        is_error: false,
                    });
                }
            };

            let bytes = body.len() as u64;
            if bytes > MAX_HTTP_CONTENT_LENGTH {
                return Ok(error_result(format!(
                    "HTTP response exceeded maximum content length of {} bytes",
                    MAX_HTTP_CONTENT_LENGTH
                )));
            }

            let body_text = String::from_utf8_lossy(&body).into_owned();
            let result_text =
                if content_type.contains("text/html") || body_text.trim_start().starts_with('<') {
                    strip_html(&body_text)
                } else {
                    body_text
                };

            const MAX_CHARS: usize = 50_000;
            let result_text = if result_text.len() > MAX_CHARS {
                let mut cut = MAX_CHARS;
                while !result_text.is_char_boundary(cut) {
                    cut -= 1;
                }
                format!("{}...[truncated]", &result_text[..cut])
            } else {
                result_text
            };
            put_cached_url(&url, bytes, code, &code_text, &content_type, &result_text);
            (bytes, code, code_text, content_type, result_text)
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        // Apply the user's prompt via the secondary model (TS parity). If
        // no secondary model is registered, return the fetched content in
        // the same output shape so the primary model can still continue.
        // The is_preapproved flag relaxes the quoting guidelines applied
        // to the secondary-model prompt (TS makeSecondaryModelPrompt).
        let is_preapproved = crate::web_fetch_preapproved::is_preapproved_url(&url);
        if is_preapproved
            && content_type.contains("text/markdown")
            && result_text.len() < MAX_MARKDOWN_LENGTH
        {
            return Ok(ToolResultData {
                data: json!({
                    "bytes": bytes,
                    "code": code,
                    "codeText": code_text,
                    "result": result_text,
                    "durationMs": duration_ms,
                    "url": url,
                }),
                is_error: false,
            });
        }
        match apply_prompt_to_markdown(&prompt, &result_text, is_preapproved, cancel.clone()).await
        {
            Ok(Some(summary)) => Ok(ToolResultData {
                data: json!({
                    "bytes": bytes,
                    "code": code,
                    "codeText": code_text,
                    "result": summary,
                    "durationMs": duration_ms,
                    "url": url,
                }),
                is_error: false,
            }),
            Ok(None) => Ok(ToolResultData {
                data: json!({
                    "bytes": bytes,
                    "code": code,
                    "codeText": code_text,
                    "result": result_text,
                    "durationMs": duration_ms,
                    "url": url,
                }),
                is_error: false,
            }),
            Err(e) => {
                // Fall back to raw content + prompt echo on secondary-model failure.
                tracing::warn!("secondary model failed for WebFetch: {}", e);
                Ok(ToolResultData {
                    data: json!({
                        "bytes": bytes,
                        "code": code,
                        "codeText": code_text,
                        "result": result_text,
                        "durationMs": duration_ms,
                        "url": url,
                    }),
                    is_error: false,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn preapproved_guidelines_are_terser() {
        let pre = make_secondary_model_prompt("<html>", "summarise", true);
        let un = make_secondary_model_prompt("<html>", "summarise", false);
        assert!(pre.contains("Provide a concise response based on the content above"));
        assert!(un.contains("125-character maximum"));
        assert!(pre.len() < un.len());
    }

    #[tokio::test]
    async fn no_model_registered_returns_none() {
        // The global model is never installed in this test harness, so
        // apply_prompt_to_markdown must gracefully return None.
        let out =
            apply_prompt_to_markdown("summarise", "hello world", false, CancellationToken::new())
                .await
                .expect("no error when model absent");
        assert!(out.is_none());
    }

    #[test]
    fn url_validation_matches_ts_security_shape() {
        assert!(validate_url("https://example.com/docs"));
        assert!(validate_url("http://example.com/docs"));
        assert!(!validate_url("not a url"));
        assert!(!validate_url("https://localhost/docs"));
        assert!(!validate_url("https://user:pass@example.com/docs"));
    }

    #[test]
    fn http_urls_are_upgraded_to_https() {
        assert_eq!(
            upgrade_http_to_https("http://example.com/a?q=1")
                .unwrap()
                .as_str(),
            "https://example.com/a?q=1"
        );
        assert_eq!(
            upgrade_http_to_https("https://example.com/a")
                .unwrap()
                .as_str(),
            "https://example.com/a"
        );
    }

    #[test]
    fn url_cache_keeps_processed_content_by_original_url() {
        put_cached_url(
            "https://example.com/a",
            42,
            200,
            "OK",
            "text/markdown",
            "# Cached",
        );
        let cached = get_cached_url("https://example.com/a").unwrap();
        assert_eq!(cached.bytes, 42);
        assert_eq!(cached.code, 200);
        assert_eq!(cached.code_text, "OK");
        assert_eq!(cached.content_type, "text/markdown");
        assert_eq!(cached.content, "# Cached");
        assert!(get_cached_url("https://example.com/b").is_none());
    }

    #[test]
    fn redirect_rules_match_ts_host_boundary() {
        assert!(is_permitted_redirect(
            "https://example.com/a",
            "https://www.example.com/b"
        ));
        assert!(is_permitted_redirect(
            "https://www.example.com/a",
            "https://example.com/b"
        ));
        assert!(!is_permitted_redirect(
            "https://example.com/a",
            "https://evil.example/b"
        ));
        assert!(!is_permitted_redirect(
            "https://example.com/a",
            "http://example.com/b"
        ));
    }

    #[test]
    fn permission_rules_are_domain_scoped_like_ts() {
        let tool = WebFetchTool;
        let input = json!({"url": "https://example.com/a", "prompt": "summarise"});
        let mut allow_rules = HashMap::new();
        allow_rules.insert(
            PermissionRuleSource::LocalSettings,
            vec!["WebFetch(domain:example.com)".to_string()],
        );
        let context = ToolPermissionContext {
            always_allow_rules: allow_rules,
            ..Default::default()
        };
        assert!(matches!(
            tool.check_permissions(&input, &context),
            PermissionResult::Allow(_)
        ));

        let context = ToolPermissionContext::default();
        match tool.check_permissions(&input, &context) {
            PermissionResult::Ask(ask) => {
                assert!(ask.message.contains("haven't granted it yet"));
                assert!(ask
                    .suggestions
                    .as_ref()
                    .is_some_and(|suggestions| !suggestions.is_empty()));
            }
            other => panic!("expected ask, got {other:?}"),
        }
    }
}
