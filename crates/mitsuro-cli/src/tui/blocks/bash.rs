//! Bash block - hybrid dense command output display
//!
//! Streams bash command output with:
//! - Compact activity-row header using shared DotEcho language
//! - Short live tail while running (hybrid density)
//! - Light left rail instead of heavy full terminal chrome
//! - Auto-scroll to follow latest output
//! - Progress as an inline suffix (or thin bar only when expanded fully)
//! - Status indicator (running/success/error) with duration
//! - Auto-collapse on success; stay open on failure

use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{BlockEvent, ClipContext, EventResult, StreamBlock};
use crate::tui::components::scrollbars::render_scrollbar;
use crate::tui::components::{activity_echo_frame, ACTIVITY_ECHO_FRAMES, ACTIVITY_ECHO_INTERVAL};
use crate::tui::themes::Theme;
use crate::tui::utils::truncate_ellipsis;

/// Live tail lines while streaming (hybrid mode)
const LIVE_TAIL_LINES: u16 = 3;

/// Max visible content lines when fully expanded
const MAX_VISIBLE_LINES: u16 = 6;

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
    /// Activity echo frame index (shared DotEcho language)
    activity_idx: usize,
    /// Last activity frame update
    last_activity_update: Instant,
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
            // Hybrid: live tail while streaming (not full terminal chrome)
            collapsed: false,
            streaming: true,
            exit_code: None,
            start_time: now,
            duration: None,
            progress: None,
            progress_text: None,
            activity_idx: 0,
            last_activity_update: now,
            scroll_offset: 0,
            cached_lines: Vec::new(),
            cached_width: 0,
            cached_height: 1 + LIVE_TAIL_LINES, // header + live tail
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
        // Hybrid: auto-collapse on success, keep open (or open) on failure
        self.collapsed = exit_code == 0;
        // Presentation mode changed (live-tail → collapsed/expanded); rebuild height.
        self.cached_width = 0;
        self.cached_height = 0;
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
    fn status_indicator(&self, theme: &Theme) -> (String, Color) {
        // If still streaming, always show activity echo
        if self.streaming {
            return (
                activity_echo_frame(self.activity_idx).to_string(),
                theme.running_color,
            );
        }
        // Not streaming - check exit code for final status
        match self.exit_code {
            Some(0) => ("✓".to_string(), theme.success_color),
            Some(_) => ("✗".to_string(), theme.error_color),
            None => ("○".to_string(), theme.dim_color),
        }
    }

    /// Max content lines for current presentation mode
    fn content_cap(&self) -> u16 {
        if self.streaming && !self.collapsed {
            LIVE_TAIL_LINES
        } else {
            MAX_VISIBLE_LINES
        }
    }

    /// Progress suffix for header (compact, not a multi-line bar)
    fn progress_suffix(&self) -> Option<String> {
        if let Some(ref text) = self.progress_text {
            Some(text.clone())
        } else {
            self.progress.map(|p| format!("{:.0}%", p * 100.0))
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

    /// Update activity echo frame
    fn update_activity(&mut self) {
        if self.streaming && self.last_activity_update.elapsed() >= ACTIVITY_ECHO_INTERVAL {
            self.activity_idx = (self.activity_idx + 1) % ACTIVITY_ECHO_FRAMES.len();
            self.last_activity_update = Instant::now();
        }
    }

    /// Get wrapped lines for current width
    fn get_lines(&mut self, width: u16) -> &[String] {
        // Light left rail + padding (not heavy dual borders)
        let content_width = width.saturating_sub(3) as usize;
        if self.cached_width != width || self.cached_lines.is_empty() {
            self.cached_lines = self.wrap_output(content_width);
            self.cached_width = width;
            let cap = self.content_cap();
            let content_lines = (self.cached_lines.len() as u16).min(cap);
            // Header + optional content (no heavy footer/progress chrome)
            self.cached_height = if content_lines == 0 {
                1
            } else {
                content_lines + 1
            };
        }
        &self.cached_lines
    }

    /// Total content lines
    fn total_lines(&mut self, width: u16) -> u16 {
        self.get_lines(width).len() as u16
    }

    /// Visible lines (capped for hybrid mode)
    fn visible_lines(&mut self, width: u16) -> u16 {
        let cap = self.content_cap();
        self.total_lines(width).min(cap)
    }

    /// Max scroll offset
    fn max_scroll(&mut self) -> u16 {
        let total = self.cached_lines.len() as u16;
        let cap = self.content_cap();
        total.saturating_sub(cap)
    }

    /// Needs scrollbar?
    fn needs_scrollbar(&mut self, width: u16) -> bool {
        let cap = self.content_cap();
        self.get_lines(width).len() as u16 > cap
    }

    /// Public scrollbar check
    pub fn has_scrollbar(&mut self, width: u16) -> bool {
        let cap = self.content_cap();
        self.get_lines(width).len() as u16 > cap
    }

    /// Get scroll info for drag handling (optimized single pass)
    pub fn get_scroll_info(&mut self, width: u16) -> (u16, u16, u16) {
        let total = self.get_lines(width).len() as u16;
        let visible = total.min(self.content_cap());
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

    /// Truncate command for header display.
    fn command_display(&self, max_cmd_width: usize) -> String {
        let first_line = self.command.lines().next().unwrap_or(&self.command);
        if UnicodeWidthStr::width(first_line) > max_cmd_width {
            truncate_ellipsis(first_line, max_cmd_width).into_owned()
        } else if self.command.contains('\n') {
            format!("{}...", first_line)
        } else {
            self.command.clone()
        }
    }

    /// Draw a single activity-row header (shared by collapsed + expanded).
    fn render_activity_header(&self, area: Rect, buf: &mut Buffer, theme: &Theme, y: u16) {
        let (status, status_color) = self.status_indicator(theme);
        let duration = self.duration_string();
        let progress = self.progress_suffix();
        let text_color = theme.text_color;
        let rail_color = theme.dim_color;

        let mut right_parts = Vec::new();
        if let Some(progress) = progress {
            right_parts.push(progress);
        }
        right_parts.push(status);
        if !duration.is_empty() {
            right_parts.push(duration);
        }
        let suffix = format!(" {}", right_parts.join(" "));
        let suffix_width = UnicodeWidthStr::width(suffix.as_str()) as u16;

        let caret = if self.collapsed { "▶" } else { "▼" };
        // Reserve space for caret, "$ ", and status suffix.
        let max_cmd_len = area
            .width
            .saturating_sub(suffix_width.saturating_add(6))
            .max(8) as usize;
        let cmd_display = self.command_display(max_cmd_len);
        let prefix = format!("{} $ {}", caret, cmd_display);
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
                if ch == '▶' || ch == '▼' || ch == '$' {
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

        // Draw status / duration / progress on the right
        let suffix_start = area.x + area.width.saturating_sub(suffix_width);
        let mut sx = suffix_start;
        for ch in suffix.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
            if sx >= area.x && sx + char_width <= area.x + area.width {
                if let Some(cell) = buf.cell_mut((sx, y)) {
                    cell.set_char(ch);
                    // Color status glyphs and echo animation characters.
                    if ch == '✓' || ch == '✗' || ch == '○' || ch == '●' || ch == '•' || ch == '·'
                    {
                        cell.set_fg(status_color);
                    } else {
                        cell.set_fg(theme.dim_color);
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

        // Soft separator dots between command and status
        let dots_start = area.x + prefix_width + 1;
        let dots_end = suffix_start.saturating_sub(1);
        for dx in dots_start..dots_end {
            if let Some(cell) = buf.cell_mut((dx, y)) {
                cell.set_char('·');
                cell.set_fg(rail_color);
            }
        }
    }

    /// Render collapsed state
    fn render_collapsed(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        self.render_activity_header(area, buf, theme, area.y);
    }

    /// Render expanded / live-tail state with clip awareness
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

        let (clip_top, _clip_bottom) = clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0));
        let rail_color = theme.dim_color;
        let content_color = theme.text_color;
        let content_cap = self.content_cap();

        // Use cached lines if available (should be populated by prior height() call)
        let content_width = area.width.saturating_sub(3) as usize;
        let fallback_lines;
        let lines: &[String] = if self.cached_width == area.width && !self.cached_lines.is_empty() {
            &self.cached_lines
        } else {
            fallback_lines = self.wrap_output(content_width);
            &fallback_lines
        };
        let total_lines = lines.len() as u16;
        let visible_lines = total_lines.min(content_cap);
        let needs_scrollbar = total_lines > content_cap;

        // Reserve space for scrollbar if needed
        let content_end_x = if needs_scrollbar {
            area.x + area.width - 2
        } else {
            area.x + area.width - 1
        };

        let mut render_y = area.y;

        // Activity header - only if not clipped
        if clip_top == 0 {
            self.render_activity_header(area, buf, theme, render_y);
            render_y += 1;
        }

        // Content area
        let content_start_offset = if clip_top > 0 { clip_top - 1 } else { 0 };
        let start_line = (self.scroll_offset + content_start_offset) as usize;

        let reserved_top = if clip_top == 0 { 1 } else { 0 };
        let content_lines_to_show = area.height.saturating_sub(reserved_top);

        // Only paint real content rows (plus the live-tail cap). Extra allocated
        // rows from a stale height must stay blank — no empty rail spam.
        let paint_rows = content_lines_to_show.min(visible_lines);
        for display_idx in 0..paint_rows {
            let line_idx = start_line + display_idx as usize;
            let y = render_y + display_idx;

            if y >= area.y + area.height {
                break;
            }

            // Light left rail instead of heavy dual borders
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('│');
                cell.set_fg(rail_color);
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
            }
        }

        // Render scrollbar if needed
        if needs_scrollbar {
            let header_lines = if clip_top == 0 { 1u16 } else { 0 };
            let scrollbar_height = area.height.saturating_sub(header_lines);

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
            // Keep fallback in lockstep with hybrid get_lines/render (header + live tail
            // or expanded content). The old progress-chrome formula inflated height on
            // every cache invalidation and made streaming rows thrash.
            let content_width = width.saturating_sub(3) as usize;
            let lines = self.wrap_output(content_width);
            let content_lines = (lines.len() as u16).min(self.content_cap());
            if content_lines == 0 {
                1
            } else {
                content_lines + 1
            }
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
            // Scroll wheel events - trust hit_test, don't re-check coordinates
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                ..
            }) => {
                if !self.collapsed {
                    self.scroll_down();
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..
            }) => {
                if !self.collapsed {
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
                            let new_offset = click_y
                                .saturating_mul(max_scroll)
                                .checked_div(track_height)
                                .unwrap_or(0);
                            self.scroll_offset = new_offset.min(max_scroll) as u16;
                            return EventResult::Consumed;
                        }
                    }

                    // Toggle behavior: collapsed=any click, expanded=header only
                    if self.collapsed {
                        self.collapsed = false;
                        // Live-tail vs expanded height depends on presentation mode.
                        self.cached_height = 0;
                        return EventResult::Action(BlockEvent::Expanded);
                    } else if internal_y == 0 {
                        self.collapsed = true;
                        self.scroll_offset = 0;
                        self.cached_height = 0;
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
                self.cached_height = 0;
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
            self.update_activity();
            true // Need redraw for activity echo or new output
        } else {
            had_pending // Only redraw if we flushed new content
        }
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }
}
