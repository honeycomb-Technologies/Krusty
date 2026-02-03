//! Message rendering
//!
//! Renders the messages panel with all block types.

mod selection;

/// Block type for position tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockType {
    Thinking,
    Bash,
    Terminal,
    ToolResult,
    Read,
    Edit,
    Write,
    WebSearch,
    Explore,
    Build,
}

/// Unified block position tracking
/// Consolidates the 10 separate Vec allocations into one
#[derive(Debug, Clone)]
struct BlockPosition {
    line_start: usize,
    height: u16,
    block_idx: usize,
    block_type: BlockType,
}

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::tui::app::App;
use crate::tui::blocks::{ClipContext, StreamBlock};
use crate::tui::markdown::{apply_hyperlinks, apply_link_hover_style, RenderedMarkdown};
use crate::tui::state::SelectionArea;
use crate::tui::utils::wrap_line;

use selection::{
    apply_selection_to_line, apply_selection_to_rendered_line, style_user_line_with_file_refs,
};

/// Symbol prefixes for message types (with trailing space)
const USER_SYMBOL: &str = "⤷ "; // Curved down-right arrow
const ASSISTANT_SYMBOL: &str = "⬡ "; // Hollow hexagon
/// Display width of message symbols (symbol char + space)
/// Used to reduce wrap width so prepending symbol doesn't cause overflow
pub const SYMBOL_WIDTH: usize = 2;

/// Simple content hash for cache keying
fn hash_content(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Clear a rectangular area in the buffer before block rendering
/// This prevents character bleed from underlying Paragraph content
fn clear_area(buf: &mut ratatui::buffer::Buffer, area: Rect, bg_color: Color) {
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_bg(bg_color);
                cell.set_fg(Color::Reset);
            }
        }
    }
}

impl App {
    /// Helper: add block position to tracking arrays (used in both passes)
    fn track_block_position(
        block_positions: &mut Vec<BlockPosition>,
        total_lines: &mut usize,
        message_heights: &mut Vec<usize>,
        height: u16,
        block_idx: usize,
        block_type: BlockType,
    ) {
        block_positions.push(BlockPosition {
            line_start: *total_lines,
            height,
            block_idx,
            block_type,
        });
        *total_lines += height as usize + 1; // blank after
        message_heights.push(0);
    }

    /// Render the messages panel
    pub fn render_messages(&mut self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(self.ui.theme.border_color));

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Leave 4 chars padding for scrollbar on right side
        // IMPORTANT: Both wrap_width and content_width must use same padding to prevent overflow
        let scrollbar_gap: u16 = 4;
        let content_width = inner.width.saturating_sub(scrollbar_gap);
        // wrap_width accounts for the symbol prefix that gets prepended to first lines
        // This prevents text from overflowing when symbol is added
        let wrap_width = (content_width as usize).saturating_sub(SYMBOL_WIDTH);

        // Create a content rect that excludes the scrollbar gap
        // This prevents text from rendering into the scrollbar area
        let content_rect = Rect {
            x: inner.x,
            y: inner.y,
            width: content_width,
            height: inner.height,
        };

        // Get selection range if selecting in messages area
        let selection = if self.scroll_system.selection.area == SelectionArea::Messages {
            self.scroll_system.selection.normalized()
        } else {
            None
        };

        // Selection highlight colors from theme
        let sel_bg = self.ui.theme.selection_bg_color;
        let sel_fg = self.ui.theme.selection_fg_color;

        // Clear cache if width changed (resize invalidation)
        self.markdown_cache.check_width(wrap_width);

        // Pre-render all markdown content with link tracking
        // Uses Arc to avoid expensive clones on cache hits
        let mut rendered_markdown: Vec<Option<Arc<RenderedMarkdown>>> =
            Vec::with_capacity(self.chat.messages.len());
        for (role, content) in &self.chat.messages {
            if role == "assistant" {
                let content_hash = hash_content(content);
                let rendered = self.markdown_cache.get_or_render_with_links(
                    content,
                    content_hash,
                    wrap_width,
                    &self.ui.theme,
                );
                rendered_markdown.push(Some(rendered));
            } else {
                rendered_markdown.push(None);
            }
        }

        // Track block positions with single Vec (consolidates 10 allocations into 1)
        let estimated_blocks = self.blocks.total_count();
        let mut block_positions: Vec<BlockPosition> = Vec::with_capacity(estimated_blocks);
        let mut thinking_idx = 0;
        let mut bash_idx = 0;
        let mut terminal_idx = 0;
        let mut tool_result_idx = 0;
        let mut read_idx = 0;
        let mut edit_idx = 0;
        let mut write_idx = 0;
        let mut web_search_idx = 0;
        let mut explore_idx = 0;
        let mut build_idx = 0;
        let mut total_lines: usize = 0;

        // Store message heights from first pass to avoid recalculating in second pass
        let mut message_heights: Vec<usize> = Vec::with_capacity(self.chat.messages.len());

        // First pass: calculate positions (using pre-rendered markdown)
        for (msg_idx, (role, content)) in self.chat.messages.iter().enumerate() {
            if role == "thinking" {
                if let Some(tb) = self.blocks.thinking.get(thinking_idx) {
                    let height = tb.height(content_width, &self.ui.theme);
                    Self::track_block_position(
                        &mut block_positions,
                        &mut total_lines,
                        &mut message_heights,
                        height,
                        thinking_idx,
                        BlockType::Thinking,
                    );
                }
                thinking_idx += 1;
                continue;
            }

            if role == "bash" {
                if let Some(bb) = self.blocks.bash.get(bash_idx) {
                    let height = bb.height(content_width, &self.ui.theme);
                    Self::track_block_position(
                        &mut block_positions,
                        &mut total_lines,
                        &mut message_heights,
                        height,
                        bash_idx,
                        BlockType::Bash,
                    );
                }
                bash_idx += 1;
                continue;
            }

            if role == "terminal" {
                if self.blocks.pinned_terminal != Some(terminal_idx) {
                    if let Some(tp) = self.blocks.terminal.get(terminal_idx) {
                        let height = tp.height(content_width, &self.ui.theme);
                        Self::track_block_position(
                            &mut block_positions,
                            &mut total_lines,
                            &mut message_heights,
                            height,
                            terminal_idx,
                            BlockType::Terminal,
                        );
                    }
                }
                terminal_idx += 1;
                continue;
            }

            if role == "tool_result" {
                if let Some(tr) = self.blocks.tool_result.get(tool_result_idx) {
                    let height = tr.height(content_width, &self.ui.theme);
                    Self::track_block_position(
                        &mut block_positions,
                        &mut total_lines,
                        &mut message_heights,
                        height,
                        tool_result_idx,
                        BlockType::ToolResult,
                    );
                }
                tool_result_idx += 1;
                continue;
            }

            if role == "read" {
                if let Some(rb) = self.blocks.read.get(read_idx) {
                    let height = rb.height(content_width, &self.ui.theme);
                    Self::track_block_position(
                        &mut block_positions,
                        &mut total_lines,
                        &mut message_heights,
                        height,
                        read_idx,
                        BlockType::Read,
                    );
                }
                read_idx += 1;
                continue;
            }

            if role == "edit" {
                if let Some(eb) = self.blocks.edit.get(edit_idx) {
                    let height = eb.height(content_width, &self.ui.theme);
                    Self::track_block_position(
                        &mut block_positions,
                        &mut total_lines,
                        &mut message_heights,
                        height,
                        edit_idx,
                        BlockType::Edit,
                    );
                }
                edit_idx += 1;
                continue;
            }

            if role == "write" {
                if let Some(wb) = self.blocks.write.get(write_idx) {
                    let height = wb.height(content_width, &self.ui.theme);
                    Self::track_block_position(
                        &mut block_positions,
                        &mut total_lines,
                        &mut message_heights,
                        height,
                        write_idx,
                        BlockType::Write,
                    );
                }
                write_idx += 1;
                continue;
            }

            if role == "web_search" {
                if let Some(ws) = self.blocks.web_search.get(web_search_idx) {
                    let height = ws.height(content_width, &self.ui.theme);
                    Self::track_block_position(
                        &mut block_positions,
                        &mut total_lines,
                        &mut message_heights,
                        height,
                        web_search_idx,
                        BlockType::WebSearch,
                    );
                }
                web_search_idx += 1;
                continue;
            }

            if role == "explore" {
                if let Some(eb) = self.blocks.explore.get(explore_idx) {
                    let height = eb.height(content_width, &self.ui.theme);
                    Self::track_block_position(
                        &mut block_positions,
                        &mut total_lines,
                        &mut message_heights,
                        height,
                        explore_idx,
                        BlockType::Explore,
                    );
                }
                explore_idx += 1;
                continue;
            }

            if role == "build" {
                if let Some(bb) = self.blocks.build.get(build_idx) {
                    let height = bb.height(content_width, &self.ui.theme);
                    Self::track_block_position(
                        &mut block_positions,
                        &mut total_lines,
                        &mut message_heights,
                        height,
                        build_idx,
                        BlockType::Build,
                    );
                }
                build_idx += 1;
                continue;
            }

            // Count content lines based on role and store height
            let msg_height = if role == "assistant" {
                // Use pre-rendered markdown lines
                rendered_markdown
                    .get(msg_idx)
                    .and_then(|r| r.as_ref())
                    .map(|r| r.lines.len())
                    .unwrap_or(0)
            } else {
                // Plain text for user/system
                content
                    .lines()
                    .map(|line| {
                        if line.is_empty() {
                            1
                        } else {
                            wrap_line(line, wrap_width).len()
                        }
                    })
                    .sum()
            };
            message_heights.push(msg_height);
            total_lines += msg_height;
            total_lines += 1; // blank line after message
        }

        // Second pass: build lines with placeholders for custom blocks
        // Also track message base line offsets for hyperlink positions
        // OPTIMIZATION: Only build styled content for visible messages
        let scroll_offset = self.scroll_system.scroll.offset;
        let viewport_height = inner.height as usize;
        let visible_start = scroll_offset.saturating_sub(viewport_height); // Buffer above
        let visible_end = scroll_offset + viewport_height * 2; // Buffer below

        let mut lines: Vec<Line> = Vec::with_capacity(total_lines.min(viewport_height * 3));
        let mut line_idx: usize = 0;
        let mut message_line_offsets: Vec<(usize, usize)> = Vec::new(); // (msg_idx, base_line)
        thinking_idx = 0;
        bash_idx = 0;
        terminal_idx = 0;
        tool_result_idx = 0;
        read_idx = 0;
        edit_idx = 0;
        write_idx = 0;
        web_search_idx = 0;
        explore_idx = 0;
        build_idx = 0;

        // Helper to find block position by type and index
        let find_position =
            |positions: &[BlockPosition], block_type: BlockType, idx: usize| -> Option<u16> {
                positions
                    .iter()
                    .find(|p| p.block_type == block_type && p.block_idx == idx)
                    .map(|p| p.height)
            };

        for (msg_idx, (role, content)) in self.chat.messages.iter().enumerate() {
            if role == "thinking" {
                if let Some(height) =
                    find_position(&block_positions, BlockType::Thinking, thinking_idx)
                {
                    for _ in 0..height {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    }
                    lines.push(Line::from("")); // blank
                    line_idx += 1;
                }
                thinking_idx += 1;
                continue;
            }

            if role == "bash" {
                if let Some(height) = find_position(&block_positions, BlockType::Bash, bash_idx) {
                    for _ in 0..height {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    }
                    lines.push(Line::from("")); // blank
                    line_idx += 1;
                }
                bash_idx += 1;
                continue;
            }

            if role == "terminal" {
                // Skip pinned terminal - it's rendered at top
                if self.blocks.pinned_terminal != Some(terminal_idx) {
                    if let Some(height) =
                        find_position(&block_positions, BlockType::Terminal, terminal_idx)
                    {
                        for _ in 0..height {
                            lines.push(Line::from(""));
                            line_idx += 1;
                        }
                        lines.push(Line::from("")); // blank
                        line_idx += 1;
                    }
                }
                terminal_idx += 1;
                continue;
            }

            if role == "tool_result" {
                if let Some(height) =
                    find_position(&block_positions, BlockType::ToolResult, tool_result_idx)
                {
                    for _ in 0..height {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    }
                    lines.push(Line::from("")); // blank
                    line_idx += 1;
                }
                tool_result_idx += 1;
                continue;
            }

            if role == "read" {
                if let Some(height) = find_position(&block_positions, BlockType::Read, read_idx) {
                    for _ in 0..height {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    }
                    lines.push(Line::from("")); // blank
                    line_idx += 1;
                }
                read_idx += 1;
                continue;
            }

            if role == "edit" {
                if let Some(height) = find_position(&block_positions, BlockType::Edit, edit_idx) {
                    for _ in 0..height {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    }
                    lines.push(Line::from("")); // blank
                    line_idx += 1;
                }
                edit_idx += 1;
                continue;
            }

            if role == "write" {
                if let Some(height) = find_position(&block_positions, BlockType::Write, write_idx) {
                    for _ in 0..height {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    }
                    lines.push(Line::from("")); // blank
                    line_idx += 1;
                }
                write_idx += 1;
                continue;
            }

            if role == "web_search" {
                if let Some(height) =
                    find_position(&block_positions, BlockType::WebSearch, web_search_idx)
                {
                    for _ in 0..height {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    }
                    lines.push(Line::from("")); // blank
                    line_idx += 1;
                }
                web_search_idx += 1;
                continue;
            }

            if role == "explore" {
                if let Some(height) =
                    find_position(&block_positions, BlockType::Explore, explore_idx)
                {
                    for _ in 0..height {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    }
                    lines.push(Line::from("")); // blank
                    line_idx += 1;
                }
                explore_idx += 1;
                continue;
            }

            if role == "build" {
                if let Some(height) = find_position(&block_positions, BlockType::Build, build_idx) {
                    for _ in 0..height {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    }
                    lines.push(Line::from("")); // blank
                    line_idx += 1;
                }
                build_idx += 1;
                continue;
            }

            // Get cached height for this message (avoids recalculating wrap_line)
            let msg_height = message_heights.get(msg_idx).copied().unwrap_or(0);
            let msg_end = line_idx + msg_height;

            // OPTIMIZATION: Check if this message is visible
            if msg_end < visible_start || line_idx > visible_end {
                // Off-screen: push empty placeholders (fast path)
                for _ in 0..msg_height {
                    lines.push(Line::from(""));
                    line_idx += 1;
                }
            } else if role == "assistant" {
                // On-screen assistant: render markdown
                if let Some(Some(rendered)) = rendered_markdown.get(msg_idx) {
                    message_line_offsets.push((msg_idx, line_idx));

                    for (md_line_idx, md_line) in rendered.lines.iter().enumerate() {
                        // Prepend symbol to first line of assistant messages
                        let line_with_symbol = if md_line_idx == 0 {
                            let symbol = Span::styled(
                                ASSISTANT_SYMBOL,
                                Style::default().fg(self.ui.theme.accent_color),
                            );
                            let mut spans = vec![symbol];
                            spans.extend(md_line.spans.clone());
                            Line::from(spans)
                        } else {
                            md_line.clone()
                        };

                        let final_line = if selection.is_some() {
                            apply_selection_to_rendered_line(
                                line_with_symbol,
                                line_idx,
                                selection,
                                sel_bg,
                                sel_fg,
                            )
                        } else {
                            line_with_symbol
                        };

                        lines.push(final_line);
                        line_idx += 1;
                    }
                }
            } else {
                // On-screen user/system: render plain text
                let content_color = match role.as_str() {
                    "user" => self.ui.theme.user_msg_color,
                    "system" => self.ui.theme.system_msg_color,
                    _ => self.ui.theme.text_color,
                };

                let hovered_file_ref = self.scroll_system.hover.message_file_ref.as_ref();
                let mut is_first_line_of_msg = true;

                for line in content.lines() {
                    if line.is_empty() {
                        lines.push(Line::from(""));
                        line_idx += 1;
                    } else {
                        for (wrap_idx, wrapped) in
                            wrap_line(line, wrap_width).into_iter().enumerate()
                        {
                            let content_line = if role == "user" {
                                style_user_line_with_file_refs(
                                    &wrapped,
                                    line_idx,
                                    selection,
                                    Style::default().fg(content_color),
                                    self.ui.theme.link_color,
                                    sel_bg,
                                    sel_fg,
                                    msg_idx,
                                    hovered_file_ref,
                                )
                            } else {
                                apply_selection_to_line(
                                    wrapped,
                                    line_idx,
                                    selection,
                                    Style::default().fg(content_color),
                                    sel_bg,
                                    sel_fg,
                                )
                            };

                            // Prepend symbol to first line of user messages
                            let final_line =
                                if role == "user" && is_first_line_of_msg && wrap_idx == 0 {
                                    is_first_line_of_msg = false;
                                    let symbol = Span::styled(
                                        USER_SYMBOL,
                                        Style::default().fg(self.ui.theme.accent_color),
                                    );
                                    let mut spans = vec![symbol];
                                    spans.extend(content_line.spans);
                                    Line::from(spans)
                                } else {
                                    content_line
                                };

                            lines.push(final_line);
                            line_idx += 1;
                        }
                    }
                }
            }
            lines.push(Line::from("")); // Blank between messages
            line_idx += 1;
        }

        // Clear the entire messages viewport (including scrollbar gap) before rendering
        // This is critical: ratatui widgets don't clear cells they don't touch
        clear_area(f.buffer_mut(), inner, self.ui.theme.bg_color);

        // Render text content into content_rect (NOT inner) to prevent overflow into scrollbar gap
        // Use a unified effective_scroll for ALL rendering operations to prevent drift
        // Clamp to u16::MAX since Paragraph::scroll uses u16 (supports ~65k lines)
        let effective_scroll = self.scroll_system.scroll.offset.min(u16::MAX as usize);
        let effective_scroll_u16 = effective_scroll as u16;
        f.render_widget(
            Paragraph::new(lines).scroll((effective_scroll_u16, 0)),
            content_rect, // Render only into content area, not scrollbar gap
        );

        // Clear the scrollbar gap after Paragraph to catch any overflow/bleed
        // This is a safety net against any content that might escape content_rect bounds
        let scrollbar_clear_rect = Rect {
            x: inner.x + content_width,
            y: inner.y,
            width: scrollbar_gap,
            height: inner.height,
        };
        clear_area(f.buffer_mut(), scrollbar_clear_rect, self.ui.theme.bg_color);

        // Apply OSC 8 hyperlinks to the buffer after Paragraph rendering
        // This wraps each link cell's symbol with escape sequences
        // Use content_rect (not inner) to match the Paragraph render area
        for (msg_idx, base_line) in &message_line_offsets {
            if let Some(Some(rendered)) = rendered_markdown.get(*msg_idx) {
                if !rendered.links.is_empty() {
                    apply_hyperlinks(
                        f.buffer_mut(),
                        content_rect, // Match Paragraph render area
                        &rendered.links,
                        effective_scroll,
                        *base_line,
                    );

                    // Apply hover styling if this message contains the hovered link
                    if let Some(hovered) = &self.scroll_system.hover.message_link {
                        if hovered.msg_idx == *msg_idx {
                            apply_link_hover_style(
                                f.buffer_mut(),
                                content_rect, // Match Paragraph render area
                                &rendered.links,
                                Some(hovered),
                                effective_scroll,
                                *base_line,
                                self.ui.theme.link_color,
                            );
                        }
                    }
                }
            }
        }

        // Overlay each block at its position
        // Use effective_scroll for consistent coordinate math with Paragraph rendering
        let scroll = effective_scroll;
        let inner_height = inner.height as usize;

        self.render_block_overlays(
            f,
            &inner,
            content_width,
            scroll,
            inner_height,
            &block_positions,
        );
        // Note: Block overlays already clear with inner.width which includes scrollbar gap,
        // so no additional scrollbar clear needed here.

        // Resize terminal PTYs to match render width (debounced)
        // Note: tick() is called in the event loop before render, not here
        for tp in &mut self.blocks.terminal {
            tp.resize_to_width(content_width);
        }
    }

    /// Render block overlays at their calculated positions
    fn render_block_overlays(
        &self,
        f: &mut Frame,
        inner: &Rect,
        content_width: u16,
        scroll: usize,
        inner_height: usize,
        block_positions: &[BlockPosition],
    ) {
        // Render all blocks in a single pass through the unified position list
        for pos in block_positions {
            let block_y = pos.line_start;
            let block_height = pos.height as usize;

            // Check if visible
            if block_y + block_height <= scroll || block_y >= scroll + inner_height {
                continue;
            }

            let screen_y = if block_y >= scroll {
                inner.y + (block_y - scroll).min(u16::MAX as usize) as u16
            } else {
                inner.y
            };

            // Compute clip values in usize first, then clamp to height before converting to u16
            // This prevents truncation bugs when scroll offsets are large
            let clip_top_usize = scroll.saturating_sub(block_y);
            let clip_top = (clip_top_usize.min(pos.height as usize)) as u16;
            let available_height = inner.height.saturating_sub(screen_y - inner.y);
            let visible_height = pos.height.saturating_sub(clip_top).min(available_height);
            let clip_bottom = pos.height.saturating_sub(clip_top + visible_height);

            if visible_height == 0 {
                continue;
            }

            let block_area = Rect {
                x: inner.x,
                y: screen_y,
                width: content_width,
                height: visible_height,
            };

            let clip = if clip_top > 0 || clip_bottom > 0 {
                Some(ClipContext {
                    clip_top,
                    clip_bottom,
                })
            } else {
                None
            };

            // Clear full inner.width to remove Paragraph bleed in scrollbar gap
            let clear_rect = Rect {
                x: inner.x,
                y: screen_y,
                width: inner.width,
                height: visible_height,
            };
            clear_area(f.buffer_mut(), clear_rect, self.ui.theme.bg_color);

            // Render the appropriate block type
            match pos.block_type {
                BlockType::Thinking => {
                    if let Some(tb) = self.blocks.thinking.get(pos.block_idx) {
                        tb.render(block_area, f.buffer_mut(), &self.ui.theme, false, clip);
                    }
                }
                BlockType::Bash => {
                    if let Some(bb) = self.blocks.bash.get(pos.block_idx) {
                        bb.render(block_area, f.buffer_mut(), &self.ui.theme, false, clip);
                    }
                }
                BlockType::Terminal => {
                    if let Some(tp) = self.blocks.terminal.get(pos.block_idx) {
                        let is_focused = self.blocks.focused_terminal == Some(pos.block_idx);
                        tp.render(block_area, f.buffer_mut(), &self.ui.theme, is_focused, clip);
                    }
                }
                BlockType::ToolResult => {
                    if let Some(tr) = self.blocks.tool_result.get(pos.block_idx) {
                        tr.render(block_area, f.buffer_mut(), &self.ui.theme, false, clip);
                    }
                }
                BlockType::Read => {
                    if let Some(rb) = self.blocks.read.get(pos.block_idx) {
                        rb.render(block_area, f.buffer_mut(), &self.ui.theme, false, clip);
                    }
                }
                BlockType::Edit => {
                    if let Some(eb) = self.blocks.edit.get(pos.block_idx) {
                        eb.render(block_area, f.buffer_mut(), &self.ui.theme, false, clip);
                    }
                }
                BlockType::Write => {
                    if let Some(wb) = self.blocks.write.get(pos.block_idx) {
                        wb.render(block_area, f.buffer_mut(), &self.ui.theme, false, clip);
                    }
                }
                BlockType::WebSearch => {
                    if let Some(ws) = self.blocks.web_search.get(pos.block_idx) {
                        ws.render(block_area, f.buffer_mut(), &self.ui.theme, false, clip);
                    }
                }
                BlockType::Explore => {
                    if let Some(eb) = self.blocks.explore.get(pos.block_idx) {
                        eb.render(block_area, f.buffer_mut(), &self.ui.theme, false, clip);
                    }
                }
                BlockType::Build => {
                    if let Some(bb) = self.blocks.build.get(pos.block_idx) {
                        bb.render(block_area, f.buffer_mut(), &self.ui.theme, false, clip);
                    }
                }
            }
        }
    }
}
