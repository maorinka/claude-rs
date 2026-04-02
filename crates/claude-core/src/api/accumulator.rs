use anyhow::{anyhow, Result};
use std::collections::HashMap;

use crate::api::sse::{ContentBlockStart, ContentDelta};
use crate::types::content::ContentBlock;

// ── In-progress block state ───────────────────────────────────────────────────

enum InProgressBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    ServerToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

// ── Accumulator ───────────────────────────────────────────────────────────────

/// Accumulates streaming SSE content-block deltas into finalized [`ContentBlock`]s.
pub struct ContentBlockAccumulator {
    blocks: HashMap<usize, InProgressBlock>,
}

impl ContentBlockAccumulator {
    /// Create a new, empty accumulator.
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
        }
    }

    /// Called when a `content_block_start` event arrives for `index`.
    pub fn on_start(&mut self, index: usize, start: ContentBlockStart) {
        let block = match start {
            ContentBlockStart::Text => InProgressBlock::Text {
                text: String::new(),
            },
            ContentBlockStart::ToolUse { id, name } => InProgressBlock::ToolUse {
                id,
                name,
                input_json: String::new(),
            },
            ContentBlockStart::Thinking => InProgressBlock::Thinking {
                thinking: String::new(),
                signature: String::new(),
            },
            ContentBlockStart::ServerToolUse { id, name } => InProgressBlock::ServerToolUse {
                id,
                name,
                input_json: String::new(),
            },
        };
        self.blocks.insert(index, block);
    }

    /// Called for each `content_block_delta` event.
    ///
    /// Mismatched delta types (e.g. `TextDelta` on a `ToolUse` block) are
    /// silently ignored, matching the TypeScript SDK behaviour.
    pub fn on_delta(&mut self, index: usize, delta: ContentDelta) {
        let Some(block) = self.blocks.get_mut(&index) else {
            return;
        };

        match (block, delta) {
            (InProgressBlock::Text { text }, ContentDelta::TextDelta { text: chunk }) => {
                text.push_str(&chunk);
            }
            (
                InProgressBlock::ToolUse { input_json, .. },
                ContentDelta::InputJsonDelta { partial_json },
            ) => {
                input_json.push_str(&partial_json);
            }
            (
                InProgressBlock::ServerToolUse { input_json, .. },
                ContentDelta::InputJsonDelta { partial_json },
            ) => {
                input_json.push_str(&partial_json);
            }
            (
                InProgressBlock::Thinking { thinking, .. },
                ContentDelta::ThinkingDelta { thinking: chunk },
            ) => {
                thinking.push_str(&chunk);
            }
            (
                InProgressBlock::Thinking { signature, .. },
                ContentDelta::SignatureDelta { signature: sig },
            ) => {
                signature.push_str(&sig);
            }
            // All mismatched combinations are silently ignored.
            _ => {}
        }
    }

    /// Called when a `content_block_stop` event arrives for `index`.
    ///
    /// Removes the in-progress block, finalizes it, and returns the completed
    /// [`ContentBlock`].  Returns an error if no block was started for `index`
    /// or if JSON input cannot be parsed.
    pub fn on_stop(&mut self, index: usize) -> Result<ContentBlock> {
        let block = self
            .blocks
            .remove(&index)
            .ok_or_else(|| anyhow!("No in-progress block at index {}", index))?;

        let content_block = match block {
            InProgressBlock::Text { text } => ContentBlock::Text { text },
            InProgressBlock::ToolUse {
                id,
                name,
                input_json,
            } => {
                let input: serde_json::Value = if input_json.is_empty() {
                    serde_json::Value::Object(serde_json::Map::new())
                } else {
                    serde_json::from_str(&input_json)
                        .map_err(|e| anyhow!("Failed to parse tool input JSON: {}", e))?
                };
                ContentBlock::ToolUse { id, name, input }
            }
            InProgressBlock::Thinking {
                thinking,
                signature,
            } => ContentBlock::Thinking {
                thinking,
                signature,
            },
            InProgressBlock::ServerToolUse {
                id,
                name,
                input_json,
            } => {
                let input: serde_json::Value = if input_json.is_empty() {
                    serde_json::Value::Object(serde_json::Map::new())
                } else {
                    serde_json::from_str(&input_json)
                        .map_err(|e| anyhow!("Failed to parse server tool input JSON: {}", e))?
                };
                ContentBlock::ToolUse { id, name, input }
            }
        };

        Ok(content_block)
    }

    /// Returns the current accumulated text for an in-progress `Text` or
    /// `Thinking` block at `index`, useful for streaming display.
    pub fn current_text(&self, index: usize) -> Option<&str> {
        match self.blocks.get(&index)? {
            InProgressBlock::Text { text } => Some(text.as_str()),
            InProgressBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
            _ => None,
        }
    }
}

impl Default for ContentBlockAccumulator {
    fn default() -> Self {
        Self::new()
    }
}
