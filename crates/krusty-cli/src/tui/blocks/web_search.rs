//! Web search block - collapsible display for web search results
//!
//! Shows web search results like tool result blocks:
//! - Collapsed: > Search "query"... [spinner] or > Search "query" N results
//! - Expanded: bordered box with title + URL pairs, scrollable

use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{BlockEvent, ClipContext, EventResult, SimpleScrollable, StreamBlock};
use crate::ai::types::WebSearchResult;
use crate::tui::components::scrollbars::render_scrollbar;
use crate::tui::themes::Theme;

/// Spinner frames
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Max visible result lines when expanded
const MAX_VISIBLE_LINES: u16 = 15;

/// Max content width
const MAX_CONTENT_WIDTH: usize = 76;

/// Minimum box width
const MIN_BOX_WIDTH: usize = 20;

/// Web search result block
pub struct WebSearchBlock {
    /// Tool use ID for matching results
    tool_use_id: String,
    /// Search query (extracted from tool input)
    query: String,
    /// Search results
    results: Vec<WebSearchResult>,
    /// Whether collapsed
    collapsed: bool,
    /// Whether still searching
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

impl WebSearchBlock {
    pub fn new(tool_use_id: String, query: String) -> Self {
        let now = Instant::now();
        Self {
            tool_use_id,
            query,
            results: Vec::new(),
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

    /// Set results from API response
    pub fn set_results(&mut self, results: Vec<WebSearchResult>) {
        self.results = results;
        self.streaming = false;
        self.duration = Some(self.start_time.elapsed());
    }

    pub fn complete(&mut self) {
        self.streaming = false;
        self.duration = Some(self.start_time.elapsed());
    }

    pub fn toggle(&mut self) {
        // Don't allow expanding if no results
        if self.collapsed && self.results.is_empty() {
            return;
        }
        self.collapsed = !self.collapsed;
        if self.collapsed {
            self.scroll_offset = 0;
        }
    }

    fn spinner_frame(&self) -> char {
        SPINNER[self.spinner_idx % SPINNER.len()]
    }

    /// Visible lines (capped)
    fn visible_lines(&self) -> u16 {
        self.total_lines().min(MAX_VISIBLE_LINES)
    }

    /// Get scroll info for drag handling
    /// Note: delegates to SimpleScrollable::simple_scroll_info
    pub fn get_scroll_info(&self) -> (u16, u16, u16) {
        self.simple_scroll_info()
    }

    /// Calculate box width
    pub fn box_width(&self, available_width: u16) -> u16 {
        let longest_line = self
            .results
            .iter()
            .flat_map(|r| [r.title.width(), r.url.width() + 2])
            .max()
            .unwrap_or(MIN_BOX_WIDTH);
        let header_text_len = "Search ".len() + self.query.width().min(30) + 15;
        let box_inner_width = longest_line
            .max(header_text_len)
            .clamp(MIN_BOX_WIDTH, MAX_CONTENT_WIDTH);
        ((box_inner_width + 4) as u16).min(available_width)
    }

    /// Render collapsed state
    fn render_collapsed(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let y = area.y;
        let text_style = Style::default()
            .fg(theme.text_color)
            .add_modifier(Modifier::ITALIC);

        // Truncate query if needed
        let max_query = 40;
        let query_display = if self.query.len() > max_query {
            format!("{}...", &self.query[..max_query.saturating_sub(3)])
        } else {
            self.query.clone()
        };

        let text = if self.streaming {
            format!("▶ Search \"{}\"... {}", query_display, self.spinner_frame())
        } else {
            let duration_str = self
                .duration
                .map(|d| format!(" {:.1}s", d.as_secs_f64()))
                .unwrap_or_default();
            format!(
                "▶ Search \"{}\" {} results{}",
                query_display,
                self.results.len(),
                duration_str
            )
        };

        let text_char_count = text.chars().count();
        let mut x = area.x;
        for (i, ch) in text.chars().enumerate() {
            if x >= area.x + area.width {
                break;
            }
            let char_width = ch.width().unwrap_or(0);
            if char_width == 0 {
                continue;
            }
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(ch);
                if i == 0 || (self.streaming && i == text_char_count - 1) {
                    cell.set_fg(theme.accent_color);
                } else {
                    cell.set_style(text_style);
                }
            }
            x += char_width as u16;
        }
    }

    /// Render expanded state
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
        let url_style = Style::default().fg(theme.dim_color);

        let total_lines = self.total_lines();
        let visible_lines = self.visible_lines();
        let needs_scrollbar = self.needs_scrollbar();

        // Truncate query if needed
        let max_query = 30;
        let query_display = if self.query.len() > max_query {
            format!("{}...", &self.query[..max_query.saturating_sub(3)])
        } else {
            self.query.clone()
        };

        // Calculate box width
        let longest_line = self
            .results
            .iter()
            .flat_map(|r| [r.title.width(), r.url.width() + 2])
            .max()
            .unwrap_or(MIN_BOX_WIDTH);

        let header_text = format!(
            " ▼ Search \"{}\" {} results ",
            query_display,
            self.results.len()
        );
        let header_width = header_text.width();

        let box_inner_width = longest_line
            .max(header_width)
            .clamp(MIN_BOX_WIDTH, MAX_CONTENT_WIDTH);
        let box_width = ((box_inner_width + 4) as u16).min(area.width);

        let content_end_x = if needs_scrollbar {
            area.x + box_width - 2
        } else {
            area.x + box_width - 1
        };
        let right_x = area.x + box_width - 1;
        let content_width = (content_end_x - area.x - 2) as usize;

        let mut render_y = area.y;

        // Top border
        if clip_top == 0 {
            let header = format!(
                " ▼ Search \"{}\" {} results ",
                query_display,
                self.results.len()
            );

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
                let char_width = ch.width().unwrap_or(0);
                if char_width == 0 {
                    continue;
                }
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

        // Content lines (title + url pairs)
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

            // Content: each result takes 2 lines (title, then url)
            let result_idx = line_idx / 2;
            let is_url_line = line_idx % 2 == 1;

            if let Some(result) = self.results.get(result_idx) {
                let (text, style) = if is_url_line {
                    (format!("  {}", result.url), url_style)
                } else {
                    (result.title.clone(), Style::default().fg(theme.text_color))
                };

                let display = if text.width() > content_width {
                    let mut truncated = String::new();
                    let mut w = 0;
                    for ch in text.chars() {
                        let cw = ch.width().unwrap_or(0);
                        if w + cw + 3 > content_width {
                            break;
                        }
                        truncated.push(ch);
                        w += cw;
                    }
                    truncated.push_str("...");
                    truncated
                } else {
                    text
                };

                let mut cx = area.x + 2;
                for ch in display.chars() {
                    if cx >= content_end_x {
                        break;
                    }
                    let char_width = ch.width().unwrap_or(0);
                    if char_width == 0 {
                        continue;
                    }
                    if let Some(cell) = buf.cell_mut((cx, y)) {
                        cell.set_char(ch);
                        cell.set_style(style);
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

        // Bottom border
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

        // Scrollbar
        if needs_scrollbar {
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
        let result_lines = self.visible_lines();
        (result_lines + 2).max(4)
    }
}

impl SimpleScrollable for WebSearchBlock {
    fn total_lines(&self) -> u16 {
        // Each result has 2 lines: title + url
        (self.results.len() * 2) as u16
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

impl StreamBlock for WebSearchBlock {
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
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column,
                row,
                ..
            }) => {
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
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                ..
            }) => {
                let actual_width = self.box_width(area.width);
                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + actual_width;

                if in_area {
                    let internal_y = (*row - area.y) + clip_top;

                    if !self.collapsed && self.needs_scrollbar() {
                        let scrollbar_x = area.x + actual_width - 2;

                        if *column == scrollbar_x && internal_y > 0 {
                            let total = self.total_lines() as usize;
                            let visible = self.visible_lines() as usize;
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

                    if self.collapsed {
                        self.toggle();
                        return EventResult::Action(BlockEvent::Expanded);
                    } else if internal_y == 0 {
                        self.toggle();
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
        let header = format!("Search \"{}\" ({} results)", self.query, self.results.len());

        if self.collapsed {
            return Some(header);
        }

        let content: String = self
            .results
            .iter()
            .map(|r| format!("{}\n  {}", r.title, r.url))
            .collect::<Vec<_>>()
            .join("\n");

        Some(format!("{}\n{}", header, content).trim_end().to_string())
    }
}
