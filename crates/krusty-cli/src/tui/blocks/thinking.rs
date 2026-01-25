//! Thinking block - collapsible display of Claude's thinking process
//!
//! Default: Collapsed with spinner, click to expand and see streaming content.
//! Expanded blocks have max height and become scrollable.

use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{BlockEvent, ClipContext, EventResult, StreamBlock, WidthScrollable};
use crate::tui::components::scrollbars::render_scrollbar;
use crate::tui::themes::Theme;
use crate::tui::utils::wrap_text;

/// Spinner frames for streaming state
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);

/// Max visible content lines when expanded (before scrolling kicks in)
const MAX_VISIBLE_LINES: u16 = 15;

/// A collapsible thinking block
///
/// Default state is COLLAPSED - shows a compact "Thinking..." with spinner.
/// Click or press space to expand and see the streaming content.
/// When expanded, caps at MAX_VISIBLE_LINES and becomes scrollable.
pub struct ThinkingBlock {
    /// The thinking content (streams in over time)
    content: String,
    /// Signature from API (used as stable block ID for state persistence)
    signature: Option<String>,
    /// Whether the block is collapsed (default: true)
    collapsed: bool,
    /// Whether content is still streaming
    streaming: bool,
    /// Last spinner update time
    last_spinner_update: Instant,
    /// Current spinner frame index
    spinner_idx: usize,
    /// Scroll offset (line index) for expanded view
    scroll_offset: u16,
    /// Cached wrapped lines (updated on render)
    cached_lines: Vec<String>,
    /// Width used for caching
    cached_width: u16,
    /// Cached height for quick access without mutable borrow
    cached_height: u16,
}

impl ThinkingBlock {
    /// Create a new thinking block (collapsed by default, streaming)
    pub fn new() -> Self {
        Self {
            content: String::new(),
            signature: None,
            collapsed: true,
            streaming: true,
            last_spinner_update: Instant::now(),
            spinner_idx: 0,
            scroll_offset: 0,
            cached_lines: Vec::new(),
            cached_width: 0,
            cached_height: 3, // minimum height
        }
    }

    /// Append streaming content
    pub fn append(&mut self, text: &str) {
        self.content.push_str(text);
        // Invalidate cache
        self.cached_width = 0;
        self.cached_height = 0;
    }

    /// Set the signature (used as stable block ID)
    pub fn set_signature(&mut self, signature: String) {
        self.signature = Some(signature);
    }

    /// Mark streaming as complete
    pub fn complete(&mut self) {
        self.streaming = false;
    }

    /// Check if collapsed
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Set collapsed state directly (for session restoration)
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
        if collapsed {
            self.scroll_offset = 0;
        }
    }

    /// Toggle collapsed/expanded state
    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
        if self.collapsed {
            // Reset scroll when collapsing
            self.scroll_offset = 0;
        }
    }

    /// Get current spinner frame (uses cached index updated by tick())
    fn spinner_frame(&self) -> char {
        SPINNER_FRAMES[self.spinner_idx % SPINNER_FRAMES.len()]
    }

    /// Update spinner animation (called by tick())
    fn update_spinner(&mut self) {
        if self.streaming && self.last_spinner_update.elapsed() >= SPINNER_INTERVAL {
            self.spinner_idx = (self.spinner_idx + 1) % SPINNER_FRAMES.len();
            self.last_spinner_update = Instant::now();
        }
    }

    /// Total content lines
    fn total_lines(&mut self, width: u16) -> u16 {
        WidthScrollable::get_lines(self, width).len() as u16
    }

    /// Visible lines (capped at MAX_VISIBLE_LINES)
    fn visible_lines(&mut self, width: u16) -> u16 {
        self.total_lines(width).min(MAX_VISIBLE_LINES)
    }

    /// Get scroll info for drag handling
    /// Note: delegates to WidthScrollable::get_width_scroll_info
    pub fn get_scroll_info(&mut self, width: u16) -> (u16, u16, u16) {
        self.get_width_scroll_info(width)
    }

    /// Check if block has enough content to need a scrollbar
    /// Note: delegates to WidthScrollable::needs_scrollbar
    pub fn has_scrollbar(&mut self, width: u16) -> bool {
        WidthScrollable::needs_scrollbar(self, width)
    }

    /// Calculate actual rendered box width for hit testing
    pub fn box_width(&mut self, available_width: u16) -> u16 {
        let lines = self.get_lines(available_width);
        let longest_line = lines
            .iter()
            .map(|l| UnicodeWidthStr::width(l.as_str()))
            .max()
            .unwrap_or(20);
        let box_inner_width = longest_line.max(20);
        ((box_inner_width + 4) as u16).min(available_width)
    }
}

impl Default for ThinkingBlock {
    fn default() -> Self {
        Self::new()
    }
}

impl WidthScrollable for ThinkingBlock {
    fn get_lines(&mut self, width: u16) -> &[String] {
        let content_width = 76usize.min(width.saturating_sub(4) as usize);
        if self.cached_width != width || self.cached_lines.is_empty() {
            self.cached_lines = wrap_text(&self.content, content_width);
            self.cached_width = width;
            let content_lines = (self.cached_lines.len() as u16).min(MAX_VISIBLE_LINES);
            self.cached_height = (content_lines + 2).max(3);
        }
        &self.cached_lines
    }

    fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }

    fn set_scroll_offset(&mut self, offset: u16) {
        let max = self
            .total_lines(self.cached_width)
            .saturating_sub(MAX_VISIBLE_LINES);
        self.scroll_offset = offset.min(max);
    }

    fn max_visible_lines(&self) -> u16 {
        MAX_VISIBLE_LINES
    }
}

impl StreamBlock for ThinkingBlock {
    fn height(&self, width: u16, _theme: &Theme) -> u16 {
        if self.collapsed {
            1
        } else if self.cached_width == width && self.cached_height > 0 {
            // Use cached height if available and width matches
            self.cached_height
        } else {
            // Fallback: compute without caching (rare case)
            let content_width = 76usize.min(width.saturating_sub(4) as usize);
            let lines = wrap_text(&self.content, content_width);
            let content_lines = (lines.len() as u16).min(MAX_VISIBLE_LINES);
            (content_lines + 2).max(3)
        }
    }

    fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        focused: bool,
        clip: Option<ClipContext>,
    ) {
        if area.height == 0 || area.width < 10 {
            return;
        }

        if self.collapsed {
            self.render_collapsed(area, buf, theme, focused);
        } else {
            let scroll_offset = self.scroll_offset;
            self.render_expanded_clipped(area, buf, theme, scroll_offset, clip);
        }
    }

    fn handle_event(
        &mut self,
        event: &Event,
        area: Rect,
        clip: Option<ClipContext>,
    ) -> EventResult {
        let clip_top = clip.map_or(0, |c| c.clip_top);

        match event {
            // Scroll wheel events
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column,
                row,
                ..
            }) => {
                // Use actual rendered box width, not full area width
                let actual_width = self.box_width(area.width);
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + actual_width;

                if in_area && !self.collapsed {
                    self.scroll_down(area.width);
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column,
                row,
                ..
            }) => {
                // Use actual rendered box width, not full area width
                let actual_width = self.box_width(area.width);
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + actual_width;

                if in_area && !self.collapsed {
                    self.scroll_up();
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            // Click on block - check if scrollbar or toggle area
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                ..
            }) => {
                // Use actual rendered box width for hit testing
                let actual_width = self.box_width(area.width);
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + actual_width;

                if in_area {
                    // Translate to block-internal Y coordinate
                    let internal_y = (*row - area.y) + clip_top;

                    // Check if click is on scrollbar area (when expanded and has scrollbar)
                    if !self.collapsed && self.needs_scrollbar(area.width) {
                        let scrollbar_x = area.x + actual_width - 2;

                        // Check scrollbar click using internal coordinates
                        if *column == scrollbar_x && internal_y > 0 {
                            let total = self.total_lines(area.width) as usize;
                            let visible = self.visible_lines(area.width) as usize;
                            let max_scroll = total.saturating_sub(visible);
                            let track_height = visible;
                            let click_y = (internal_y - 1) as usize; // -1 for header
                            let new_offset = if track_height > 0 {
                                (click_y * max_scroll) / track_height
                            } else {
                                0
                            };
                            self.scroll_offset = new_offset.min(max_scroll) as u16;
                            return EventResult::Consumed;
                        }
                    }

                    // Toggle behavior:
                    // - Collapsed: any click expands
                    // - Expanded: only click on header (internal_y == 0) collapses
                    if self.collapsed {
                        self.toggle();
                        return EventResult::Action(BlockEvent::Expanded);
                    } else if internal_y == 0 {
                        // Only toggle when clicking on header line
                        self.toggle();
                        return EventResult::Action(BlockEvent::Collapsed);
                    }
                    // Click in content area - consume but don't toggle
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            // Enter/Space toggles when focused
            Event::Key(KeyEvent {
                code: KeyCode::Enter | KeyCode::Char(' '),
                ..
            }) => {
                self.toggle();
                if self.collapsed {
                    EventResult::Action(BlockEvent::Collapsed)
                } else {
                    EventResult::Action(BlockEvent::Expanded)
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn get_text_content(&self) -> Option<String> {
        if self.collapsed {
            return Some("Thinking".to_string());
        }
        match (self.content.is_empty(), self.streaming) {
            (true, true) => Some("Thinking...".to_string()),
            (true, false) => None,
            (false, _) => Some(self.content.clone()),
        }
    }

    fn tick(&mut self) -> bool {
        if self.streaming {
            self.update_spinner();
            true // Need redraw for spinner animation
        } else {
            false
        }
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }
}

impl ThinkingBlock {
    /// Render collapsed state: single line
    ///
    /// While streaming: `▶ Thinking... ⠋`
    /// After complete:  `▶ Thinking`
    fn render_collapsed(&self, area: Rect, buf: &mut Buffer, theme: &Theme, _focused: bool) {
        let y = area.y;
        let text_style = Style::default()
            .fg(theme.text_color)
            .add_modifier(Modifier::ITALIC);

        let text = if self.streaming {
            format!("▶ Thinking... {}", self.spinner_frame())
        } else {
            "▶ Thinking".to_string()
        };

        for (i, ch) in text.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if i == 0 || (self.streaming && i == text.chars().count() - 1) {
                        cell.set_fg(theme.accent_color);
                    } else {
                        cell.set_style(text_style);
                    }
                }
            }
        }
    }

    /// Render expanded state with clip awareness
    ///
    /// When partially visible, skips rendering clipped borders:
    /// - clip_top > 0: skip top border
    /// - clip_bottom > 0: skip bottom border
    fn render_expanded_clipped(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        scroll_offset: u16,
        clip: Option<ClipContext>,
    ) {
        let (clip_top, clip_bottom) = clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0));

        let border_color = theme.border_color;
        let header_style = Style::default()
            .fg(theme.text_color)
            .add_modifier(Modifier::ITALIC);
        let content_style = Style::default()
            .fg(theme.dim_color)
            .add_modifier(Modifier::ITALIC);

        // Use cached lines if available (should be populated by prior height() call)
        let content_width = 76usize.min(area.width.saturating_sub(4) as usize);
        let fallback_lines;
        let lines: &[String] = if self.cached_width == area.width && !self.cached_lines.is_empty() {
            &self.cached_lines
        } else {
            // Fallback - cache should normally be populated but compute if not
            fallback_lines = wrap_text(&self.content, content_width);
            &fallback_lines
        };
        let total_lines = lines.len() as u16;
        let visible_lines = total_lines.min(MAX_VISIBLE_LINES);
        let needs_scrollbar = total_lines > MAX_VISIBLE_LINES;

        // Find longest line for box sizing
        let longest_line = lines
            .iter()
            .map(|l| UnicodeWidthStr::width(l.as_str()))
            .max()
            .unwrap_or(20);
        let box_inner_width = longest_line.max(20);
        let box_width = (box_inner_width + 4) as u16;
        let box_width = box_width.min(area.width);

        // Reserve space for scrollbar if needed (1 char)
        let content_end_x = if needs_scrollbar {
            area.x + box_width - 2
        } else {
            area.x + box_width - 1
        };
        let right_x = area.x + box_width - 1;

        // Track current render line (0 = top of visible area)
        let mut render_y = area.y;

        // Top border: ╭─ ▼ Thinking ─────────── ⠋ ─╮
        // Only render if top is not clipped
        if clip_top == 0 {
            let header_text = " ▼ Thinking ";
            let header_len = UnicodeWidthStr::width(header_text);

            if let Some(cell) = buf.cell_mut((area.x, render_y)) {
                cell.set_char('╭');
                cell.set_fg(border_color);
            }

            for (i, ch) in header_text.chars().enumerate() {
                let x = area.x + 1 + i as u16;
                if x < content_end_x {
                    if let Some(cell) = buf.cell_mut((x, render_y)) {
                        cell.set_char(ch);
                        if ch == '▼' {
                            cell.set_fg(theme.accent_color);
                        } else {
                            cell.set_style(header_style);
                        }
                    }
                }
            }

            let border_end = if self.streaming && !needs_scrollbar {
                right_x - 3
            } else {
                content_end_x
            };
            for x in (area.x + 1 + header_len as u16)..border_end {
                if let Some(cell) = buf.cell_mut((x, render_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            if self.streaming && !needs_scrollbar {
                let spinner = self.spinner_frame();
                if let Some(cell) = buf.cell_mut((right_x - 3, render_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
                if let Some(cell) = buf.cell_mut((right_x - 2, render_y)) {
                    cell.set_char(spinner);
                    cell.set_fg(theme.accent_color);
                }
                if let Some(cell) = buf.cell_mut((right_x - 1, render_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            // Fill gap at scrollbar column on header line
            if needs_scrollbar && content_end_x < right_x {
                if let Some(cell) = buf.cell_mut((content_end_x, render_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            if right_x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((right_x, render_y)) {
                    cell.set_char('╮');
                    cell.set_fg(border_color);
                }
            }

            render_y += 1;
        }

        // Content lines with side borders
        // When clipped from top, we skip the header but still render content
        let content_start_offset = if clip_top > 0 { clip_top - 1 } else { 0 }; // -1 because header is 1 line
        let start_line = (scroll_offset + content_start_offset) as usize;
        let lines_to_render = if clip_top > 0 {
            area.height - if clip_bottom == 0 { 1 } else { 0 } // Reserve for bottom if not clipped
        } else {
            area.height.saturating_sub(2) // Header + footer
        };

        for display_idx in 0..lines_to_render {
            let line_idx = start_line + display_idx as usize;
            let y = render_y + display_idx;

            if y >= area.y + area.height {
                break;
            }
            if clip_bottom == 0 && y >= area.y + area.height - 1 {
                break; // Reserve last line for bottom border
            }

            // Left border
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }

            // Content
            if let Some(line) = lines.get(line_idx) {
                let mut x = area.x + 2;
                for ch in line.chars() {
                    let char_width = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                    if x + char_width > content_end_x {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(ch);
                        cell.set_style(content_style);
                    }
                    // For wide chars (width=2), fill the next cell with space to occupy width
                    if char_width == 2 {
                        if let Some(cell) = buf.cell_mut((x + 1, y)) {
                            cell.set_char(' ');
                            cell.set_style(content_style);
                        }
                    }
                    x += char_width;
                }
            }

            // Right border
            if right_x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((right_x, y)) {
                    cell.set_char('│');
                    cell.set_fg(border_color);
                }
            }
        }

        // Bottom border - only render if bottom is not clipped
        if clip_bottom == 0 && area.height > 1 {
            let bottom_y = area.y + area.height - 1;

            if let Some(cell) = buf.cell_mut((area.x, bottom_y)) {
                cell.set_char('╰');
                cell.set_fg(border_color);
            }
            for x in (area.x + 1)..content_end_x {
                if let Some(cell) = buf.cell_mut((x, bottom_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }
            // Fill gap at scrollbar column on footer line
            if needs_scrollbar && content_end_x < right_x {
                if let Some(cell) = buf.cell_mut((content_end_x, bottom_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }
            if right_x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((right_x, bottom_y)) {
                    cell.set_char('╯');
                    cell.set_fg(border_color);
                }
            }
        }

        // Render scrollbar if needed
        if needs_scrollbar && area.height > 2 {
            let scrollbar_y = if clip_top == 0 { area.y + 1 } else { area.y };
            let scrollbar_height = if clip_top == 0 && clip_bottom == 0 {
                area.height - 2
            } else if clip_top == 0 || clip_bottom == 0 {
                area.height - 1
            } else {
                area.height
            };

            if scrollbar_height > 0 {
                let scrollbar_area = Rect::new(content_end_x, scrollbar_y, 1, scrollbar_height);
                render_scrollbar(
                    buf,
                    scrollbar_area,
                    scroll_offset as usize,
                    total_lines as usize,
                    visible_lines as usize,
                    theme.accent_color,
                    theme.scrollbar_bg_color,
                );
            }
        }
    }
}
