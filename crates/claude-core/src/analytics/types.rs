use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalyticsEvent {
    pub event_name: String,
    pub timestamp: u64,
    pub properties: serde_json::Value,
    pub session_id: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub session_id: String,
    pub device_id: String,
    pub model: String,
    pub subscription_type: Option<String>,
    pub platform: String,
    pub cli_version: String,
}
