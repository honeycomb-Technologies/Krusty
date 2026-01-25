//! Write block - animated file writing display
//!
//! Shows file writing progress with typewriter animation:
//! - Cursor types out dots: ▌ .▌ ..▌ ...▌
//! - Cursor blinks at end, then resets
//! - Checkmark (✓) when complete
//! - Expandable to show written content

use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{BlockEvent, ClipContext, EventResult, StreamBlock, WidthScrollable};
use crate::tui::components::scrollbars::render_scrollbar;
use crate::tui::themes::Theme;

/// Animation intervals
const TYPE_INTERVAL: Duration = Duration::from_millis(80); // Typing speed
const BLINK_INTERVAL: Duration = Duration::from_millis(400); // Cursor blink
const PAUSE_DURATION: Duration = Duration::from_millis(500); // Pause before reset

/// Max visible content lines when expanded
const MAX_VISIBLE_LINES: u16 = 12;

/// Max content width for wrapping (matches ThinkingBlock)
const MAX_CONTENT_WIDTH: usize = 76;

/// Minimum box width for readability
const MIN_BOX_WIDTH: usize = 30;

/// Typewriter animation frames - typing out "Writing" with cursor
const TYPING_FRAMES: &[&str] = &[
    "▌",
    "W▌",
    "Wr▌",
    "Wri▌",
    "Writ▌",
    "Writi▌",
    "Writin▌",
    "Writing▌",
];

/// Blink frames (after typing complete)
const BLINK_ON: &str = "Writing▌";
const BLINK_OFF: &str = "Writing ";

/// Complete indicator
const COMPLETE_SYMBOL: &str = "✓";

/// Animation state
#[derive(Debug, Clone, Copy, PartialEq)]
enum AnimState {
    Typing,   // Typing out dots
    Blinking, // Cursor blinking at end
    Pausing,  // Brief pause before reset
}

/// A file writing block with typewriter animation
pub struct WriteBlock {
    /// Tool use ID for state persistence
    tool_use_id: Option<String>,
    /// File path being written
    file_path: String,
    /// File content (lines)
    content: Vec<String>,
    /// Total lines written
    total_lines: usize,
    /// Whether writing is still in progress
    streaming: bool,
    /// Whether the block is collapsed (default: true)
    collapsed: bool,
    /// Current animation frame index
    frame_idx: usize,
    /// Animation state
    anim_state: AnimState,
    /// Blink state (on/off)
    blink_on: bool,
    /// Blink count (reset after 2-3 blinks)
    blink_count: u8,
    /// Last animation update time
    last_frame_update: Instant,
    /// Scroll offset for content
    scroll_offset: u16,
    /// Cached wrapped lines
    cached_lines: Vec<String>,
    /// Width used for caching
    cached_width: u16,
    /// Cached height
    cached_height: u16,
}

impl WriteBlock {
    /// Create a new pending write block
    pub fn new_pending(file_path: String) -> Self {
        Self {
            tool_use_id: None,
            file_path,
            content: Vec::new(),
            total_lines: 0,
            streaming: true,
            collapsed: true,
            frame_idx: 0,
            anim_state: AnimState::Typing,
            blink_on: true,
            blink_count: 0,
            last_frame_update: Instant::now(),
            scroll_offset: 0,
            cached_lines: Vec::new(),
            cached_width: 0,
            cached_height: 1,
        }
    }

    /// Set the tool use ID (for state persistence)
    pub fn set_tool_use_id(&mut self, id: String) {
        self.tool_use_id = Some(id);
    }

    /// Get collapsed state
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Set collapsed state directly (for session restoration)
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    /// Check if this is a pending block (no content yet)
    pub fn is_pending(&self) -> bool {
        self.content.is_empty()
    }

    /// Set the file content
    pub fn set_content(&mut self, file_path: String, content: String) {
        self.file_path = file_path;
        self.content = content.lines().map(|s| s.to_string()).collect();
        self.total_lines = self.content.len();
        // Invalidate cache
        self.cached_width = 0;
        // Pre-compute expanded height
        self.update_cached_height(80); // Default width estimate
    }

    /// Mark as complete (stops animation)
    pub fn complete(&mut self) {
        self.streaming = false;
    }

    /// Update cached height for given width
    fn update_cached_height(&mut self, width: u16) {
        let content_width = MAX_CONTENT_WIDTH.min(width.saturating_sub(10) as usize);
        let lines = self.wrap_content(content_width);
        let content_lines = (lines.len() as u16).min(MAX_VISIBLE_LINES);
        self.cached_height = if self.collapsed {
            1
        } else {
            (content_lines + 2).max(3)
        };
    }

    /// Get current animation frame string
    fn get_anim_frame(&self) -> &'static str {
        if !self.streaming {
            return COMPLETE_SYMBOL;
        }

        match self.anim_state {
            AnimState::Typing => TYPING_FRAMES
                .get(self.frame_idx)
                .copied()
                .unwrap_or(TYPING_FRAMES[0]),
            AnimState::Blinking | AnimState::Pausing => {
                if self.blink_on {
                    BLINK_ON
                } else {
                    BLINK_OFF
                }
            }
        }
    }

    /// Update animation state
    fn update_animation(&mut self) {
        if !self.streaming {
            return;
        }

        let elapsed = self.last_frame_update.elapsed();

        match self.anim_state {
            AnimState::Typing => {
                if elapsed >= TYPE_INTERVAL {
                    self.frame_idx += 1;
                    if self.frame_idx >= TYPING_FRAMES.len() {
                        // Done typing, start blinking
                        self.anim_state = AnimState::Blinking;
                        self.blink_on = true;
                        self.blink_count = 0;
                    }
                    self.last_frame_update = Instant::now();
                }
            }
            AnimState::Blinking => {
                if elapsed >= BLINK_INTERVAL {
                    self.blink_on = !self.blink_on;
                    if !self.blink_on {
                        self.blink_count += 1;
                    }
                    // After 2 blinks, pause then reset
                    if self.blink_count >= 2 {
                        self.anim_state = AnimState::Pausing;
                    }
                    self.last_frame_update = Instant::now();
                }
            }
            AnimState::Pausing => {
                if elapsed >= PAUSE_DURATION {
                    // Reset to typing
                    self.anim_state = AnimState::Typing;
                    self.frame_idx = 0;
                    self.blink_count = 0;
                    self.blink_on = true;
                    self.last_frame_update = Instant::now();
                }
            }
        }
    }

    /// Toggle collapsed/expanded state
    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
        self.cached_height = 0; // Invalidate
        if self.collapsed {
            self.scroll_offset = 0;
        }
    }

    /// Wrap content lines to fit width
    fn wrap_content(&self, max_width: usize) -> Vec<String> {
        if max_width == 0 {
            return vec![];
        }

        let mut result = Vec::new();
        for line in &self.content {
            if line.is_empty() {
                result.push(String::new());
            } else if UnicodeWidthStr::width(line.as_str()) <= max_width {
                result.push(line.clone());
            } else {
                let mut current_line = String::new();
                let mut current_width = 0;
                for ch in line.chars() {
                    let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
                    if current_width + ch_width > max_width {
                        result.push(current_line);
                        current_line = String::new();
                        current_width = 0;
                    }
                    current_line.push(ch);
                    current_width += ch_width;
                }
                if !current_line.is_empty() {
                    result.push(current_line);
                }
            }
        }
        result
    }

    /// Get scroll info for drag handling
    /// Note: delegates to WidthScrollable::get_width_scroll_info
    pub fn get_scroll_info(&mut self, width: u16) -> (u16, u16, u16) {
        self.get_width_scroll_info(width)
    }

    /// Calculate actual rendered box width for hit testing
    pub fn box_width(&mut self, available_width: u16) -> u16 {
        let content_width = MAX_CONTENT_WIDTH.min(available_width.saturating_sub(10) as usize);
        let wrapped_lines = self.wrap_content(content_width);
        self.calc_box_width(&wrapped_lines, available_width)
    }

    /// Get short version of file path
    fn short_path(&self) -> &str {
        self.file_path.rsplit('/').next().unwrap_or(&self.file_path)
    }

    /// Calculate fitted box width for a given set of wrapped lines
    fn calc_box_width(&self, wrapped_lines: &[String], area_width: u16) -> u16 {
        let longest_line = wrapped_lines
            .iter()
            .map(|l| UnicodeWidthStr::width(l.as_str()))
            .max()
            .unwrap_or(MIN_BOX_WIDTH);

        // Header width calculation
        let anim = self.get_anim_frame();
        let status = format!(" Wrote {} ({} lines)", self.short_path(), self.total_lines);
        let header_text = format!(" ▼ {}{} ", anim, status);
        let header_width = UnicodeWidthStr::width(header_text.as_str());

        // Box inner = max(content + line_nums, header, min_width), capped at max
        let box_inner_width = (longest_line + 7)
            .max(header_width)
            .clamp(MIN_BOX_WIDTH, MAX_CONTENT_WIDTH + 7);
        ((box_inner_width + 3) as u16).min(area_width)
    }

    /// Render collapsed state
    fn render_collapsed(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let y = area.y;
        let text_style = Style::default()
            .fg(theme.text_color)
            .add_modifier(Modifier::ITALIC);

        let anim_color = if self.streaming {
            theme.accent_color
        } else {
            theme.success_color
        };

        let mut x = area.x;

        // Arrow indicator
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char('▶');
            cell.set_fg(theme.accent_color);
        }
        x += 1;

        // Space
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(' ');
        }
        x += 1;

        // Animation frame
        let anim = self.get_anim_frame();
        for ch in anim.chars() {
            if x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(anim_color);
                }
                x += UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
            }
        }

        // Space
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(' ');
        }
        x += 1;

        // Status text - only show filename when complete
        let status = if self.streaming {
            String::new()
        } else {
            format!(" Wrote {} ({} lines)", self.short_path(), self.total_lines)
        };

        for ch in status.chars() {
            if x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_style(text_style);
                }
                x += UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
            }
        }

        // Clear rest of line
        while x < area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_fg(Color::Reset);
            }
            x += 1;
        }
    }

    /// Render expanded state with content - fitted box width
    fn render_expanded(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        clip: Option<ClipContext>,
    ) {
        if area.width < 10 || area.height < 3 {
            return;
        }

        let (clip_top, clip_bottom) = clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0));

        let border_color = theme.border_color;
        let header_style = Style::default()
            .fg(theme.text_color)
            .add_modifier(Modifier::ITALIC);
        let content_color = theme.text_color;
        let line_num_color = theme.line_number_color;
        let anim_color = if self.streaming {
            theme.accent_color
        } else {
            theme.success_color
        };

        // Always compute fresh wrapped lines (like ToolResultBlock) - simpler and avoids cache bugs
        let content_width = MAX_CONTENT_WIDTH.min(area.width.saturating_sub(10) as usize);
        let wrapped_lines = self.wrap_content(content_width);

        let total_lines = wrapped_lines.len() as u16;
        let needs_scrollbar = total_lines > MAX_VISIBLE_LINES;

        // Use helper for consistent box width calculation
        let box_width = self.calc_box_width(&wrapped_lines, area.width);

        // Build header for rendering
        let anim = self.get_anim_frame();
        let status = format!(" Wrote {} ({} lines)", self.short_path(), self.total_lines);
        let header_text = format!(" ▼ {}{} ", anim, status);
        let anim_len = UnicodeWidthStr::width(anim);

        // Position borders relative to fitted box (not area.width)
        let content_end_x = if needs_scrollbar {
            area.x + box_width - 2
        } else {
            area.x + box_width - 1
        };
        let right_x = area.x + box_width - 1;

        // Header row
        if clip_top == 0 {
            let y = area.y;

            // Left corner
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('╭');
                cell.set_fg(border_color);
            }

            // Dash after corner
            if let Some(cell) = buf.cell_mut((area.x + 1, y)) {
                cell.set_char('─');
                cell.set_fg(border_color);
            }

            // Render header content (already built above): " ▼ Writing... Wrote file.rs (N lines) "
            let mut x = area.x + 2;
            let mut width_so_far = 0usize;
            for ch in header_text.chars() {
                let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
                if x < content_end_x {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(ch);
                        if ch == '▼' {
                            cell.set_fg(theme.accent_color);
                        } else if ch == '✓' || ch == '▌' {
                            cell.set_fg(anim_color);
                        } else if width_so_far >= 3 && width_so_far < 3 + anim_len {
                            // Animation text (after " ▼ ")
                            cell.set_fg(anim_color);
                        } else {
                            cell.set_style(header_style);
                        }
                    }
                    x += ch_width as u16;
                }
                width_so_far += ch_width;
            }

            // Fill with dashes
            for fx in x..content_end_x {
                if let Some(cell) = buf.cell_mut((fx, y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            // Scrollbar column dash
            if needs_scrollbar && content_end_x < right_x {
                if let Some(cell) = buf.cell_mut((content_end_x, y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            // Right corner
            if let Some(cell) = buf.cell_mut((right_x, y)) {
                cell.set_char('╮');
                cell.set_fg(border_color);
            }
        }

        // Content area
        let content_start_offset = if clip_top > 0 { clip_top - 1 } else { 0 };
        let start_line = (self.scroll_offset + content_start_offset) as usize;

        let reserved_top = if clip_top == 0 { 1u16 } else { 0 };
        let reserved_bottom = if clip_bottom == 0 { 1u16 } else { 0 };
        let content_lines_to_show = area.height.saturating_sub(reserved_top + reserved_bottom);
        let render_y = area.y + reserved_top;

        for display_idx in 0..content_lines_to_show {
            let line_idx = start_line + display_idx as usize;
            let y = render_y + display_idx;

            if y >= area.y + area.height {
                break;
            }
            if clip_bottom == 0 && y >= area.y + area.height - reserved_bottom {
                break;
            }

            // Left border
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }

            // Line number
            let line_num_str = format!("{:>5} ", line_idx + 1);
            for (i, ch) in line_num_str.chars().enumerate().take(6) {
                let x = area.x + 1 + i as u16;
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(line_num_color);
                }
            }

            // Content
            let content_start_x = area.x + 7;
            if let Some(line) = wrapped_lines.get(line_idx) {
                let mut x = content_start_x;
                for ch in line.chars() {
                    if x >= content_end_x {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(ch);
                        cell.set_fg(content_color);
                    }
                    x += UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
                }
            }

            // Right border
            if let Some(cell) = buf.cell_mut((right_x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }
        }

        // Bottom border
        if clip_bottom == 0 {
            let y = area.y + area.height - 1;

            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('╰');
                cell.set_fg(border_color);
            }

            for x in (area.x + 1)..content_end_x {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            if needs_scrollbar && content_end_x < right_x {
                if let Some(cell) = buf.cell_mut((content_end_x, y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            if let Some(cell) = buf.cell_mut((right_x, y)) {
                cell.set_char('╯');
                cell.set_fg(border_color);
            }
        }

        // Scrollbar
        if needs_scrollbar {
            let header_lines = if clip_top == 0 { 1u16 } else { 0 };
            let footer_lines = if clip_bottom == 0 { 1u16 } else { 0 };
            let scrollbar_height = area.height.saturating_sub(header_lines + footer_lines);

            if scrollbar_height > 0 {
                let scrollbar_y = area.y + header_lines;
                let scrollbar_area = Rect::new(content_end_x, scrollbar_y, 1, scrollbar_height);
                render_scrollbar(
                    buf,
                    scrollbar_area,
                    self.scroll_offset as usize,
                    total_lines as usize,
                    MAX_VISIBLE_LINES as usize,
                    theme.accent_color,
                    theme.scrollbar_bg_color,
                );
            }
        }
    }
}

impl WidthScrollable for WriteBlock {
    fn get_lines(&mut self, width: u16) -> &[String] {
        let content_width = MAX_CONTENT_WIDTH.min(width.saturating_sub(10) as usize);
        if self.cached_width != width || self.cached_lines.is_empty() {
            self.cached_lines = self.wrap_content(content_width);
            self.cached_width = width;
            let content_lines = (self.cached_lines.len() as u16).min(MAX_VISIBLE_LINES);
            self.cached_height = if self.collapsed {
                1
            } else {
                (content_lines + 2).max(3)
            };
        }
        &self.cached_lines
    }

    fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }

    fn set_scroll_offset(&mut self, offset: u16) {
        self.scroll_offset = offset;
    }

    fn max_visible_lines(&self) -> u16 {
        MAX_VISIBLE_LINES
    }
}

impl StreamBlock for WriteBlock {
    fn height(&self, width: u16, _theme: &Theme) -> u16 {
        if self.collapsed {
            1
        } else if self.cached_width == width && self.cached_height > 0 {
            self.cached_height
        } else {
            let content_width = MAX_CONTENT_WIDTH.min(width.saturating_sub(10) as usize);
            let lines = self.wrap_content(content_width);
            let content_lines = (lines.len() as u16).min(MAX_VISIBLE_LINES);
            (content_lines + 2).max(3)
        }
    }

    fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        _focused: bool,
        clip: Option<ClipContext>,
    ) {
        if area.height == 0 || area.width < 10 {
            return;
        }

        if self.collapsed {
            self.render_collapsed(area, buf, theme);
        } else {
            self.render_expanded(area, buf, theme, clip);
        }
    }

    fn handle_event(
        &mut self,
        event: &Event,
        area: Rect,
        clip: Option<ClipContext>,
    ) -> EventResult {
        let clip_top = clip.map_or(0, |c| c.clip_top);

        // Pre-calculate box width for hit testing (uses same logic as render)
        let content_width = MAX_CONTENT_WIDTH.min(area.width.saturating_sub(10) as usize);
        let wrapped_lines = self.wrap_content(content_width);
        let actual_width = self.calc_box_width(&wrapped_lines, area.width);

        match event {
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column,
                row,
                ..
            }) => {
                // Use actual rendered box width, not full area width
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
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                ..
            }) => {
                // Use actual rendered box width for hit testing
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + actual_width;

                if in_area {
                    let internal_y = (*row - area.y) + clip_top;

                    // Check scrollbar click when expanded
                    if !self.collapsed {
                        let needs_scrollbar = wrapped_lines.len() as u16 > MAX_VISIBLE_LINES;

                        if needs_scrollbar {
                            let scrollbar_x = area.x + actual_width - 2;

                            if *column == scrollbar_x && internal_y > 0 {
                                let total = wrapped_lines.len();
                                let visible = MAX_VISIBLE_LINES as usize;
                                let max_scroll = total.saturating_sub(visible);
                                let click_y = (internal_y - 1) as usize;
                                let new_offset = if visible > 0 {
                                    (click_y * max_scroll) / visible
                                } else {
                                    0
                                };
                                self.scroll_offset = new_offset.min(max_scroll) as u16;
                                return EventResult::Consumed;
                            }
                        }
                    }

                    // Toggle behavior - only when complete
                    if !self.streaming {
                        if self.collapsed {
                            self.toggle();
                            return EventResult::Action(BlockEvent::Expanded);
                        } else if internal_y == 0 {
                            self.toggle();
                            return EventResult::Action(BlockEvent::Collapsed);
                        }
                    }
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char(' '),
                ..
            }) if !self.streaming => {
                self.toggle();
                if self.collapsed {
                    EventResult::Action(BlockEvent::Collapsed)
                } else {
                    EventResult::Action(BlockEvent::Expanded)
                }
            }
            Event::Key(KeyEvent {
                code: KeyCode::Up, ..
            }) if !self.collapsed => {
                self.scroll_up();
                EventResult::Consumed
            }
            Event::Key(KeyEvent {
                code: KeyCode::Down,
                ..
            }) if !self.collapsed => {
                self.scroll_down(area.width);
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }

    fn tick(&mut self) -> bool {
        if self.streaming {
            self.update_animation();
            true
        } else {
            false
        }
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }

    fn get_text_content(&self) -> Option<String> {
        let header = if self.content.is_empty() {
            format!("Wrote {}", self.file_path)
        } else {
            format!("Wrote {} ({} lines)", self.file_path, self.content.len())
        };

        if self.collapsed {
            return Some(header);
        }

        let content = self.content.join("\n");
        Some(format!("{}\n{}", header, content).trim_end().to_string())
    }
}
