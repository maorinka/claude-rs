use super::content::ContentBlock;
use super::usage::Usage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    #[serde(rename = "user")]
    User(UserMessage),
    #[serde(rename = "assistant")]
    Assistant(AssistantMessage),
    #[serde(rename = "system")]
    System(SystemMessage),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserMessage {
    pub uuid: Uuid,
    pub content: Vec<ContentBlock>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub uuid: Uuid,
    pub message: ApiMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiMessage {
    pub id: String,
    pub model: String,
    pub role: Role,
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    pub usage: Usage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "subtype")]
pub enum SystemMessage {
    #[serde(rename = "compact_boundary")]
    CompactBoundary { summary: String },
    #[serde(rename = "memory_saved")]
    MemorySaved { path: String },
    #[serde(rename = "api_error")]
    ApiError {
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    #[serde(rename = "local_command")]
    LocalCommand { command: String, output: String },
    #[serde(rename = "hook_progress")]
    HookProgress { hook_name: String, message: String },
    #[serde(rename = "rate_limit")]
    RateLimit {
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_after: Option<u64>,
    },
}
