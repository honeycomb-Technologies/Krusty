//! Scroll calculation utilities
//!
//! Calculates message line counts for scrollbar positioning.
//! Must match render_messages() logic exactly for consistent scroll behavior.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::tui::app::App;
use crate::tui::blocks::{BlockType, StreamBlock};
use crate::tui::state::BlockIndices;
use crate::tui::utils::count_wrapped_lines;

use super::messages::SYMBOL_WIDTH;

impl App {
    /// Calculate total lines in messages for scrollbar
    /// Uses the same wrapping logic as render_messages for accurate counting
    /// NOTE: Takes &mut self to populate markdown cache for consistency with render
    pub fn calculate_message_lines(&mut self, width: u16) -> usize {
        let mut total = 0;
        let mut indices = BlockIndices::new();
        // Account for borders (2) + scrollbar padding (4) = 6 total
        // MUST match render_messages() which uses: inner.width.saturating_sub(4)
        // where inner.width = area.width - 2 (from block.inner), so total = width - 6
        let inner_width = width.saturating_sub(6) as usize;
        let content_width = width.saturating_sub(6); // Must match inner_width for blocks
                                                     // wrap_width accounts for symbol prefix (same as render_messages)
        let wrap_width = inner_width.saturating_sub(SYMBOL_WIDTH);

        // Pre-render markdown to cache (same as render_messages) to ensure consistent line counts
        self.markdown_cache.check_width(wrap_width);

        for (role, content) in &self.chat.messages {
            if let Some((block_type, idx)) = indices.get_and_increment(role) {
                // Handle block types
                let height = match block_type {
                    BlockType::Thinking => self
                        .blocks
                        .thinking
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Bash => self
                        .blocks
                        .bash
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::TerminalPane => {
                        // Skip pinned terminal - it's rendered at top
                        if self.blocks.pinned_terminal == Some(idx) {
                            None
                        } else {
                            self.blocks
                                .terminal
                                .get(idx)
                                .map(|b| b.height(content_width, &self.ui.theme))
                        }
                    }
                    BlockType::ToolResult => self
                        .blocks
                        .tool_result
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Read => self
                        .blocks
                        .read
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Edit => self
                        .blocks
                        .edit
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Write => self
                        .blocks
                        .write
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::WebSearch => self
                        .blocks
                        .web_search
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Explore => self
                        .blocks
                        .explore
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Build => self
                        .blocks
                        .build
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                };
                if let Some(h) = height {
                    total += h as usize + 1; // +1 for blank after
                }
            } else if role == "assistant" {
                // Render markdown to cache and get line count (matches render_messages exactly)
                let mut hasher = DefaultHasher::new();
                content.hash(&mut hasher);
                let content_hash = hasher.finish();
                let rendered = self.markdown_cache.get_or_render_with_links(
                    content,
                    content_hash,
                    wrap_width,
                    &self.ui.theme,
                );
                total += rendered.lines.len() + 1; // +1 for blank after
            } else {
                // User/system messages - plain text with wrapping
                // Must match render_messages exactly: wrap each line, then blank after
                for line in content.lines() {
                    if line.is_empty() {
                        total += 1;
                    } else {
                        total += count_wrapped_lines(line, wrap_width);
                    }
                }
                total += 1; // Blank line after
            }
        }
        total
    }
}
