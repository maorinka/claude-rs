use super::types::*;
use anyhow::Result;
use std::path::Path;

pub struct VcrPlayer {
    fixture: VcrFixture,
    index: usize,
}

impl VcrPlayer {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let fixture: VcrFixture = serde_json::from_str(&content)?;
        Ok(Self { fixture, index: 0 })
    }

    pub fn next_response(&mut self) -> Option<&VcrRequest> {
        if self.index < self.fixture.requests.len() {
            let req = &self.fixture.requests[self.index];
            self.index += 1;
            Some(req)
        } else {
            None
        }
    }

    pub fn remaining(&self) -> usize {
        self.fixture.requests.len() - self.index
    }

    pub fn reset(&mut self) {
        self.index = 0;
    }
}
