//! Inbound bridge attachment helpers.
//!
//! Mirrors TS `src/bridge/inboundAttachments.ts`: parse `file_attachments`,
//! fetch each OAuth-backed file into `~/.claude/uploads/{sessionId}/`, and
//! prepend quoted `@"/path"` references to the last text block.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const DOWNLOAD_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundAttachment {
    pub file_uuid: String,
    pub file_name: String,
}

#[derive(Debug, Clone)]
pub struct InboundAttachmentResolver {
    pub access_token: Option<String>,
    pub base_url: String,
    pub config_home: PathBuf,
    pub session_id: String,
    pub client: reqwest::Client,
}

impl InboundAttachmentResolver {
    pub async fn from_runtime_config(session_id: impl Into<String>) -> Result<Self> {
        let access_token = bridge_access_token().await;
        let base_url = bridge_base_url()?;
        let config_home = crate::config::paths::claude_dir()?;
        Ok(Self::new(access_token, base_url, config_home, session_id))
    }

    pub fn new(
        access_token: Option<String>,
        base_url: impl Into<String>,
        config_home: PathBuf,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            access_token,
            base_url: base_url.into(),
            config_home,
            session_id: session_id.into(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn resolve_inbound_attachments(&self, attachments: &[InboundAttachment]) -> String {
        if attachments.is_empty() {
            return String::new();
        }
        tracing::debug!(
            target: "bridge::inbound_attachments",
            count = attachments.len(),
            "resolving inbound attachment(s)"
        );

        let mut ok = Vec::new();
        for attachment in attachments {
            if let Some(path) = self.resolve_one(attachment).await {
                ok.push(path);
            }
        }
        if ok.is_empty() {
            return String::new();
        }
        ok.into_iter()
            .map(|path| format!("@\"{}\"", path.display()))
            .collect::<Vec<_>>()
            .join(" ")
            + " "
    }

    pub async fn resolve_and_prepend(&self, msg: &Value, content: Value) -> Value {
        let attachments = extract_inbound_attachments(msg);
        if attachments.is_empty() {
            return content;
        }
        let prefix = self.resolve_inbound_attachments(&attachments).await;
        prepend_path_refs(content, &prefix)
    }

    async fn resolve_one(&self, attachment: &InboundAttachment) -> Option<PathBuf> {
        let Some(token) = self
            .access_token
            .as_deref()
            .filter(|token| !token.trim().is_empty())
        else {
            tracing::debug!(target: "bridge::inbound_attachments", "skip: no oauth token");
            return None;
        };

        let url = format!(
            "{}/api/oauth/files/{}/content",
            self.base_url.trim_end_matches('/'),
            urlencoding::encode(&attachment.file_uuid)
        );
        let response = match self
            .client
            .get(url)
            .bearer_auth(token)
            .timeout(Duration::from_millis(DOWNLOAD_TIMEOUT_MS))
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::debug!(
                    target: "bridge::inbound_attachments",
                    file_uuid = attachment.file_uuid,
                    error = %err,
                    "attachment fetch failed"
                );
                return None;
            }
        };
        if response.status().as_u16() != 200 {
            tracing::debug!(
                target: "bridge::inbound_attachments",
                file_uuid = attachment.file_uuid,
                status = response.status().as_u16(),
                "attachment fetch returned non-200"
            );
            return None;
        }
        let data = match response.bytes().await {
            Ok(data) => data,
            Err(err) => {
                tracing::debug!(
                    target: "bridge::inbound_attachments",
                    file_uuid = attachment.file_uuid,
                    error = %err,
                    "attachment body read failed"
                );
                return None;
            }
        };

        let dir = uploads_dir(&self.config_home, &self.session_id);
        let out_path = dir.join(prefixed_safe_file_name(attachment));
        if let Err(err) = write_attachment(&dir, &out_path, &data).await {
            tracing::debug!(
                target: "bridge::inbound_attachments",
                path = %out_path.display(),
                error = %err,
                "attachment write failed"
            );
            return None;
        }

        tracing::debug!(
            target: "bridge::inbound_attachments",
            file_uuid = attachment.file_uuid,
            path = %out_path.display(),
            bytes = data.len(),
            "resolved inbound attachment"
        );
        Some(out_path)
    }
}

async fn bridge_access_token() -> Option<String> {
    if crate::user_type::is_ant() {
        if let Ok(token) = std::env::var("CLAUDE_BRIDGE_OAUTH_TOKEN") {
            if !token.trim().is_empty() {
                return Some(token);
            }
        }
    }
    crate::auth::storage::load_tokens()
        .await
        .ok()
        .flatten()
        .map(|tokens| tokens.access_token)
        .filter(|token| !token.trim().is_empty())
}

fn bridge_base_url() -> Result<String> {
    if crate::user_type::is_ant() {
        if let Ok(base_url) = std::env::var("CLAUDE_BRIDGE_BASE_URL") {
            if !base_url.trim().is_empty() {
                return Ok(base_url);
            }
        }
    }
    Ok(crate::constants::oauth::get_oauth_config()?.base_api_url)
}

pub fn extract_inbound_attachments(msg: &Value) -> Vec<InboundAttachment> {
    let Some(array) = msg.get("file_attachments").and_then(Value::as_array) else {
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|item| {
            Some(InboundAttachment {
                file_uuid: item.get("file_uuid")?.as_str()?.to_string(),
                file_name: item.get("file_name")?.as_str()?.to_string(),
            })
        })
        .collect()
}

pub fn prepend_path_refs(content: Value, prefix: &str) -> Value {
    if prefix.is_empty() {
        return content;
    }
    if let Some(text) = content.as_str() {
        return Value::String(format!("{prefix}{text}"));
    }
    let Some(blocks) = content.as_array() else {
        return content;
    };
    let mut blocks = blocks.clone();
    if let Some(index) = blocks
        .iter()
        .rposition(|block| block.get("type").and_then(Value::as_str) == Some("text"))
    {
        if let Some(text) = blocks[index].get("text").and_then(Value::as_str) {
            let mut block = blocks[index].clone();
            block["text"] = Value::String(format!("{prefix}{text}"));
            blocks[index] = block;
            return Value::Array(blocks);
        }
    }
    blocks.push(json!({"type": "text", "text": prefix.trim_end()}));
    Value::Array(blocks)
}

pub fn sanitize_file_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(name);
    let safe: String = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        "attachment".to_string()
    } else {
        safe
    }
}

pub fn uploads_dir(config_home: &Path, session_id: &str) -> PathBuf {
    config_home.join("uploads").join(session_id)
}

fn prefixed_safe_file_name(attachment: &InboundAttachment) -> String {
    let prefix_raw = if attachment.file_uuid.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        attachment.file_uuid.chars().take(8).collect()
    };
    let prefix: String = prefix_raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix}-{}", sanitize_file_name(&attachment.file_name))
}

async fn write_attachment(dir: &Path, path: &Path, data: &[u8]) -> Result<()> {
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("creating {}", dir.display()))?;
    tokio::fs::write(path, data)
        .await
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_valid_file_attachments_only() {
        let msg = json!({
            "file_attachments": [
                {"file_uuid": "file-1", "file_name": "a.txt"},
                {"file_uuid": 123, "file_name": "bad"},
                {"file_uuid": "file-2"}
            ]
        });
        assert_eq!(
            extract_inbound_attachments(&msg),
            vec![InboundAttachment {
                file_uuid: "file-1".to_string(),
                file_name: "a.txt".to_string()
            }]
        );
        assert!(extract_inbound_attachments(&json!({"file_attachments": "bad"})).is_empty());
    }

    #[test]
    fn sanitizes_file_name_like_ts() {
        assert_eq!(sanitize_file_name("../a b?.txt"), "a_b_.txt");
        assert_eq!(sanitize_file_name(""), "attachment");
        assert_eq!(sanitize_file_name("safe-name_1.pdf"), "safe-name_1.pdf");
    }

    #[test]
    fn prepends_to_string_content() {
        assert_eq!(
            prepend_path_refs(json!("hello"), "@\"/tmp/a b.txt\" "),
            json!("@\"/tmp/a b.txt\" hello")
        );
    }

    #[test]
    fn prepends_to_last_text_block() {
        let content = json!([
            {"type": "text", "text": "first"},
            {"type": "image", "source": {"type": "base64", "data": "abc"}},
            {"type": "text", "text": "last"}
        ]);
        let out = prepend_path_refs(content, "@\"/tmp/a.txt\" ");
        assert_eq!(out[0]["text"], "first");
        assert_eq!(out[2]["text"], "@\"/tmp/a.txt\" last");
    }

    #[test]
    fn appends_text_block_when_no_text_exists() {
        let content = json!([{"type": "image", "source": {"type": "base64", "data": "abc"}}]);
        let out = prepend_path_refs(content, "@\"/tmp/a.txt\" ");
        assert_eq!(out[1], json!({"type": "text", "text": "@\"/tmp/a.txt\""}));
    }
}
