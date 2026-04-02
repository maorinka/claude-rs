use anyhow::Result;
use async_trait::async_trait;
use claude_core::types::events::{ToolProgressData, ToolResultData};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub type ProgressSender = mpsc::Sender<ToolProgressData>;

/// Metadata recorded when a file is read. Used by Write/Edit tools
/// to detect staleness (file modified externally after we last read it).
#[derive(Debug, Clone)]
pub struct ReadFileEntry {
    /// Milliseconds since UNIX epoch when the read was performed.
    pub timestamp: u64,
    /// Whether this was a partial view (offset/limit supplied).
    pub is_partial_view: bool,
}

/// Shared state tracking which files have been read and when.
///
/// The `FileReadTool` records entries here; `FileWriteTool` and `FileEditTool`
/// check entries before allowing writes (matching the TS `readFileState`).
#[derive(Debug, Clone, Default)]
pub struct ReadFileState {
    entries: HashMap<String, ReadFileEntry>,
}

impl ReadFileState {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Record that a file was read at the current time.
    pub fn record_read(&mut self, path: &str, is_partial_view: bool) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.entries.insert(
            path.to_string(),
            ReadFileEntry {
                timestamp: now,
                is_partial_view,
            },
        );
    }

    /// Update the read timestamp for a file after a successful write, so
    /// subsequent writes do not get rejected as stale.
    pub fn update_after_write(&mut self, path: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.entries.insert(
            path.to_string(),
            ReadFileEntry {
                timestamp: now,
                is_partial_view: false,
            },
        );
    }

    /// Get the read entry for a file, if it exists.
    pub fn get(&self, path: &str) -> Option<&ReadFileEntry> {
        self.entries.get(path)
    }

    /// Insert a read entry with an explicit timestamp (for testing).
    #[cfg(test)]
    pub fn insert_raw(&mut self, path: &str, entry: ReadFileEntry) {
        self.entries.insert(path.to_string(), entry);
    }

    /// Insert a read entry with an explicit timestamp.
    /// This is `pub(crate)` so that sibling modules (e.g. `write` tests) can
    /// set up fixture state.
    #[allow(dead_code)]
    pub(crate) fn insert_entry(&mut self, path: &str, entry: ReadFileEntry) {
        self.entries.insert(path.to_string(), entry);
    }
}

pub struct ToolUseContext {
    pub working_directory: PathBuf,
    pub read_file_state: Arc<std::sync::Mutex<ReadFileState>>,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    fn name(&self) -> &str;
    fn aliases(&self) -> &[&str] {
        &[]
    }
    /// Full description of what this tool does, sent to the API as the tool's
    /// `description` field.  Mirrors the TS `tool.prompt()` output.
    fn description(&self) -> String {
        format!("Tool: {}", self.name())
    }
    fn input_schema(&self) -> Value;
    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        cancel: CancellationToken,
        progress: Option<ProgressSender>,
    ) -> Result<ToolResultData>;
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }
    fn is_read_only(&self, _input: &Value) -> bool {
        false
    }
    fn is_destructive(&self, _input: &Value) -> bool {
        false
    }
    fn max_result_size_chars(&self) -> usize {
        100_000
    }
}

#[derive(Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn ToolExecutor>>,
    aliases: HashMap<String, String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn ToolExecutor>) {
        let name = tool.name().to_string();
        for alias in tool.aliases() {
            self.aliases.insert(alias.to_string(), name.clone());
        }
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ToolExecutor>> {
        self.tools
            .get(name)
            .or_else(|| self.aliases.get(name).and_then(|n| self.tools.get(n)))
            .cloned()
    }

    pub fn all(&self) -> Vec<Arc<dyn ToolExecutor>> {
        self.tools.values().cloned().collect()
    }

    pub fn schemas(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| serde_json::json!({"name": t.name(), "input_schema": t.input_schema()}))
            .collect()
    }

    pub fn tool_definitions(&self) -> Vec<claude_core::api::client::ToolDefinition> {
        self.tools
            .values()
            .map(|t| claude_core::api::client::ToolDefinition {
                name: t.name().to_string(),
                description: t.description(),
                input_schema: t.input_schema(),
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
