use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VcrFixture {
    pub version: u32,
    pub requests: Vec<VcrRequest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VcrRequest {
    pub method: String,
    pub url: String,
    pub request_body: serde_json::Value,
    pub response_status: u16,
    pub response_body: String,
    pub timestamp: u64,
}
