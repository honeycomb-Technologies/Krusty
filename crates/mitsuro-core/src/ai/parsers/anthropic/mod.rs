//! Anthropic-specific SSE parser

mod content_blocks;
mod messages;
mod state;
mod web;

use anyhow::Result;
use serde_json::Value;

use crate::ai::sse::{
    ServerToolAccumulator, SseEvent, SseParser, ThinkingAccumulator, ToolCallAccumulator,
};

/// Anthropic-specific SSE parser
pub struct AnthropicParser {
    /// Track tool calls by content block index
    tool_accumulators: std::sync::Mutex<std::collections::HashMap<usize, ToolCallAccumulator>>,
    /// Track thinking blocks by content block index
    thinking_accumulators: std::sync::Mutex<std::collections::HashMap<usize, ThinkingAccumulator>>,
    /// Track server tool uses by content block index
    server_tool_accumulators:
        std::sync::Mutex<std::collections::HashMap<usize, ServerToolAccumulator>>,
}

impl AnthropicParser {
    pub fn new() -> Self {
        Self {
            tool_accumulators: std::sync::Mutex::new(std::collections::HashMap::new()),
            thinking_accumulators: std::sync::Mutex::new(std::collections::HashMap::new()),
            server_tool_accumulators: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl SseParser for AnthropicParser {
    async fn parse_event(&self, json: &Value) -> Result<SseEvent> {
        let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            "content_block_start" => self.parse_content_block_start(json),
            "content_block_delta" => self.parse_content_block_delta(json),
            "content_block_stop" => self.parse_content_block_stop(json),
            "message_delta" => Ok(self.parse_message_delta(json)),
            "message_start" => Ok(self.parse_message_start(json)),
            "message_stop" => Ok(self.parse_message_stop()),
            "error" => self.parse_error_event(json),
            _ => Ok(SseEvent::Skip),
        }
    }
}

impl Default for AnthropicParser {
    fn default() -> Self {
        Self::new()
    }
}
