use anyhow::Result;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::types::events::ToolResultData;

/// Info about a tool to execute
#[derive(Clone, Debug)]
pub struct PendingTool {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub is_concurrent: bool,
}

/// Result of tool execution
#[derive(Debug)]
pub struct CompletedTool {
    pub id: String,
    pub name: String,
    pub result: Result<ToolResultData>,
}

/// Callback type for executing a single tool
pub type ToolCallFn = Arc<
    dyn Fn(
            String,
            String,
            Value,
            CancellationToken,
        ) -> tokio::task::JoinHandle<Result<ToolResultData>>
        + Send
        + Sync,
>;

/// Executes tools with concurrency control.
/// Concurrent-safe tools run in parallel.
/// Non-concurrent tools run exclusively.
pub struct StreamingToolExecutor {
    cancel: CancellationToken,
    executing: JoinSet<CompletedTool>,
    pending_exclusive: VecDeque<PendingTool>,
    running_concurrent_count: usize,
    running_exclusive: bool,
    tool_call_fn: ToolCallFn,
    completed: Vec<CompletedTool>,
}

impl StreamingToolExecutor {
    pub fn new(cancel: CancellationToken, tool_call_fn: ToolCallFn) -> Self {
        Self {
            cancel,
            executing: JoinSet::new(),
            pending_exclusive: VecDeque::new(),
            running_concurrent_count: 0,
            running_exclusive: false,
            tool_call_fn,
            completed: Vec::new(),
        }
    }

    /// Add a tool for execution. Concurrent tools start immediately if possible.
    pub fn add_tool(&mut self, tool: PendingTool) {
        if tool.is_concurrent && !self.running_exclusive {
            self.spawn_tool(tool);
        } else {
            self.pending_exclusive.push_back(tool);
        }
    }

    /// Check if any tools have completed. Non-blocking.
    pub fn poll_completed(&mut self) -> Vec<CompletedTool> {
        // Try to join any completed tasks
        while let Some(result) = self.executing.try_join_next() {
            match result {
                Ok(completed) => {
                    if completed.result.is_ok() {
                        // Adjust counts (simplified — we don't track per-task concurrency here)
                    }
                    self.completed.push(completed);
                }
                Err(e) => {
                    tracing::warn!("Tool task panicked: {}", e);
                }
            }
        }

        // If nothing running, start pending exclusive tools
        if self.executing.is_empty() && !self.pending_exclusive.is_empty() {
            self.running_exclusive = false;
            self.running_concurrent_count = 0;

            if let Some(tool) = self.pending_exclusive.pop_front() {
                if tool.is_concurrent {
                    // Start this and any subsequent concurrent tools
                    self.spawn_tool(tool);
                    while self
                        .pending_exclusive
                        .front()
                        .map_or(false, |t| t.is_concurrent)
                    {
                        let t = self.pending_exclusive.pop_front().unwrap();
                        self.spawn_tool(t);
                    }
                } else {
                    self.running_exclusive = true;
                    self.spawn_tool(tool);
                }
            }
        }

        std::mem::take(&mut self.completed)
    }

    /// Wait for all tools to complete
    pub async fn flush(&mut self) -> Vec<CompletedTool> {
        while let Some(result) = self.executing.join_next().await {
            match result {
                Ok(completed) => self.completed.push(completed),
                Err(e) => tracing::warn!("Tool task panicked: {}", e),
            }
        }

        // Process any remaining pending
        while !self.pending_exclusive.is_empty() {
            if let Some(tool) = self.pending_exclusive.pop_front() {
                self.spawn_tool(tool);
            }
            while let Some(result) = self.executing.join_next().await {
                match result {
                    Ok(completed) => self.completed.push(completed),
                    Err(e) => tracing::warn!("Tool task panicked: {}", e),
                }
            }
        }

        std::mem::take(&mut self.completed)
    }

    pub fn has_pending(&self) -> bool {
        !self.executing.is_empty() || !self.pending_exclusive.is_empty()
    }

    fn spawn_tool(&mut self, tool: PendingTool) {
        let cancel = self.cancel.child_token();
        let call_fn = self.tool_call_fn.clone();
        let id = tool.id.clone();
        let name = tool.name.clone();
        let input = tool.input.clone();

        if tool.is_concurrent {
            self.running_concurrent_count += 1;
        }

        self.executing.spawn(async move {
            let handle = call_fn(name.clone(), id.clone(), input, cancel);
            let result = handle
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("Task join error: {}", e)));
            CompletedTool { id, name, result }
        });
    }
}
