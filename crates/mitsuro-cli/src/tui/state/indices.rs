//! Block Indices - Tracks per-type indices when iterating messages
//!
//! Eliminates duplication of the 8-index tracking pattern used in:
//! - calculate_message_lines()
//! - hit_test_any_block()

use crate::tui::blocks::BlockType;

/// Tracks block indices by type during message iteration
///
/// When iterating through messages, each block type has its own index
/// that increments independently. This struct encapsulates that pattern.
#[derive(Debug, Default)]
pub struct BlockIndices {
    pub thinking: usize,
    pub pinch: usize,
    pub bash: usize,
    pub terminal: usize,
    pub tool_result: usize,
    pub read: usize,
    pub edit: usize,
    pub write: usize,
    pub web_search: usize,
    pub explore: usize,
    pub build: usize,
}

impl BlockIndices {
    /// Create new zeroed indices
    pub fn new() -> Self {
        Self::default()
    }

    /// Get current index for a block role and increment it
    ///
    /// Returns Some((block_type, index)) if role is a block type,
    /// None if role is a text message (user, assistant, system, tool)
    pub fn get_and_increment(&mut self, role: &str) -> Option<(BlockType, usize)> {
        match role {
            "thinking" => {
                let idx = self.thinking;
                self.thinking += 1;
                Some((BlockType::Thinking, idx))
            }
            "pinch" => {
                let idx = self.pinch;
                self.pinch += 1;
                Some((BlockType::Pinch, idx))
            }
            "bash" => {
                let idx = self.bash;
                self.bash += 1;
                Some((BlockType::Bash, idx))
            }
            "terminal" => {
                let idx = self.terminal;
                self.terminal += 1;
                Some((BlockType::TerminalPane, idx))
            }
            "tool_result" => {
                let idx = self.tool_result;
                self.tool_result += 1;
                Some((BlockType::ToolResult, idx))
            }
            "read" => {
                let idx = self.read;
                self.read += 1;
                Some((BlockType::Read, idx))
            }
            "edit" => {
                let idx = self.edit;
                self.edit += 1;
                Some((BlockType::Edit, idx))
            }
            "write" => {
                let idx = self.write;
                self.write += 1;
                Some((BlockType::Write, idx))
            }
            "web_search" => {
                let idx = self.web_search;
                self.web_search += 1;
                Some((BlockType::WebSearch, idx))
            }
            "explore" => {
                let idx = self.explore;
                self.explore += 1;
                Some((BlockType::Explore, idx))
            }
            "build" => {
                let idx = self.build;
                self.build += 1;
                Some((BlockType::Build, idx))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_and_increment() {
        let mut indices = BlockIndices::new();

        // First thinking block
        assert_eq!(
            indices.get_and_increment("thinking"),
            Some((BlockType::Thinking, 0))
        );
        // Second thinking block
        assert_eq!(
            indices.get_and_increment("thinking"),
            Some((BlockType::Thinking, 1))
        );

        // First bash block (independent counter)
        assert_eq!(
            indices.get_and_increment("bash"),
            Some((BlockType::Bash, 0))
        );

        // Text messages return None
        assert_eq!(indices.get_and_increment("user"), None);
        assert_eq!(indices.get_and_increment("assistant"), None);

        // Third thinking block
        assert_eq!(
            indices.get_and_increment("thinking"),
            Some((BlockType::Thinking, 2))
        );
    }
}
