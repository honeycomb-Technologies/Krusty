//! Hit testing handlers
//!
//! Coordinates-to-element conversion for mouse event handling.
//! Extracted from app.rs to reduce its size.

use crate::tui::app::App;
use crate::tui::blocks::{BlockHitResult, BlockType, ClipContext, StreamBlock};
use crate::tui::state::BlockIndices;
use crate::tui::utils::count_wrapped_lines;

impl App {
    /// Convert screen coordinates to messages text position (line, column)
    /// Returns None if click is outside content
    pub fn hit_test_messages(&self, screen_x: u16, screen_y: u16) -> Option<(usize, usize)> {
        let area = self.ui.scroll_system.layout.messages_area?;

        // Check if within messages area (accounting for border)
        let inner_x = area.x + 1;
        let inner_y = area.y + 1;
        let inner_width = area.width.saturating_sub(2);
        let inner_height = area.height.saturating_sub(2);

        if screen_x < inner_x
            || screen_x >= inner_x + inner_width
            || screen_y < inner_y
            || screen_y >= inner_y + inner_height
        {
            return None;
        }

        // Calculate relative position within content area
        let rel_x = (screen_x - inner_x) as usize;
        let rel_y = (screen_y - inner_y) as usize;

        // Add scroll offset to get actual line index
        let line_index = rel_y + self.ui.scroll_system.scroll.offset;

        Some((line_index, rel_x))
    }

    /// Get markdown line count from cache (O(1) cache hit) or fast estimate on miss
    /// Used by hit_test functions - prefers cache hit from calculate_message_lines
    pub fn get_markdown_line_count(&self, content: &str, wrap_width: usize) -> usize {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = hasher.finish();

        // First check the new cache (populated by calculate_message_lines via get_or_render_with_links)
        if let Some(cached) = self
            .ui
            .markdown_cache
            .get_rendered(content_hash, wrap_width)
        {
            return cached.lines.len();
        }

        // Fall back to legacy cache
        if let Some(cached) = self.ui.markdown_cache.get(content_hash, wrap_width) {
            return cached.len();
        }

        // Cache miss - use fast estimation (should rarely happen after calculate_message_lines runs)
        let wrap_width = wrap_width.max(1);
        let lines: usize = content
            .lines()
            .map(|line| {
                let char_count = line.chars().count();
                if char_count == 0 {
                    1
                } else {
                    char_count.div_ceil(wrap_width)
                }
            })
            .sum();
        if lines == 0 && !content.is_empty() {
            1
        } else {
            lines
        }
    }

    /// Unified hit test for any block type - returns first block hit with type info
    ///
    /// This consolidates hit testing into a single message iteration instead of 5 separate ones.
    pub fn hit_test_any_block(&self, screen_x: u16, screen_y: u16) -> Option<BlockHitResult> {
        let area = self.ui.scroll_system.layout.messages_area?;
        let inner_x = area.x + 1;
        let inner_y = area.y + 1;
        let inner_width = area.width.saturating_sub(2);
        let inner_height = area.height.saturating_sub(2);

        if screen_x < inner_x
            || screen_x >= inner_x + inner_width
            || screen_y < inner_y
            || screen_y >= inner_y + inner_height
        {
            return None;
        }

        let scroll = self.ui.scroll_system.scroll.offset as u16;
        // MUST match render_messages():
        // - content_width = inner.width - 4 (scrollbar gap)
        // - wrap_width = content_width - 2 (SYMBOL_WIDTH for message prefixes)
        let content_width = inner_width.saturating_sub(4);
        let wrap_width = content_width.saturating_sub(2) as usize;

        let mut indices = BlockIndices::new();
        let mut current_line: u16 = 0;

        // Helper closure to check if block at current_line is hit
        let check_hit = |block_y: u16,
                         height: u16,
                         idx: usize,
                         block_type: BlockType|
         -> Option<BlockHitResult> {
            if block_y + height > scroll && block_y < scroll + inner_height {
                let screen_y_start = if block_y >= scroll {
                    inner_y + (block_y - scroll)
                } else {
                    inner_y
                };
                let clip_top = scroll.saturating_sub(block_y);
                let available_height = inner_height.saturating_sub(screen_y_start - inner_y);
                let visible_height = (height - clip_top).min(available_height);
                let clip_bottom = height.saturating_sub(clip_top + visible_height);

                if screen_y >= screen_y_start && screen_y < screen_y_start + visible_height {
                    return Some(BlockHitResult {
                        block_type,
                        index: idx,
                        area: ratatui::layout::Rect {
                            x: inner_x,
                            y: screen_y_start,
                            width: content_width,
                            height: visible_height,
                        },
                        clip: if clip_top > 0 || clip_bottom > 0 {
                            Some(ClipContext {
                                clip_top,
                                clip_bottom,
                            })
                        } else {
                            None
                        },
                    });
                }
            }
            None
        };

        for (role, content) in &self.runtime.chat.messages {
            if let Some((block_type, idx)) = indices.get_and_increment(role) {
                // Get block height based on type
                let height = match block_type {
                    BlockType::Thinking => self
                        .runtime
                        .blocks
                        .thinking
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Bash => self
                        .runtime
                        .blocks
                        .bash
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::TerminalPane => {
                        // Skip pinned terminal - handled separately at top
                        if self.runtime.blocks.pinned_terminal == Some(idx) {
                            None
                        } else {
                            self.runtime
                                .blocks
                                .terminal
                                .get(idx)
                                .map(|b| b.height(content_width, &self.ui.theme))
                        }
                    }
                    BlockType::ToolResult => self
                        .runtime
                        .blocks
                        .tool_result
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Read => self
                        .runtime
                        .blocks
                        .read
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Edit => self
                        .runtime
                        .blocks
                        .edit
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Write => self
                        .runtime
                        .blocks
                        .write
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::WebSearch => self
                        .runtime
                        .blocks
                        .web_search
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Explore => self
                        .runtime
                        .blocks
                        .explore
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                    BlockType::Build => self
                        .runtime
                        .blocks
                        .build
                        .get(idx)
                        .map(|b| b.height(content_width, &self.ui.theme)),
                };

                if let Some(height) = height {
                    if let Some(result) = check_hit(current_line, height, idx, block_type) {
                        return Some(result);
                    }
                    current_line += height + 1;
                }
            } else if role == "assistant" {
                // Assistant messages are markdown-rendered
                let line_count = self.get_markdown_line_count(content, wrap_width);
                current_line += line_count as u16 + 1;
            } else {
                // User/system messages - plain text with wrapping
                for line in content.lines() {
                    if line.is_empty() {
                        current_line += 1;
                    } else {
                        current_line += count_wrapped_lines(line, wrap_width) as u16;
                    }
                }
                current_line += 1; // blank
            }
        }

        None
    }

    /// Convert screen coordinates to input text position (line, column)
    /// Returns None if click is outside content
    pub fn hit_test_input(&self, screen_x: u16, screen_y: u16) -> Option<(usize, usize)> {
        let area = self.ui.scroll_system.layout.input_area?;

        // Check if within input area (accounting for border)
        let inner_x = area.x + 1;
        let inner_y = area.y + 1;
        let inner_width = area.width.saturating_sub(2);
        let inner_height = area.height.saturating_sub(2);

        if screen_x < inner_x
            || screen_x >= inner_x + inner_width
            || screen_y < inner_y
            || screen_y >= inner_y + inner_height
        {
            return None;
        }

        // Calculate relative position within content area
        let rel_x = (screen_x - inner_x) as usize;
        let rel_y = (screen_y - inner_y) as usize;

        // Add viewport offset to get actual line index
        let line_index = rel_y + self.ui.input.get_viewport_offset();

        Some((line_index, rel_x))
    }
}
