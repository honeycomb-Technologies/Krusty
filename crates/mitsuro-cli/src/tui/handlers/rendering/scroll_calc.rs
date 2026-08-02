//! Scroll calculation utilities
//!
//! Calculates message line counts for scrollbar positioning.
//! Must match render_messages() logic exactly for consistent scroll behavior.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::tui::app::App;
use crate::tui::utils::count_wrapped_lines;

use super::display_list::DisplayList;
use super::messages::SYMBOL_WIDTH;

impl App {
    /// Calculate total lines in messages for scrollbar
    /// Uses the same wrapping logic as render_messages for accurate counting
    /// NOTE: Takes &mut self to populate markdown cache for consistency with render
    pub fn calculate_message_lines(&mut self, width: u16) -> usize {
        // Account for borders (2) + scrollbar padding (4) = 6 total
        // MUST match render_messages() which uses: inner.width.saturating_sub(4)
        // where inner.width = area.width - 2 (from block.inner), so total = width - 6
        let inner_width = width.saturating_sub(6) as usize;
        let content_width = width.saturating_sub(6); // Must match inner_width for blocks
                                                     // wrap_width accounts for symbol prefix (same as render_messages)
        let wrap_width = inner_width.saturating_sub(SYMBOL_WIDTH);

        // Pre-render markdown to cache (same as render_messages) to ensure consistent line counts
        self.ui.markdown_cache.check_width(wrap_width);

        let mut message_heights = Vec::with_capacity(self.runtime.chat.messages.len());
        for (role, content) in &self.runtime.chat.messages {
            let height = if role == "assistant" {
                // Render markdown to cache and get line count (matches render_messages exactly)
                let mut hasher = DefaultHasher::new();
                content.hash(&mut hasher);
                let content_hash = hasher.finish();
                let rendered = self.ui.markdown_cache.get_or_render_with_links(
                    content,
                    content_hash,
                    wrap_width,
                    &self.ui.theme,
                );
                rendered.lines.len()
            } else {
                // User/system messages - plain text with wrapping
                // Must match render_messages exactly: wrap each line, then blank after
                content
                    .lines()
                    .map(|line| {
                        if line.is_empty() {
                            1
                        } else {
                            count_wrapped_lines(line, wrap_width)
                        }
                    })
                    .sum()
            };
            message_heights.push(height);
        }

        DisplayList::build(
            &self.runtime.chat.messages,
            |message_index, _, _| message_heights[message_index],
            |block_type, index| self.stream_block_height(block_type, index, content_width),
        )
        .total_lines
    }
}
