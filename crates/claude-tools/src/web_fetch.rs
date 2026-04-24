use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::secondary_model;
use claude_core::types::events::ToolResultData;

pub struct WebFetchTool;

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

        let start = std::time::Instant::now();

        // Build reqwest client
        let client = match reqwest::Client::builder()
            .user_agent("claude-rs/0.1")
            .timeout(std::time::Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => return Ok(error_result(format!("failed to build HTTP client: {}", e))),
        };

        // Make the request, respecting cancellation
        let response = tokio::select! {
            _ = cancel.cancelled() => {
                return Ok(error_result("request cancelled"));
            }
            res = client.get(&url).send() => {
                match res {
                    Ok(r) => r,
                    Err(e) => return Ok(error_result(format!("HTTP request failed: {}", e))),
                }
            }
        };

        let status = response.status();
        let code = status.as_u16();
        let code_text = status.canonical_reason().unwrap_or("Unknown").to_string();

        // Read body
        let body_bytes = tokio::select! {
            _ = cancel.cancelled() => {
                return Ok(error_result("request cancelled while reading body"));
            }
            res = response.bytes() => {
                match res {
                    Ok(b) => b,
                    Err(e) => return Ok(error_result(format!("failed to read response body: {}", e))),
                }
            }
        };

        let bytes = body_bytes.len() as u64;
        let duration_ms = start.elapsed().as_millis() as u64;

        // Convert body to string
        let body_text = String::from_utf8_lossy(&body_bytes).into_owned();

        // Strip HTML if it looks like HTML
        let result_text = if body_text.trim_start().starts_with('<') {
            strip_html(&body_text)
        } else {
            body_text
        };

        // Truncate to reasonable size before sending anything downstream.
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

        // Apply the user's prompt via the secondary model (TS parity). If
        // no secondary model is registered, echo the prompt back in the
        // result so the primary model can apply it to the raw content
        // itself — this is strictly better than silently discarding it.
        // The is_preapproved flag relaxes the quoting guidelines applied
        // to the secondary-model prompt (TS makeSecondaryModelPrompt).
        let is_preapproved = crate::web_fetch_preapproved::is_preapproved_url(&url);
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
                    "promptApplied": true,
                }),
                is_error: false,
            }),
            Ok(None) => Ok(ToolResultData {
                data: json!({
                    "bytes": bytes,
                    "code": code,
                    "codeText": code_text,
                    "result": result_text,
                    "prompt": prompt,
                    "durationMs": duration_ms,
                    "url": url,
                    "promptApplied": false,
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
                        "prompt": prompt,
                        "durationMs": duration_ms,
                        "url": url,
                        "promptApplied": false,
                        "secondaryModelError": e.to_string(),
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
}
