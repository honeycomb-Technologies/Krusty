use anyhow::Result;

use super::AnthropicParser;
use crate::ai::sse::{ServerToolAccumulator, ThinkingAccumulator, ToolCallAccumulator};

impl AnthropicParser {
    /// Lock tool accumulators with proper error handling
    pub(super) fn lock_tool_accumulators(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, std::collections::HashMap<usize, ToolCallAccumulator>>>
    {
        self.tool_accumulators
            .lock()
            .map_err(|e| anyhow::anyhow!("Tool accumulators lock poisoned: {}", e))
    }

    /// Lock thinking accumulators with proper error handling
    pub(super) fn lock_thinking_accumulators(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, std::collections::HashMap<usize, ThinkingAccumulator>>>
    {
        self.thinking_accumulators
            .lock()
            .map_err(|e| anyhow::anyhow!("Thinking accumulators lock poisoned: {}", e))
    }

    /// Lock server tool accumulators with proper error handling
    pub(super) fn lock_server_tool_accumulators(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, std::collections::HashMap<usize, ServerToolAccumulator>>>
    {
        self.server_tool_accumulators
            .lock()
            .map_err(|e| anyhow::anyhow!("Server tool accumulators lock poisoned: {}", e))
    }
}
