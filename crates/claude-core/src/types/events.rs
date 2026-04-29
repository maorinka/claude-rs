use super::error::QueryError;
use super::message::{AssistantMessage, StopReason};
use super::usage::Usage;

#[derive(Clone, Debug)]
pub enum StreamEvent {
    RequestStart {
        request_id: String,
    },
    RawSse {
        event: serde_json::Value,
    },
    AssistantMessage(AssistantMessage),
    ToolStart {
        tool_use_id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolProgress {
        tool_use_id: String,
        progress: ToolProgressData,
    },
    ToolResult {
        tool_use_id: String,
        result: ToolResultData,
    },
    ThinkingDelta {
        text: String,
    },
    TextDelta {
        text: String,
    },
    Compacted {
        summary: String,
    },
    UsageUpdate(Usage),
    Done {
        stop_reason: StopReason,
    },
    Error(QueryError),
}

#[derive(Clone, Debug)]
pub enum ToolProgressData {
    BashProgress { stdout: String, stderr: String },
    ReadProgress { bytes_read: u64 },
    WebSearchProgress { results_found: u32 },
}

#[derive(Clone, Debug)]
pub struct ToolResultData {
    pub data: serde_json::Value,
    pub is_error: bool,
}
