use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::registry::ToolUseContext;

pub const TOOL_RESULTS_SUBDIR: &str = "tool-results";

pub(crate) fn sanitize_session_path(name: &str) -> String {
    name.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

pub(crate) fn tool_results_dir(ctx: &ToolUseContext) -> Result<PathBuf> {
    let session_id = ctx
        .options
        .session_id
        .as_deref()
        .unwrap_or_else(|| claude_core::api::client::get_session_id().as_str());
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home
        .join(".claude")
        .join("projects")
        .join(sanitize_session_path(
            &ctx.working_directory.display().to_string(),
        ))
        .join(session_id)
        .join(TOOL_RESULTS_SUBDIR))
}

pub(crate) async fn persist_binary_content(
    bytes: &[u8],
    mime_type: Option<&str>,
    persist_id: &str,
    ctx: &ToolUseContext,
) -> Result<(PathBuf, usize)> {
    let dir = tool_results_dir(ctx)?;
    tokio::fs::create_dir_all(&dir).await?;
    let ext = claude_core::mcp::output_storage::extension_for_mime_type(mime_type);
    let filepath = dir.join(format!("{persist_id}.{ext}"));
    tokio::fs::write(&filepath, bytes).await?;
    Ok((filepath, bytes.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_path_sanitizer_matches_ts_shape() {
        assert_eq!(
            sanitize_session_path("/Users/alice/work/claude-rs"),
            "-Users-alice-work-claude-rs"
        );
    }
}
