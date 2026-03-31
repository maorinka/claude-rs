//! Proxy API calls through the installed `claude` binary.
//!
//! Claude Code's OAuth tokens only work with Anthropic's internal SDK (bundled
//! in the `claude` binary). For OAuth users, we delegate the actual API call
//! to `claude -p --output-format stream-json` and parse its output.

use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

/// Check if the `claude` binary is available.
pub fn is_claude_available() -> bool {
    std::process::Command::new("claude")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Send a prompt through the real `claude` binary and stream the text response
/// line by line via the callback.
pub async fn stream_via_claude(
    prompt: &str,
    model: Option<&str>,
    cancel: CancellationToken,
    mut on_text: impl FnMut(&str),
) -> Result<()> {
    let mut cmd = Command::new("claude");
    cmd.arg("-p").arg(prompt);
    cmd.arg("--output-format").arg("text");

    if let Some(m) = model {
        cmd.arg("--model").arg(m);
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().context("Failed to start claude binary")?;

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                break;
            }
            line = reader.next_line() => {
                match line {
                    Ok(Some(text)) => {
                        on_text(&text);
                        on_text("\n");
                    }
                    Ok(None) => break, // EOF
                    Err(e) => {
                        tracing::warn!("Error reading claude output: {}", e);
                        break;
                    }
                }
            }
        }
    }

    let _ = child.wait().await;
    Ok(())
}
