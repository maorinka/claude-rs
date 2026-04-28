use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::env;
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

#[derive(Debug)]
struct QueuedTool {
    index: usize,
    tool: PendingTool,
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
    executing: JoinSet<(usize, bool, CompletedTool)>,
    queued: VecDeque<QueuedTool>,
    active: BTreeMap<usize, bool>,
    completed_buffer: BTreeMap<usize, CompletedTool>,
    next_index: usize,
    next_yield_index: usize,
    running_concurrent_count: usize,
    running_exclusive: bool,
    max_concurrent_tools: usize,
    tool_call_fn: ToolCallFn,
}

impl StreamingToolExecutor {
    pub fn new(cancel: CancellationToken, tool_call_fn: ToolCallFn) -> Self {
        Self {
            cancel,
            executing: JoinSet::new(),
            queued: VecDeque::new(),
            active: BTreeMap::new(),
            completed_buffer: BTreeMap::new(),
            next_index: 0,
            next_yield_index: 0,
            running_concurrent_count: 0,
            running_exclusive: false,
            max_concurrent_tools: get_max_tool_use_concurrency(),
            tool_call_fn,
        }
    }

    /// Add a tool for execution. Concurrent tools start immediately if possible.
    pub fn add_tool(&mut self, tool: PendingTool) {
        let index = self.next_index;
        self.next_index += 1;
        self.queued.push_back(QueuedTool { index, tool });
        self.process_queue();
    }

    /// Check if any tools have completed. Non-blocking.
    pub fn poll_completed(&mut self) -> Vec<CompletedTool> {
        // Try to join any completed tasks
        while let Some(result) = self.executing.try_join_next() {
            match result {
                Ok((index, is_concurrent, completed)) => {
                    self.active.remove(&index);
                    if is_concurrent {
                        self.running_concurrent_count =
                            self.running_concurrent_count.saturating_sub(1);
                    } else {
                        self.running_exclusive = false;
                    }
                    self.completed_buffer.insert(index, completed);
                }
                Err(e) => {
                    tracing::warn!("Tool task panicked: {}", e);
                }
            }
        }

        self.process_queue();
        self.take_ordered_completed()
    }

    /// Wait for all tools to complete
    pub async fn flush(&mut self) -> Vec<CompletedTool> {
        while !self.executing.is_empty() || !self.queued.is_empty() {
            self.process_queue();
            if let Some(result) = self.executing.join_next().await {
                match result {
                    Ok((index, is_concurrent, completed)) => {
                        self.active.remove(&index);
                        if is_concurrent {
                            self.running_concurrent_count =
                                self.running_concurrent_count.saturating_sub(1);
                        } else {
                            self.running_exclusive = false;
                        }
                        self.completed_buffer.insert(index, completed);
                    }
                    Err(e) => tracing::warn!("Tool task panicked: {}", e),
                }
            }
        }

        self.take_ordered_completed()
    }

    pub fn has_pending(&self) -> bool {
        !self.executing.is_empty() || !self.queued.is_empty() || !self.completed_buffer.is_empty()
    }

    fn process_queue(&mut self) {
        while self
            .queued
            .front()
            .is_some_and(|queued| self.can_execute(queued.tool.is_concurrent))
        {
            let queued = self.queued.pop_front().expect("front checked above");
            self.spawn_tool(queued.index, queued.tool);
        }
    }

    fn can_execute(&self, is_concurrent: bool) -> bool {
        if self.active.is_empty() {
            return true;
        }
        is_concurrent
            && self.active.values().all(|active| *active)
            && self.running_concurrent_count < self.max_concurrent_tools
    }

    fn take_ordered_completed(&mut self) -> Vec<CompletedTool> {
        let mut out = Vec::new();
        while let Some(completed) = self.completed_buffer.remove(&self.next_yield_index) {
            self.next_yield_index += 1;
            out.push(completed);
        }
        out
    }

    fn spawn_tool(&mut self, index: usize, tool: PendingTool) {
        let cancel = self.cancel.child_token();
        let call_fn = self.tool_call_fn.clone();
        let id = tool.id.clone();
        let name = tool.name.clone();
        let input = tool.input.clone();
        let is_concurrent = tool.is_concurrent;

        self.active.insert(index, is_concurrent);
        if is_concurrent {
            self.running_concurrent_count += 1;
        } else {
            self.running_exclusive = true;
        }

        self.executing.spawn(async move {
            let handle = call_fn(name.clone(), id.clone(), input, cancel);
            let result = handle
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("Task join error: {}", e)));
            (index, is_concurrent, CompletedTool { id, name, result })
        });
    }
}

fn get_max_tool_use_concurrency() -> usize {
    env::var("CLAUDE_CODE_MAX_TOOL_USE_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(10)
}
