//! Bash block - terminal-style command output display
//!
//! Streams bash command output with:
//! - Heavy/thick line borders in theme accent color
//! - Scrollable content with scrollbar
//! - Auto-scroll to follow latest output
//! - Blinking cursor while streaming
//! - Progress bar detection
//! - Status indicator (running/success/error) with duration
//! - Collapsible when complete

use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{BlockEvent, ClipContext, EventResult, StreamBlock};
use crate::tui::components::scrollbars::render_scrollbar;
use crate::tui::themes::Theme;

/// Cursor blink interval
const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// Max visible content lines when expanded (before scrolling kicks in)
const MAX_VISIBLE_LINES: u16 = 12;

/// A terminal-style bash block
pub struct BashBlock {
    /// The command being executed
    command: String,
    /// Output content (streams in)
    output: String,
    /// Whether the block is collapsed
    collapsed: bool,
    /// Whether command is still running
    streaming: bool,
    /// Exit code when complete
    exit_code: Option<i32>,
    /// Start time for duration tracking
    start_time: Instant,
    /// Duration when complete
    duration: Option<Duration>,
    /// Detected progress (0.0 - 1.0)
    progress: Option<f32>,
    /// Progress text (e.g., "12/35 crates")
    progress_text: Option<String>,
    /// Cursor blink state
    cursor_visible: bool,
    /// Last cursor toggle time
    last_cursor_toggle: Instant,
    /// Scroll offset for content
    scroll_offset: u16,
    /// Cached wrapped lines
    cached_lines: Vec<String>,
    /// Width used for caching
    cached_width: u16,
    /// Cached height for quick access without mutable borrow
    cached_height: u16,
    /// Tool use ID for matching output chunks to the correct block
    tool_use_id: Option<String>,
    /// Process ID for background processes (tracked via ProcessRegistry)
    background_process_id: Option<String>,
    /// Flag indicating cache needs rebuild (deferred invalidation)
    cache_dirty: bool,
    /// Pending output to append (batched writes)
    pending_output: String,
}

impl BashBlock {
    /// Create a new bash block
    pub fn new(command: String) -> Self {
        let now = Instant::now();
        Self {
            command,
            output: String::new(),
            collapsed: false, // Start expanded to show streaming
            streaming: true,
            exit_code: None,
            start_time: now,
            duration: None,
            progress: None,
            progress_text: None,
            cursor_visible: true,
            last_cursor_toggle: now,
            scroll_offset: 0,
            cached_lines: Vec::new(),
            cached_width: 0,
            cached_height: 4, // minimum height
            tool_use_id: None,
            background_process_id: None,
            cache_dirty: false,
            pending_output: String::new(),
        }
    }

    /// Create a new bash block with tool use ID for output matching
    pub fn with_tool_id(command: String, tool_use_id: String) -> Self {
        let mut block = Self::new(command);
        block.tool_use_id = Some(tool_use_id);
        block
    }

    /// Get the tool use ID
    pub fn tool_use_id(&self) -> Option<&str> {
        self.tool_use_id.as_deref()
    }

    /// Get background process ID (if this is a background process)
    pub fn background_process_id(&self) -> Option<&str> {
        self.background_process_id.as_deref()
    }

    /// Get collapsed state
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Set collapsed state directly (for session restoration)
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    /// Set scroll offset (for session restoration)
    pub fn set_scroll_offset(&mut self, offset: u16) {
        self.scroll_offset = offset.min(self.max_scroll());
    }

    /// Check if command is still running (streaming output)
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// Set background process ID (converts this to a background process block)
    /// Called when tool result returns the process ID from ProcessRegistry
    pub fn set_background_process_id(&mut self, process_id: String) {
        self.background_process_id = Some(process_id);
        self.collapsed = true; // Background processes start collapsed (no streaming output)
        self.cursor_visible = false; // No cursor for background
    }

    /// Append streaming output (batched - call flush_pending() before render)
    pub fn append(&mut self, text: &str) {
        // Batch output - don't invalidate cache on every small chunk
        self.pending_output.push_str(text);
        self.cache_dirty = true;
    }

    /// Flush pending output and update cache if dirty
    /// Call this once per frame before rendering
    pub fn flush_pending(&mut self) {
        if !self.pending_output.is_empty() {
            self.output.push_str(&self.pending_output);

            // Only detect progress on newlines (expensive operation)
            if self.pending_output.contains('\n') {
                self.detect_progress();
            }

            self.pending_output.clear();

            // Invalidate cache dimensions (will rebuild on next get_lines call)
            self.cached_width = 0;
            self.cached_height = 0;

            // Auto-scroll to bottom while streaming
            if self.streaming {
                self.scroll_to_bottom();
            }
        }
        self.cache_dirty = false;
    }

    /// Mark command as complete
    pub fn complete(&mut self, exit_code: i32) {
        // Flush any pending output before marking complete
        self.flush_pending();
        self.streaming = false;
        self.exit_code = Some(exit_code);
        self.duration = Some(self.start_time.elapsed());
        self.scroll_to_bottom();
    }

    /// Scroll to bottom of content
    fn scroll_to_bottom(&mut self) {
        let max = self.max_scroll();
        self.scroll_offset = max;
    }

    /// Get formatted duration string
    fn duration_string(&self) -> String {
        let secs = self
            .duration
            .unwrap_or_else(|| self.start_time.elapsed())
            .as_secs_f32();
        if secs < 60.0 {
            format!("{:.1}s", secs)
        } else {
            format!("{:.1}m", secs / 60.0)
        }
    }

    /// Get status indicator
    /// Checks streaming flag first, then exit code for proper state display
    fn status_indicator(&self, theme: &Theme) -> (&'static str, Color) {
        // If still streaming, always show running indicator
        if self.streaming {
            return ("●", theme.running_color);
        }
        // Not streaming - check exit code for final status
        match self.exit_code {
            Some(0) => ("✓", theme.success_color),
            Some(_) => ("✗", theme.error_color),
            None => ("○", theme.dim_color),
        }
    }

    /// Detect progress from output patterns
    fn detect_progress(&mut self) {
        let last_lines: Vec<&str> = self.output.lines().rev().take(5).collect();

        for line in last_lines {
            // Pattern: "34%" or "34.5%"
            if let Some(pct) = Self::extract_percentage(line) {
                self.progress = Some(pct / 100.0);
                return;
            }

            // Pattern: "12/35" or "12 of 35"
            if let Some((current, total, text)) = Self::extract_fraction(line) {
                self.progress = Some(current as f32 / total as f32);
                self.progress_text = Some(text);
                return;
            }
        }
    }

    /// Extract percentage from text
    fn extract_percentage(text: &str) -> Option<f32> {
        text.split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_numeric() && c != '.' && c != '%'))
            .filter(|w| w.ends_with('%'))
            .filter_map(|w| w.trim_end_matches('%').parse::<f32>().ok())
            .find(|&pct| (0.0..=100.0).contains(&pct))
    }

    /// Extract fraction like "12/35" or "12 of 35"
    fn extract_fraction(text: &str) -> Option<(usize, usize, String)> {
        // Pattern: "12/35"
        for word in text.split_whitespace() {
            if let Some((a, b)) = word.split_once('/') {
                if let (Ok(current), Ok(total)) = (a.parse::<usize>(), b.parse::<usize>()) {
                    if current <= total && total > 0 {
                        return Some((current, total, format!("{}/{}", current, total)));
                    }
                }
            }
        }

        // Pattern: "12 of 35"
        let words: Vec<&str> = text.split_whitespace().collect();
        for i in 0..words.len().saturating_sub(2) {
            if words.get(i + 1) == Some(&"of") {
                if let (Ok(current), Ok(total)) = (
                    words[i].parse::<usize>(),
                    words[i + 2]
                        .trim_matches(|c: char| !c.is_numeric())
                        .parse::<usize>(),
                ) {
                    if current <= total && total > 0 {
                        return Some((current, total, format!("{} of {}", current, total)));
                    }
                }
            }
        }

        None
    }

    /// Update cursor blink state
    fn update_cursor(&mut self) {
        if self.streaming && self.last_cursor_toggle.elapsed() >= CURSOR_BLINK_INTERVAL {
            self.cursor_visible = !self.cursor_visible;
            self.last_cursor_toggle = Instant::now();
        }
    }

    /// Get wrapped lines for current width
    fn get_lines(&mut self, width: u16) -> &[String] {
        let content_width = width.saturating_sub(4) as usize; // borders + padding
        if self.cached_width != width || self.cached_lines.is_empty() {
            self.cached_lines = self.wrap_output(content_width);
            self.cached_width = width;
            // Also update cached height
            let content_lines = (self.cached_lines.len() as u16).min(MAX_VISIBLE_LINES);
            let has_progress = self.progress.is_some();
            self.cached_height = if has_progress {
                content_lines + 4
            } else {
                content_lines + 2
            }
            .max(4);
        }
        &self.cached_lines
    }

    /// Total content lines
    fn total_lines(&mut self, width: u16) -> u16 {
        self.get_lines(width).len() as u16
    }

    /// Visible lines (capped at MAX_VISIBLE_LINES)
    fn visible_lines(&mut self, width: u16) -> u16 {
        self.total_lines(width).min(MAX_VISIBLE_LINES)
    }

    /// Max scroll offset
    fn max_scroll(&mut self) -> u16 {
        let total = self.cached_lines.len() as u16;
        total.saturating_sub(MAX_VISIBLE_LINES)
    }

    /// Needs scrollbar?
    fn needs_scrollbar(&mut self, width: u16) -> bool {
        self.get_lines(width).len() as u16 > MAX_VISIBLE_LINES
    }

    /// Public scrollbar check
    pub fn has_scrollbar(&mut self, width: u16) -> bool {
        self.get_lines(width).len() as u16 > MAX_VISIBLE_LINES
    }

    /// Get scroll info for drag handling (optimized single pass)
    pub fn get_scroll_info(&mut self, width: u16) -> (u16, u16, u16) {
        let total = self.get_lines(width).len() as u16;
        let visible = total.min(MAX_VISIBLE_LINES);
        (total, visible, visible)
    }

    /// Scroll up
    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Scroll down
    fn scroll_down(&mut self) {
        let max = self.max_scroll();
        if self.scroll_offset < max {
            self.scroll_offset += 1;
        }
    }

    /// Wrap output text to width
    fn wrap_output(&self, max_width: usize) -> Vec<String> {
        if max_width == 0 || self.output.is_empty() {
            return vec![];
        }

        let mut result = Vec::new();
        for line in self.output.lines() {
            if line.is_empty() {
                result.push(String::new());
            } else if UnicodeWidthStr::width(line) <= max_width {
                result.push(line.to_string());
            } else {
                // Hard wrap long lines using unicode width
                let mut current_line = String::new();
                let mut current_width = 0usize;
                for ch in line.chars() {
                    let char_width = UnicodeWidthChar::width(ch).unwrap_or(1);
                    if current_width + char_width > max_width {
                        result.push(current_line);
                        current_line = String::new();
                        current_width = 0;
                    }
                    current_line.push(ch);
                    current_width += char_width;
                }
                if !current_line.is_empty() {
                    result.push(current_line);
                }
            }
        }
        result
    }

    /// Render collapsed state
    fn render_collapsed(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let y = area.y;
        let (status, status_color) = self.status_indicator(theme);
        let duration = self.duration_string();

        // Use accent color for consistency with expanded view
        let border_color = theme.accent_color;
        let text_color = theme.text_color;

        // Truncate command if needed - use only first line for multi-line commands
        let first_line = self.command.lines().next().unwrap_or(&self.command);
        let max_cmd_len = area.width.saturating_sub(20) as usize;
        let cmd_display = if first_line.len() > max_cmd_len {
            format!("{}...", &first_line[..max_cmd_len.saturating_sub(3)])
        } else if self.command.contains('\n') {
            format!("{}...", first_line)
        } else {
            self.command.clone()
        };

        let prefix = format!("▶ $ {}", cmd_display);
        let prefix_width = UnicodeWidthStr::width(prefix.as_str()) as u16;

        // Draw prefix
        let mut x = area.x;
        for ch in prefix.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if x + char_width > area.x + area.width {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(ch);
                if ch == '▶' || ch == '$' {
                    cell.set_fg(theme.accent_color);
                } else {
                    cell.set_fg(text_color);
                }
            }
            if char_width == 2 {
                if let Some(cell) = buf.cell_mut((x + 1, y)) {
                    cell.set_char(' ');
                }
            }
            x += char_width;
        }

        // Draw status and duration on right
        let suffix = format!(" {} {}", status, duration);
        let suffix_width = UnicodeWidthStr::width(suffix.as_str()) as u16;
        let suffix_start = area.x + area.width - suffix_width;
        let mut sx = suffix_start;
        for ch in suffix.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if sx >= area.x && sx + char_width <= area.x + area.width {
                if let Some(cell) = buf.cell_mut((sx, y)) {
                    cell.set_char(ch);
                    if ch == '✓' || ch == '✗' || ch == '●' {
                        cell.set_fg(status_color);
                    } else {
                        cell.set_fg(text_color);
                    }
                }
                if char_width == 2 {
                    if let Some(cell) = buf.cell_mut((sx + 1, y)) {
                        cell.set_char(' ');
                    }
                }
            }
            sx += char_width;
        }

        // Fill middle with dots
        let dots_start = area.x + prefix_width + 1;
        let dots_end = suffix_start.saturating_sub(1);
        for x in dots_start..dots_end {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char('·');
                cell.set_fg(border_color);
            }
        }
    }

    /// Render expanded state with clip awareness
    fn render_expanded_clipped(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        clip: Option<ClipContext>,
    ) {
        if area.height < 1 || area.width < 10 {
            return;
        }

        let (clip_top, clip_bottom) = clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0));

        // Always use accent color for bash block borders (consistent styling)
        let border_color = theme.accent_color;
        let content_color = theme.text_color;

        // Use cached lines if available (should be populated by prior height() call)
        let content_width = area.width.saturating_sub(4) as usize;
        let fallback_lines;
        let lines: &[String] = if self.cached_width == area.width && !self.cached_lines.is_empty() {
            &self.cached_lines
        } else {
            fallback_lines = self.wrap_output(content_width);
            &fallback_lines
        };
        let total_lines = lines.len() as u16;
        let visible_lines = total_lines.min(MAX_VISIBLE_LINES);
        let needs_scrollbar = total_lines > MAX_VISIBLE_LINES;
        let has_progress = self.progress.is_some();

        // Reserve space for scrollbar if needed
        let content_end_x = if needs_scrollbar {
            area.x + area.width - 2
        } else {
            area.x + area.width - 1
        };
        let right_x = area.x + area.width - 1;

        let mut render_y = area.y;

        // Top border - only if not clipped
        if clip_top == 0 {
            self.render_header(
                render_y,
                area.x,
                area.width,
                content_end_x,
                right_x,
                buf,
                theme,
                border_color,
                needs_scrollbar,
            );
            render_y += 1;
        }

        // Content area
        let content_start_offset = if clip_top > 0 { clip_top - 1 } else { 0 };
        let start_line = (self.scroll_offset + content_start_offset) as usize;

        // Calculate lines to render
        let reserved_bottom = if clip_bottom == 0 {
            if has_progress {
                3
            } else {
                1
            }
        } else {
            0
        };
        let reserved_top = if clip_top == 0 { 1 } else { 0 };
        let content_lines_to_show = area.height.saturating_sub(reserved_top + reserved_bottom);

        for display_idx in 0..content_lines_to_show {
            let line_idx = start_line + display_idx as usize;
            let y = render_y + display_idx;

            if y >= area.y + area.height {
                break;
            }
            if clip_bottom == 0 && y >= area.y + area.height - reserved_bottom {
                break;
            }

            // Left border (heavy vertical)
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('┃');
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
                        cell.set_fg(content_color);
                    }
                    if char_width == 2 {
                        if let Some(cell) = buf.cell_mut((x + 1, y)) {
                            cell.set_char(' ');
                        }
                    }
                    x += char_width;
                }

                // Cursor at end of last visible line while streaming
                if self.streaming
                    && line_idx == lines.len().saturating_sub(1)
                    && self.cursor_visible
                {
                    let cursor_x = area.x + 2 + UnicodeWidthStr::width(line.as_str()) as u16;
                    if cursor_x < content_end_x {
                        if let Some(cell) = buf.cell_mut((cursor_x, y)) {
                            cell.set_char('█');
                            cell.set_fg(theme.accent_color);
                        }
                    }
                }
            }

            // Right border (heavy vertical)
            if let Some(cell) = buf.cell_mut((right_x, y)) {
                cell.set_char('┃');
                cell.set_fg(border_color);
            }
        }

        // Progress bar and bottom border - only if not clipped from bottom
        if clip_bottom == 0 {
            if has_progress {
                let progress_y = area.y + area.height - 3;
                self.render_progress_bar(area, buf, progress_y, border_color, theme);
            }
            self.render_footer(
                area,
                buf,
                border_color,
                content_end_x,
                right_x,
                needs_scrollbar,
                theme,
            );
        }

        // Render scrollbar if needed
        if needs_scrollbar {
            let header_lines = if clip_top == 0 { 1u16 } else { 0 };
            let footer_lines = if clip_bottom == 0 {
                if has_progress {
                    3u16
                } else {
                    1u16
                }
            } else {
                0
            };
            let scrollbar_height = area.height.saturating_sub(header_lines + footer_lines);

            if scrollbar_height > 0 {
                let scrollbar_y = area.y + header_lines;
                let scrollbar_area = Rect::new(content_end_x, scrollbar_y, 1, scrollbar_height);
                render_scrollbar(
                    buf,
                    scrollbar_area,
                    self.scroll_offset as usize,
                    total_lines as usize,
                    visible_lines as usize,
                    theme.accent_color,
                    theme.scrollbar_bg_color,
                );
            }
        }
    }

    /// Render header with command (heavy/thick line style)
    fn render_header(
        &self,
        y: u16,
        x: u16,
        width: u16,
        content_end_x: u16,
        right_x: u16,
        buf: &mut Buffer,
        theme: &Theme,
        border_color: Color,
        needs_scrollbar: bool,
    ) {
        let text_color = theme.text_color;

        // Left corner (heavy)
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char('┏');
            cell.set_fg(border_color);
        }

        // Get status indicator for header
        let (status, status_color) = self.status_indicator(theme);
        let duration = self.duration_string();
        let status_suffix = format!(" {} {} ", status, duration);
        let status_width = UnicodeWidthStr::width(status_suffix.as_str()) as u16;

        // " ▼ $ command " - reserve space for status
        // Use only the first line of the command to prevent multi-line heredocs from bleeding
        let first_line = self.command.lines().next().unwrap_or(&self.command);
        let max_cmd_width = (width as usize).saturating_sub(12 + status_width as usize);
        let cmd_display = if UnicodeWidthStr::width(first_line) > max_cmd_width {
            // Truncate by unicode width
            let mut truncated = String::new();
            let mut w = 0usize;
            for ch in first_line.chars() {
                let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
                if w + cw + 3 > max_cmd_width {
                    break;
                }
                truncated.push(ch);
                w += cw;
            }
            format!("{}...", truncated)
        } else if self.command.contains('\n') {
            // Multi-line command - show first line with indicator
            format!("{}...", first_line)
        } else {
            self.command.clone()
        };

        let header = format!(" ▼ $ {} ", cmd_display);

        let mut cx = x + 1;
        for ch in header.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if cx + char_width > content_end_x.saturating_sub(status_width) {
                break;
            }
            if let Some(cell) = buf.cell_mut((cx, y)) {
                cell.set_char(ch);
                if ch == '▼' || ch == '$' {
                    cell.set_fg(theme.accent_color);
                } else {
                    cell.set_fg(text_color);
                }
            }
            if char_width == 2 {
                if let Some(cell) = buf.cell_mut((cx + 1, y)) {
                    cell.set_char(' ');
                }
            }
            cx += char_width;
        }

        // Fill with ━ (heavy horizontal) up to status
        let status_start = content_end_x.saturating_sub(status_width);
        while cx < status_start {
            if let Some(cell) = buf.cell_mut((cx, y)) {
                cell.set_char('━');
                cell.set_fg(border_color);
            }
            cx += 1;
        }

        // Draw status indicator (● running, ✓ success, ✗ error) and duration
        for ch in status_suffix.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if cx + char_width > content_end_x {
                break;
            }
            if let Some(cell) = buf.cell_mut((cx, y)) {
                cell.set_char(ch);
                if ch == '●' || ch == '✓' || ch == '✗' {
                    cell.set_fg(status_color);
                } else {
                    cell.set_fg(theme.dim_color);
                }
            }
            if char_width == 2 {
                if let Some(cell) = buf.cell_mut((cx + 1, y)) {
                    cell.set_char(' ');
                }
            }
            cx += char_width;
        }

        // Fill gap at scrollbar column
        if needs_scrollbar && content_end_x < right_x {
            if let Some(cell) = buf.cell_mut((content_end_x, y)) {
                cell.set_char('━');
                cell.set_fg(border_color);
            }
        }

        // Right corner (heavy)
        if let Some(cell) = buf.cell_mut((right_x, y)) {
            cell.set_char('┓');
            cell.set_fg(border_color);
        }
    }

    /// Render progress bar
    fn render_progress_bar(
        &self,
        area: Rect,
        buf: &mut Buffer,
        y: u16,
        border_color: Color,
        theme: &Theme,
    ) {
        let progress = self.progress.unwrap_or(0.0);
        let right_x = area.x + area.width - 1;

        // Separator line (heavy horizontal with tees)
        if let Some(cell) = buf.cell_mut((area.x, y)) {
            cell.set_char('┠');
            cell.set_fg(border_color);
        }
        for px in (area.x + 1)..right_x {
            if let Some(cell) = buf.cell_mut((px, y)) {
                cell.set_char('━');
                cell.set_fg(border_color);
            }
        }
        if let Some(cell) = buf.cell_mut((right_x, y)) {
            cell.set_char('┨');
            cell.set_fg(border_color);
        }

        // Progress bar line
        let bar_y = y + 1;

        // Left border (heavy vertical)
        if let Some(cell) = buf.cell_mut((area.x, bar_y)) {
            cell.set_char('┃');
            cell.set_fg(border_color);
        }

        // Progress bar
        let bar_width = (area.width as usize).saturating_sub(12);
        let filled = (progress * bar_width as f32) as usize;

        let mut px = area.x + 2;

        // Filled portion
        for _ in 0..filled {
            if px < area.x + area.width - 10 {
                if let Some(cell) = buf.cell_mut((px, bar_y)) {
                    cell.set_char('█');
                    cell.set_fg(theme.accent_color);
                }
                px += 1;
            }
        }

        // Empty portion
        for _ in filled..bar_width {
            if px < area.x + area.width - 10 {
                if let Some(cell) = buf.cell_mut((px, bar_y)) {
                    cell.set_char('░');
                    cell.set_fg(border_color);
                }
                px += 1;
            }
        }

        // Percentage
        let pct_text = if let Some(ref text) = self.progress_text {
            format!(" {} ", text)
        } else {
            format!(" {:3.0}% ", progress * 100.0)
        };

        for ch in pct_text.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if px + char_width > right_x {
                break;
            }
            if let Some(cell) = buf.cell_mut((px, bar_y)) {
                cell.set_char(ch);
                cell.set_fg(theme.dim_color);
            }
            if char_width == 2 {
                if let Some(cell) = buf.cell_mut((px + 1, bar_y)) {
                    cell.set_char(' ');
                }
            }
            px += char_width;
        }

        // Right border (heavy vertical)
        if let Some(cell) = buf.cell_mut((right_x, bar_y)) {
            cell.set_char('┃');
            cell.set_fg(border_color);
        }
    }

    /// Render footer
    fn render_footer(
        &self,
        area: Rect,
        buf: &mut Buffer,
        border_color: Color,
        content_end_x: u16,
        right_x: u16,
        needs_scrollbar: bool,
        _theme: &Theme,
    ) {
        let y = area.y + area.height - 1;

        // Left corner (heavy)
        if let Some(cell) = buf.cell_mut((area.x, y)) {
            cell.set_char('┗');
            cell.set_fg(border_color);
        }

        // Fill with ━ (heavy horizontal)
        for fx in (area.x + 1)..content_end_x {
            if let Some(cell) = buf.cell_mut((fx, y)) {
                cell.set_char('━');
                cell.set_fg(border_color);
            }
        }

        // Status is now shown in header, no need to duplicate in footer

        // Fill gap at scrollbar column
        if needs_scrollbar && content_end_x < right_x {
            if let Some(cell) = buf.cell_mut((content_end_x, y)) {
                cell.set_char('━');
                cell.set_fg(border_color);
            }
        }

        // Right corner (heavy)
        if let Some(cell) = buf.cell_mut((right_x, y)) {
            cell.set_char('┛');
            cell.set_fg(border_color);
        }
    }
}

impl Default for BashBlock {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl StreamBlock for BashBlock {
    fn height(&self, width: u16, _theme: &Theme) -> u16 {
        if self.collapsed {
            1
        } else if self.cached_width == width && self.cached_height > 0 {
            // Use cached height if available and width matches
            self.cached_height
        } else {
            // Fallback: compute without caching (rare case)
            let content_width = width.saturating_sub(4) as usize;
            let lines = self.wrap_output(content_width);
            let content_lines = (lines.len() as u16).min(MAX_VISIBLE_LINES);
            let has_progress = self.progress.is_some();

            if has_progress {
                content_lines + 4
            } else {
                content_lines + 2
            }
            .max(4)
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
            self.render_expanded_clipped(area, buf, theme, clip);
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
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + area.width;

                if in_area && !self.collapsed {
                    self.scroll_down();
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
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + area.width;

                if in_area && !self.collapsed {
                    self.scroll_up();
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            // Click on block
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                ..
            }) => {
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + area.width;

                if in_area {
                    let internal_y = (*row - area.y) + clip_top;

                    // Check scrollbar click when expanded
                    if !self.collapsed && self.needs_scrollbar(area.width) {
                        let scrollbar_x = area.x + area.width - 2;
                        if *column >= scrollbar_x && internal_y > 0 {
                            let total = self.total_lines(area.width) as usize;
                            let visible = self.visible_lines(area.width) as usize;
                            let max_scroll = total.saturating_sub(visible);
                            let track_height = visible;
                            let click_y = (internal_y - 1) as usize;
                            let new_offset = if track_height > 0 {
                                (click_y * max_scroll) / track_height
                            } else {
                                0
                            };
                            self.scroll_offset = new_offset.min(max_scroll) as u16;
                            return EventResult::Consumed;
                        }
                    }

                    // Toggle behavior: collapsed=any click, expanded=header only
                    if self.collapsed {
                        self.collapsed = false;
                        return EventResult::Action(BlockEvent::Expanded);
                    } else if internal_y == 0 {
                        self.collapsed = true;
                        self.scroll_offset = 0;
                        return EventResult::Action(BlockEvent::Collapsed);
                    }
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            Event::Key(KeyEvent {
                code: KeyCode::Enter | KeyCode::Char(' '),
                ..
            }) => {
                self.collapsed = !self.collapsed;
                if self.collapsed {
                    self.scroll_offset = 0;
                    EventResult::Action(BlockEvent::Collapsed)
                } else {
                    EventResult::Action(BlockEvent::Expanded)
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn get_text_content(&self) -> Option<String> {
        let base = format!("$ {}", self.command);
        Some(if self.collapsed || self.output.is_empty() {
            base
        } else {
            format!("{}\n{}", base, self.output)
        })
    }

    fn tick(&mut self) -> bool {
        // Flush any pending output before render (batched writes)
        let had_pending = !self.pending_output.is_empty();
        self.flush_pending();

        if self.streaming {
            self.update_cursor();
            true // Need redraw for cursor blink or new output
        } else {
            had_pending // Only redraw if we flushed new content
        }
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }
}
