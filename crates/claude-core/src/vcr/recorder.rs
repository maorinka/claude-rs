use super::types::*;
use anyhow::Result;
use std::path::Path;

pub struct VcrRecorder {
    requests: Vec<VcrRequest>,
    enabled: bool,
}

impl VcrRecorder {
    pub fn new(enabled: bool) -> Self {
        Self {
            requests: Vec::new(),
            enabled,
        }
    }

    pub fn record(
        &mut self,
        method: &str,
        url: &str,
        request_body: &serde_json::Value,
        response_status: u16,
        response_body: &str,
    ) {
        if !self.enabled {
            return;
        }
        self.requests.push(VcrRequest {
            method: method.to_string(),
            url: url.to_string(),
            request_body: request_body.clone(),
            response_status,
            response_body: response_body.to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        });
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let fixture = VcrFixture {
            version: 1,
            requests: self.requests.clone(),
        };
        let json = serde_json::to_string_pretty(&fixture)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn request_count(&self) -> usize {
        self.requests.len()
    }
}
