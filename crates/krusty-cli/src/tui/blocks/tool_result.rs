//! Tool result block - collapsible display for search results (grep/glob)
//!
//! Shows search/find results like thinking blocks:
//! - Collapsed: ▶ grep (pattern) N results
//! - Expanded: bordered box with results list, scrollable if > 15 results

use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{BlockEvent, ClipContext, EventResult, SimpleScrollable, StreamBlock};
use crate::tui::components::scrollbars::render_scrollbar;
use crate::tui::themes::Theme;
use crate::tui::utils::truncate_ellipsis;

/// Spinner frames
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Max visible result lines when expanded (before scrolling kicks in)
const MAX_VISIBLE_LINES: u16 = 15;

/// Max content width for wrapping (matches ThinkingBlock)
const MAX_CONTENT_WIDTH: usize = 76;

/// Minimum box width for readability
const MIN_BOX_WIDTH: usize = 20;

/// Tool result/status block for search results and compact generic tool status.
pub struct ToolResultBlock {
    /// Tool use ID for matching results
    tool_use_id: String,
    /// Tool name
    tool_name: String,
    /// Search pattern, path, action, or other short label
    pattern: String,
    /// Result/detail lines
    results: Vec<String>,
    /// Result count
    count: usize,
    /// Compact human summary for non-search tools
    summary: Option<String>,
    /// Whether result was an error
    is_error: bool,
    /// Whether collapsed
    collapsed: bool,
    /// Whether still running
    streaming: bool,
    /// Start time
    start_time: Instant,
    /// Duration when complete
    duration: Option<Duration>,
    /// Spinner frame
    spinner_idx: usize,
    /// Last spinner update
    last_spinner: Instant,
    /// Scroll offset for expanded view
    scroll_offset: u16,
}

impl ToolResultBlock {
    pub fn new(tool_use_id: String, tool_name: String, pattern: String) -> Self {
        let now = Instant::now();
        Self {
            tool_use_id,
            tool_name,
            pattern,
            results: Vec::new(),
            count: 0,
            summary: None,
            is_error: false,
            collapsed: true,
            streaming: true,
            start_time: now,
            duration: None,
            spinner_idx: 0,
            last_spinner: now,
            scroll_offset: 0,
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

    /// Mark this block as an error result.
    pub fn set_error(&mut self, is_error: bool) {
        self.is_error = is_error;
    }

    /// Parse and set results from tool output.
    pub fn set_results(&mut self, output: &str) {
        let view = crate::tui::tool_presentation::ToolOutputView::from_output(output);
        self.summary = view.summary;
        if let Some(is_error) = view.is_error {
            self.is_error = is_error;
        }

        let Ok(json) = serde_json::from_str::<serde_json::Value>(&view.render_output) else {
            self.set_generic_lines(&view.render_output);
            return;
        };

        let payload = json.get("data").unwrap_or(&json);
        match self.tool_name.as_str() {
            "glob" => self.set_glob_results(payload),
            "grep" => self.set_grep_results(payload),
            "list" => self.set_list_results(payload),
            _ => self.set_generic_json_results(payload, &json),
        }
    }

    fn set_glob_results(&mut self, payload: &serde_json::Value) {
        if let Some(matches) = payload.get("matches").and_then(|v| v.as_array()) {
            self.results = matches
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            self.count = payload
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or(self.results.len() as u64) as usize;
        }
    }

    fn set_grep_results(&mut self, payload: &serde_json::Value) {
        self.count = payload
            .get("total_matches")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        // Content mode: matches array with file/line/line_number
        if let Some(matches) = payload.get("matches").and_then(|v| v.as_array()) {
            self.results = matches
                .iter()
                .filter_map(|m| {
                    let file = m.get("file").and_then(|f| f.as_str())?;
                    let line_num = m.get("line_number").and_then(|n| n.as_u64());
                    let line = m.get("line").and_then(|l| l.as_str()).unwrap_or("");
                    if let Some(ln) = line_num {
                        Some(format!("{}:{}: {}", file, ln, line))
                    } else {
                        Some(format!("{}: {}", file, line))
                    }
                })
                .collect();
        }
        // files_with_matches mode: files array
        else if let Some(files) = payload.get("files").and_then(|v| v.as_array()) {
            self.results = files
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            self.count = self.results.len();
        }
    }

    fn set_list_results(&mut self, payload: &serde_json::Value) {
        self.results = payload
            .get("entries")
            .or_else(|| payload.get("output"))
            .and_then(|value| {
                value
                    .as_array()
                    .map(|entries| {
                        entries
                            .iter()
                            .filter_map(|entry| entry.as_str().map(ToString::to_string))
                            .collect::<Vec<_>>()
                    })
                    .or_else(|| {
                        value.as_str().map(|text| {
                            text.lines()
                                .filter(|line| !line.trim().is_empty())
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                        })
                    })
            })
            .unwrap_or_default();
        self.count = payload
            .get("total_entries")
            .or_else(|| payload.get("count"))
            .and_then(|v| v.as_u64())
            .unwrap_or(self.results.len() as u64) as usize;
    }

    fn set_generic_json_results(&mut self, payload: &serde_json::Value, full: &serde_json::Value) {
        if self.summary.is_none() {
            self.summary = payload
                .get("message")
                .or_else(|| payload.get("summary"))
                .or_else(|| payload.get("investigation_summary"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| {
                    full.get("error")
                        .and_then(|v| v.get("message"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                });
        }

        let mut lines = Vec::new();
        if let Some(summary) = &self.summary {
            lines.push(summary.clone());
        }
        for key in [
            "file_path",
            "path",
            "url",
            "title",
            "memory_id",
            "report_id",
            "task_id",
            "delegated_run_id",
        ] {
            if let Some(value) = payload.get(key).and_then(|v| v.as_str()) {
                lines.push(format!("{}: {}", key.replace('_', " "), value));
            }
        }
        for key in [
            "count",
            "result_count",
            "total_matches",
            "total_entries",
            "bytes_written",
            "line_count",
            "files_modified",
            "lines_added",
            "lines_removed",
        ] {
            if let Some(value) = payload.get(key).and_then(|v| v.as_u64()) {
                lines.push(format!("{}: {}", key.replace('_', " "), value));
            }
        }

        if lines.is_empty() {
            lines.push(crate::tui::utils::truncate_ellipsis(&full.to_string(), 220).into_owned());
        }
        self.count = lines.len();
        self.results = lines;
    }

    fn set_generic_lines(&mut self, output: &str) {
        let lines = output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(12)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if self.summary.is_none() {
            self.summary = lines.first().cloned();
        }
        self.count = lines.len();
        self.results = lines;
    }

    pub fn complete(&mut self) {
        self.streaming = false;
        self.duration = Some(self.start_time.elapsed());
    }

    pub fn toggle(&mut self) {
        // Don't allow expanding if there are no details to show.
        if self.collapsed && self.results.is_empty() {
            return;
        }
        self.collapsed = !self.collapsed;
        if self.collapsed {
            // Reset scroll when collapsing
            self.scroll_offset = 0;
        }
    }

    fn spinner_frame(&self) -> char {
        SPINNER[self.spinner_idx % SPINNER.len()]
    }

    /// Visible lines (capped at MAX_VISIBLE_LINES)
    fn visible_lines(&self) -> u16 {
        self.total_lines().min(MAX_VISIBLE_LINES)
    }

    /// Get scroll info for drag handling: (total_lines, visible_lines, scrollbar_height)
    /// Note: delegates to SimpleScrollable::simple_scroll_info
    pub fn get_scroll_info(&self) -> (u16, u16, u16) {
        self.simple_scroll_info()
    }

    /// Check if block has enough content to need a scrollbar
    /// Note: delegates to SimpleScrollable::needs_scrollbar
    pub fn has_scrollbar(&self) -> bool {
        SimpleScrollable::needs_scrollbar(self)
    }

    /// Calculate actual rendered box width for hit testing
    pub fn box_width(&self, available_width: u16) -> u16 {
        let longest_line = self
            .results
            .iter()
            .map(|l| UnicodeWidthStr::width(l.as_str()))
            .max()
            .unwrap_or(MIN_BOX_WIDTH);
        let header_text_len = UnicodeWidthStr::width(self.tool_name.as_str())
            + UnicodeWidthStr::width(self.pattern.as_str()).min(30)
            + 15;
        let box_inner_width = longest_line
            .max(header_text_len)
            .clamp(MIN_BOX_WIDTH, MAX_CONTENT_WIDTH);
        ((box_inner_width + 4) as u16).min(available_width)
    }

    fn is_search_style(&self) -> bool {
        matches!(self.tool_name.as_str(), "grep" | "glob" | "list")
    }

    fn compact_label(&self, pat_display: &str) -> String {
        if pat_display.is_empty() {
            self.tool_name.clone()
        } else {
            format!("{} ({})", self.tool_name, pat_display)
        }
    }

    /// Render collapsed state: single line like thinking block
    fn render_collapsed(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let y = area.y;
        let text_style = Style::default()
            .fg(theme.text_color)
            .add_modifier(Modifier::ITALIC);

        // Truncate pattern if needed
        let max_pat = 30;
        let pat_display = if UnicodeWidthStr::width(self.pattern.as_str()) > max_pat {
            truncate_ellipsis(&self.pattern, max_pat).into_owned()
        } else {
            self.pattern.clone()
        };
        let label = self.compact_label(&pat_display);

        let text = if self.streaming {
            format!("▶ {}... {}", label, self.spinner_frame())
        } else if self.is_search_style() {
            format!("▶ {} {} results", label, self.count)
        } else {
            let marker = if self.is_error { "✗" } else { "✓" };
            match self
                .summary
                .as_deref()
                .filter(|summary| !summary.trim().is_empty())
            {
                Some(summary) => {
                    format!("▶ {} {} {}", label, marker, truncate_ellipsis(summary, 72))
                }
                None => format!("▶ {} {}", label, marker),
            }
        };

        let text_len = UnicodeWidthStr::width(text.as_str());
        let mut x = area.x;
        for (i, ch) in text.chars().enumerate() {
            if x >= area.x + area.width {
                break;
            }
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(ch);
                if i == 0 || (self.streaming && x + char_width as u16 >= area.x + text_len as u16) {
                    cell.set_fg(theme.accent_color);
                } else {
                    cell.set_style(text_style);
                }
            }
            x += char_width as u16;
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
        if area.height < 1 || area.width < 20 {
            return;
        }

        let (clip_top, clip_bottom) = clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0));

        let border_color = theme.border_color;
        let header_style = Style::default()
            .fg(theme.text_color)
            .add_modifier(Modifier::ITALIC);

        let total_lines = self.total_lines();
        let visible_lines = self.visible_lines();
        let needs_scrollbar = self.needs_scrollbar();

        // Truncate pattern if needed
        let max_pat = 30;
        let pat_display = if UnicodeWidthStr::width(self.pattern.as_str()) > max_pat {
            truncate_ellipsis(&self.pattern, max_pat).into_owned()
        } else {
            self.pattern.clone()
        };

        // Calculate fitted box width from longest result (ThinkingBlock pattern)
        let longest_line = self
            .results
            .iter()
            .map(|l| UnicodeWidthStr::width(l.as_str()))
            .max()
            .unwrap_or(MIN_BOX_WIDTH);

        // Also consider header width
        let label = self.compact_label(&pat_display);
        let count_label = if self.is_search_style() {
            format!("{} results", self.count)
        } else {
            format!(
                "{} detail{}",
                self.results.len(),
                if self.results.len() == 1 { "" } else { "s" }
            )
        };
        let header_text = format!(" ▼ {} {} ", label, count_label);
        let header_width = UnicodeWidthStr::width(header_text.as_str());

        let box_inner_width = longest_line
            .max(header_width)
            .clamp(MIN_BOX_WIDTH, MAX_CONTENT_WIDTH);
        let box_width = ((box_inner_width + 4) as u16).min(area.width);

        // Reserve space for scrollbar if needed (1 char)
        let content_end_x = if needs_scrollbar {
            area.x + box_width - 2
        } else {
            area.x + box_width - 1
        };
        let right_x = area.x + box_width - 1;
        let content_width = (content_end_x - area.x - 2) as usize;

        let mut render_y = area.y;

        // Top border - only if not clipped
        if clip_top == 0 {
            let header = format!(" ▼ {} {} ", label, count_label);

            if let Some(cell) = buf.cell_mut((area.x, render_y)) {
                cell.set_char('╭');
                cell.set_fg(border_color);
            }

            if let Some(cell) = buf.cell_mut((area.x + 1, render_y)) {
                cell.set_char('─');
                cell.set_fg(border_color);
            }

            let mut x = area.x + 2;
            for ch in header.chars() {
                if x >= content_end_x {
                    break;
                }
                let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
                if let Some(cell) = buf.cell_mut((x, render_y)) {
                    cell.set_char(ch);
                    if ch == '▼' {
                        cell.set_fg(theme.accent_color);
                    } else {
                        cell.set_style(header_style);
                    }
                }
                x += char_width as u16;
            }

            for fx in x..content_end_x {
                if let Some(cell) = buf.cell_mut((fx, render_y)) {
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

            if let Some(cell) = buf.cell_mut((right_x, render_y)) {
                cell.set_char('╮');
                cell.set_fg(border_color);
            }

            render_y += 1;
        }

        // Content lines
        let content_start_offset = if clip_top > 0 { clip_top - 1 } else { 0 };
        let start_line = (self.scroll_offset + content_start_offset) as usize;
        let lines_to_render = if clip_top > 0 {
            area.height - if clip_bottom == 0 { 1 } else { 0 }
        } else {
            area.height.saturating_sub(2)
        };

        for display_idx in 0..lines_to_render {
            let line_idx = start_line + display_idx as usize;
            let y = render_y + display_idx;

            if y >= area.y + area.height {
                break;
            }
            if clip_bottom == 0 && y >= area.y + area.height - 1 {
                break;
            }

            // Left border
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }

            // Content
            if let Some(result) = self.results.get(line_idx) {
                let display = if UnicodeWidthStr::width(result.as_str()) > content_width {
                    let mut truncated = String::new();
                    let mut width = 0;
                    for ch in result.chars() {
                        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
                        if width + ch_width + 3 > content_width {
                            break;
                        }
                        truncated.push(ch);
                        width += ch_width;
                    }
                    format!("{}...", truncated)
                } else {
                    result.clone()
                };

                let mut cx = area.x + 2;
                for ch in display.chars() {
                    if cx >= content_end_x {
                        break;
                    }
                    let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
                    if let Some(cell) = buf.cell_mut((cx, y)) {
                        cell.set_char(ch);
                        cell.set_fg(theme.text_color);
                    }
                    cx += char_width as u16;
                }
            }

            // Right border
            if let Some(cell) = buf.cell_mut((right_x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }
        }

        // Bottom border - only if not clipped
        if clip_bottom == 0 && area.height > 1 {
            let bottom_y = area.y + area.height - 1;

            if let Some(cell) = buf.cell_mut((area.x, bottom_y)) {
                cell.set_char('╰');
                cell.set_fg(border_color);
            }
            for bx in (area.x + 1)..content_end_x {
                if let Some(cell) = buf.cell_mut((bx, bottom_y)) {
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
            if let Some(cell) = buf.cell_mut((right_x, bottom_y)) {
                cell.set_char('╯');
                cell.set_fg(border_color);
            }
        }

        // Render scrollbar if needed (only if we have content lines visible)
        if needs_scrollbar {
            // Calculate content area for scrollbar (exclude header/footer if visible)
            let header_lines = if clip_top == 0 { 1u16 } else { 0 };
            let footer_lines = if clip_bottom == 0 { 1u16 } else { 0 };
            let content_lines = area.height.saturating_sub(header_lines + footer_lines);

            if content_lines > 0 {
                let scrollbar_y = area.y + header_lines;
                let scrollbar_area = Rect::new(content_end_x, scrollbar_y, 1, content_lines);
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

    fn expanded_height(&self) -> u16 {
        // Header + visible results + bottom border
        let result_lines = self.visible_lines();
        (result_lines + 2).max(4)
    }
}

impl SimpleScrollable for ToolResultBlock {
    fn total_lines(&self) -> u16 {
        self.results.len() as u16
    }

    fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }

    fn set_scroll_offset(&mut self, offset: u16) {
        let max = self.max_scroll();
        self.scroll_offset = offset.min(max);
    }

    fn max_visible_lines(&self) -> u16 {
        MAX_VISIBLE_LINES
    }
}

impl StreamBlock for ToolResultBlock {
    fn height(&self, _width: u16, _theme: &Theme) -> u16 {
        if self.collapsed {
            1
        } else {
            self.expanded_height()
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
        if area.height == 0 || area.width < 20 {
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
                // Use actual rendered box width, not full area width
                let actual_width = self.box_width(area.width);
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + actual_width;

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
                    if !self.collapsed && self.needs_scrollbar() {
                        let scrollbar_x = area.x + actual_width - 2;

                        if *column == scrollbar_x && internal_y > 0 {
                            let total = self.total_lines() as usize;
                            let visible = self.visible_lines() as usize;
                            let max_scroll = total.saturating_sub(visible);
                            let track_height = visible;
                            let click_y = (internal_y - 1) as usize; // -1 for header
                            let new_offset = click_y
                                .saturating_mul(max_scroll)
                                .checked_div(track_height)
                                .unwrap_or(0);
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
            Event::Key(KeyEvent {
                code: KeyCode::Enter | KeyCode::Char(' '),
                ..
            }) => {
                let was_collapsed = self.collapsed;
                self.toggle();
                if was_collapsed != self.collapsed {
                    if self.collapsed {
                        EventResult::Action(BlockEvent::Collapsed)
                    } else {
                        EventResult::Action(BlockEvent::Expanded)
                    }
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn tick(&mut self) -> bool {
        if self.streaming && self.last_spinner.elapsed() >= Duration::from_millis(80) {
            self.spinner_idx = (self.spinner_idx + 1) % SPINNER.len();
            self.last_spinner = Instant::now();
            true
        } else {
            false
        }
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }

    fn get_text_content(&self) -> Option<String> {
        let label = self.compact_label(&self.pattern);
        let header = if self.is_search_style() {
            format!("{} ({} results)", label, self.count)
        } else if let Some(summary) = &self.summary {
            format!("{}: {}", label, summary)
        } else {
            label
        };

        // When collapsed, only return header (matches rendered height of 1)
        if self.collapsed {
            return Some(header);
        }

        let mut result = String::new();
        result.push_str(&header);
        result.push('\n');

        // Results
        if !self.results.is_empty() {
            for line in &self.results {
                result.push_str(line);
                result.push('\n');
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result.trim_end().to_string())
        }
    }
}
