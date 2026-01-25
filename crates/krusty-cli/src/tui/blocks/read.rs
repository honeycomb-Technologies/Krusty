//! Read block - animated file reading display
//!
//! Shows file reading progress with animated crab eyes:
//! - Eyes scan left/right while reading
//! - Eyes blink periodically
//! - Happy face (^_^) when complete
//! - Expandable to show file content preview

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

/// Animation frame interval for eye movement
const EYE_MOVE_INTERVAL: Duration = Duration::from_millis(250);
/// Animation frame interval for blinking
const BLINK_INTERVAL: Duration = Duration::from_millis(120);

/// Max visible content lines when expanded (before scrolling kicks in)
const MAX_VISIBLE_LINES: u16 = 12;

/// Max content width for wrapping (matches ThinkingBlock)
const MAX_CONTENT_WIDTH: usize = 76;

/// Minimum box width for readability
const MIN_BOX_WIDTH: usize = 30;

/// Eye animation frames - fixed 9-char width for stable layout
/// Format: padding + left_eye + 5 spaces + right_eye + padding
const EYE_FRAMES: &[&str] = &[
    " o     o ", // 0: center (normal)
    "  o     o", // 1: look right
    " o     o ", // 2: center
    "o     o  ", // 3: look left
    " o     o ", // 4: center
    " -     - ", // 5: blink
    " o     o ", // 6: open
    " -     - ", // 7: blink again
];

/// Happy face - same 9-char width
const HAPPY_FACE: &str = " ^  _  ^ ";

/// A file reading block with animated eyes
pub struct ReadBlock {
    /// Tool use ID for matching results
    tool_use_id: String,
    /// File path being read
    file_path: String,
    /// File content (lines)
    content: Vec<String>,
    /// Total lines in file
    total_lines: usize,
    /// Lines actually returned (may be limited)
    lines_returned: usize,
    /// Whether reading is still in progress
    streaming: bool,
    /// Whether the block is collapsed (default: true)
    collapsed: bool,
    /// Current animation frame index
    frame_idx: usize,
    /// Last animation update time
    last_frame_update: Instant,
    /// Whether currently in blink frame (uses shorter timing)
    in_blink: bool,
    /// Scroll offset for content
    scroll_offset: u16,
    /// Cached wrapped lines
    cached_lines: Vec<String>,
    /// Width used for caching
    cached_width: u16,
    /// Cached height
    cached_height: u16,
}

impl ReadBlock {
    /// Create a new read block for a file
    pub fn new(tool_use_id: String, file_path: String) -> Self {
        Self {
            tool_use_id,
            file_path,
            content: Vec::new(),
            total_lines: 0,
            lines_returned: 0,
            streaming: true,
            collapsed: true,
            frame_idx: 0,
            last_frame_update: Instant::now(),
            in_blink: false,
            scroll_offset: 0,
            cached_lines: Vec::new(),
            cached_width: 0,
            cached_height: 1,
        }
    }

    /// Get the tool use ID
    pub fn tool_use_id(&self) -> &str {
        &self.tool_use_id
    }

    /// Get collapsed state
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Set collapsed state directly (for session restoration)
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    /// Set the file content (does NOT mark complete - animation continues)
    pub fn set_content(&mut self, content: String, total_lines: usize, lines_returned: usize) {
        // Replace tabs with spaces (tabs cause display issues in TUI cells)
        self.content = content.lines().map(|s| s.replace('\t', "    ")).collect();
        self.total_lines = total_lines;
        self.lines_returned = lines_returned;
        // Invalidate cache
        self.cached_width = 0;
    }

    /// Mark as complete (stops animation)
    pub fn complete(&mut self) {
        self.streaming = false;
    }

    /// Update animation frame
    fn update_animation(&mut self) {
        if !self.streaming {
            return;
        }

        let interval = if self.in_blink {
            BLINK_INTERVAL
        } else {
            EYE_MOVE_INTERVAL
        };

        if self.last_frame_update.elapsed() >= interval {
            self.frame_idx = (self.frame_idx + 1) % EYE_FRAMES.len();
            self.last_frame_update = Instant::now();
            self.in_blink = matches!(self.frame_idx, 5 | 7);
        }
    }

    /// Toggle collapsed/expanded state
    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
        // Invalidate cached height since it depends on collapsed state
        self.cached_height = 0;
        if self.collapsed {
            self.scroll_offset = 0;
        }
    }

    /// Wrap content lines to fit width (width-based, handles wide characters)
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
                // Use width-based chunking for proper display width handling
                let mut chunk = String::new();
                let mut chunk_width = 0usize;
                for ch in line.chars() {
                    let char_width = UnicodeWidthChar::width(ch).unwrap_or(1);
                    if chunk_width + char_width > max_width && !chunk.is_empty() {
                        result.push(chunk);
                        chunk = String::new();
                        chunk_width = 0;
                    }
                    chunk.push(ch);
                    chunk_width += char_width;
                }
                if !chunk.is_empty() {
                    result.push(chunk);
                }
            }
        }
        result
    }

    /// Check if block needs a scrollbar
    /// Note: delegates to WidthScrollable::needs_scrollbar
    pub fn has_scrollbar(&mut self, width: u16) -> bool {
        WidthScrollable::needs_scrollbar(self, width)
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

    /// Get short version of file path for display
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
        let eyes = if self.streaming {
            EYE_FRAMES[self.frame_idx % EYE_FRAMES.len()]
        } else {
            HAPPY_FACE
        };
        let status = if self.streaming {
            format!("Reading {}...", self.short_path())
        } else {
            format!("Read {} ({} lines)", self.short_path(), self.total_lines)
        };
        let header_text = format!(" ▼ {} {} ", eyes, status);
        let header_width = UnicodeWidthStr::width(header_text.as_str());

        // Box inner = max(content + line_nums, header, min_width), capped at max
        let box_inner_width = (longest_line + 7)
            .max(header_width)
            .clamp(MIN_BOX_WIDTH, MAX_CONTENT_WIDTH + 7);
        ((box_inner_width + 3) as u16).min(area_width)
    }

    /// Render collapsed state - simple text like ThinkingBlock
    /// While streaming: `▶ o o Reading file.rs...`
    /// After complete:  `▶ ^_^ Read file.rs (150 lines)`
    fn render_collapsed(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        // Only clear the single line we're rendering (match ToolResultBlock pattern)
        let y = area.y;
        let text_style = Style::default()
            .fg(theme.text_color)
            .add_modifier(Modifier::ITALIC);

        let eye_color = if self.streaming {
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

        // Eyes - use pre-defined frames with proper spacing (all 9 chars wide)
        let eyes = if self.streaming {
            EYE_FRAMES[self.frame_idx % EYE_FRAMES.len()]
        } else {
            HAPPY_FACE
        };

        for ch in eyes.chars() {
            if x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    if ch != ' ' {
                        cell.set_fg(eye_color);
                    }
                }
                x += 1;
            }
        }

        // Space
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(' ');
        }
        x += 1;

        // Status text
        let status = if self.streaming {
            format!("Reading {}...", self.short_path())
        } else {
            format!("Read {} ({} lines)", self.short_path(), self.total_lines)
        };

        for ch in status.chars() {
            if x < area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_style(text_style);
                }
                x += 1;
            }
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
        let eye_color = if self.streaming {
            theme.accent_color
        } else {
            theme.success_color
        };

        // Fitted-width approach (matches WriteBlock)
        // Cap content width and fit box to actual content
        let content_width = MAX_CONTENT_WIDTH.min(area.width.saturating_sub(10) as usize);
        let wrapped_lines = self.wrap_content(content_width);

        let total_lines = wrapped_lines.len() as u16;
        let needs_scrollbar = total_lines > MAX_VISIBLE_LINES;

        // Use helper for consistent box width calculation
        let box_width = self.calc_box_width(&wrapped_lines, area.width);

        // Position borders relative to fitted box (not area.width)
        let content_end_x = if needs_scrollbar {
            area.x + box_width - 2
        } else {
            area.x + box_width - 1
        };
        let right_x = area.x + box_width - 1;

        // Build header text
        let eyes = if self.streaming {
            EYE_FRAMES[self.frame_idx % EYE_FRAMES.len()]
        } else {
            HAPPY_FACE
        };
        let status = if self.streaming {
            format!("Reading {}...", self.short_path())
        } else {
            format!("Read {} ({} lines)", self.short_path(), self.total_lines)
        };
        let header_text = format!(" ▼ {} {} ", eyes, status);

        // Header row (only if not clipped at top)
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

            // Render header content (already built above): " ▼ ^_^ Read file.rs (59 lines) "
            let mut x = area.x + 2;
            for ch in header_text.chars() {
                if x < content_end_x {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(ch);
                        if ch == '▼' {
                            cell.set_fg(theme.accent_color);
                        } else if ch == '^' || ch == 'o' || ch == '-' || ch == '_' {
                            cell.set_fg(eye_color);
                        } else {
                            cell.set_style(header_style);
                        }
                    }
                    x += UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                }
            }

            // Fill rest with dashes up to content_end_x
            for fx in x..content_end_x {
                if let Some(cell) = buf.cell_mut((fx, y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            // Scrollbar column dash (between content and border)
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

        // Content area - match BashBlock's pattern exactly
        let content_start_offset = if clip_top > 0 { clip_top - 1 } else { 0 };
        let start_line = (self.scroll_offset + content_start_offset) as usize;

        let reserved_top = if clip_top == 0 { 1u16 } else { 0 };
        let reserved_bottom = if clip_bottom == 0 { 1u16 } else { 0 };
        let content_lines_to_show = area.height.saturating_sub(reserved_top + reserved_bottom);
        let render_y = area.y + reserved_top;

        for display_idx in 0..content_lines_to_show {
            let line_idx = start_line + display_idx as usize;
            let y = render_y + display_idx;

            // Bounds check
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

            // Line number (5 chars + space = 6 chars)
            let line_num_str = format!("{:>5} ", line_idx + 1);
            for (i, ch) in line_num_str.chars().enumerate().take(6) {
                let x = area.x + 1 + i as u16;
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(line_num_color);
                }
            }

            // Content starts after line number
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
                    x += UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                }
            }

            // Right border
            if let Some(cell) = buf.cell_mut((right_x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }
        }

        // Bottom border (only if not clipped at bottom)
        if clip_bottom == 0 {
            let y = area.y + area.height - 1;

            // Left corner
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('╰');
                cell.set_fg(border_color);
            }

            // Fill with dashes
            for x in (area.x + 1)..content_end_x {
                if let Some(cell) = buf.cell_mut((x, y)) {
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
                cell.set_char('╯');
                cell.set_fg(border_color);
            }
        }

        // Scrollbar - render at content_end_x when needed
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

impl WidthScrollable for ReadBlock {
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

impl StreamBlock for ReadBlock {
    fn height(&self, width: u16, _theme: &Theme) -> u16 {
        if self.collapsed {
            1
        } else if self.cached_width == width && self.cached_height > 0 {
            // Use cached height if available and width matches
            self.cached_height
        } else {
            // Fallback: compute without caching (rare case)
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
                    // Translate to block-internal Y coordinate
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

                    // Toggle behavior: collapsed clicks anywhere, expanded only header
                    if self.collapsed {
                        self.toggle();
                        return EventResult::Action(BlockEvent::Expanded);
                    } else if internal_y == 0 {
                        self.toggle();
                        return EventResult::Action(BlockEvent::Collapsed);
                    }
                    // Click in content area - consume but don't toggle
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char(' '),
                ..
            }) => {
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
        let header = if self.total_lines > 0 {
            format!("{} ({} lines)", self.file_path, self.total_lines)
        } else {
            self.file_path.clone()
        };

        if self.collapsed {
            return Some(header);
        }

        let content = self.content.join("\n");
        Some(format!("{}\n{}", header, content).trim_end().to_string())
    }
}
