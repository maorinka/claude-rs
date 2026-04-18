//! Optional secondary ("fast/small") model used by tools that need a cheap
//! LLM call for post-processing — e.g. WebFetchTool's `applyPromptToMarkdown`
//! which summarises fetched content against a user prompt.
//!
//! Mirrors TS `services/api/claude.ts::queryHaiku()` from the reference
//! implementation. Parked behind a trait + global OnceLock so tools in
//! `claude-tools/` can use it without plumbing an ApiClient through every
//! `ToolUseContext` construction site.
//!
//! The application entry point (CLI/TUI) is responsible for calling
//! `set_global()` once at startup to wire in a concrete implementation.
//! Tools that call `get_global()` when no model is registered gracefully
//! fall back to a no-op path.

use std::sync::{Arc, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

/// A minimal secondary-model interface: take a fully-formed user prompt,
/// return the model's text response.
#[async_trait]
pub trait SecondaryModel: Send + Sync {
    async fn summarize(&self, user_prompt: &str, cancel: CancellationToken) -> Result<String>;
}

static GLOBAL: OnceLock<Arc<dyn SecondaryModel>> = OnceLock::new();

/// Register the process-wide secondary model. Only the first call wins —
/// subsequent calls are silently ignored (matches `OnceLock` semantics).
pub fn set_global(model: Arc<dyn SecondaryModel>) {
    let _ = GLOBAL.set(model);
}

/// Fetch the registered secondary model if one has been installed.
pub fn get_global() -> Option<Arc<dyn SecondaryModel>> {
    GLOBAL.get().cloned()
}
