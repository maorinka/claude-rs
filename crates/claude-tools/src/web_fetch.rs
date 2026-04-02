use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::types::events::ToolResultData;

pub struct WebFetchTool;

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

#[async_trait]
impl ToolExecutor for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn description(&self) -> String {
        "Fetch the contents of a URL and return the text content. Supports HTML pages (converted to text), JSON, and plain text responses.".to_string()
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
        let _prompt = match input["prompt"].as_str() {
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

        // Truncate to reasonable size
        const MAX_CHARS: usize = 50_000;
        let result_text = if result_text.len() > MAX_CHARS {
            format!("{}...[truncated]", &result_text[..MAX_CHARS])
        } else {
            result_text
        };

        Ok(ToolResultData {
            data: json!({
                "bytes": bytes,
                "code": code,
                "codeText": code_text,
                "result": result_text,
                "durationMs": duration_ms,
                "url": url
            }),
            is_error: false,
        })
    }
}
