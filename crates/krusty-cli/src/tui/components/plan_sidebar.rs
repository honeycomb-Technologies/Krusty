//! Plan sidebar component
//!
//! Renders a collapsible sidebar showing the current plan's phases and tasks.
//! Uses caching to avoid rebuilding content every frame.

use std::hash::{Hash, Hasher};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Widget},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Result of rendering the plan sidebar
pub struct PlanSidebarRenderResult {
    /// Scrollbar area (if scrolling is needed)
    pub scrollbar_area: Option<Rect>,
}

use super::scrollbars::render_scrollbar;
use crate::plan::PlanFile;
use crate::tui::themes::Theme;

/// Sidebar width when fully expanded
pub const SIDEBAR_WIDTH: u16 = 76;

/// Minimum terminal width to show sidebar
pub const MIN_TERMINAL_WIDTH: u16 = 140;

/// Horizontal padding inside sidebar content area
const PAD_X: u16 = 2;

/// Number of blank lines between phases
const PHASE_GAP_LINES: usize = 2;

/// Indentation for wrapped task continuation lines (aligns with description start)
const TASK_CONTINUATION_INDENT: &str = "    ";

/// Plan sidebar state with content caching
#[derive(Debug, Clone, Default)]
pub struct PlanSidebarState {
    /// Whether sidebar is visible
    pub visible: bool,
    /// Current animated width (0 to SIDEBAR_WIDTH)
    pub current_width: u16,
    /// Target width (0 or SIDEBAR_WIDTH)
    pub target_width: u16,
    /// Scroll offset for content
    pub scroll_offset: usize,
    /// Total content lines (calculated during render)
    pub total_lines: usize,
    /// Pending plan clear after collapse animation completes
    pending_clear: bool,

    // === Caching fields ===
    /// Cached rendered lines (avoids rebuilding every frame)
    cached_lines: Vec<Line<'static>>,
    /// Hash of plan content when cache was built
    cached_plan_hash: u64,
    /// Width when cache was built
    cached_width: u16,
}

impl PlanSidebarState {
    /// Toggle sidebar visibility
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        self.target_width = if self.visible { SIDEBAR_WIDTH } else { 0 };
        // Reset scroll when toggling
        if !self.visible {
            self.scroll_offset = 0;
        }
    }

    /// Start graceful collapse animation (for plan completion)
    /// The plan should be cleared after animation completes via should_clear_plan()
    pub fn start_collapse(&mut self) {
        self.target_width = 0;
        self.pending_clear = true;
    }

    /// Check if plan should be cleared (collapse animation complete)
    /// Returns true once and resets the pending flag
    pub fn should_clear_plan(&mut self) -> bool {
        if self.pending_clear && self.current_width == 0 {
            self.pending_clear = false;
            self.visible = false;
            self.scroll_offset = 0;
            true
        } else {
            false
        }
    }

    /// Reset sidebar to initial state
    pub fn reset(&mut self) {
        self.visible = false;
        self.current_width = 0;
        self.target_width = 0;
        self.scroll_offset = 0;
        self.total_lines = 0;
        self.pending_clear = false;
        // Clear cache
        self.cached_lines.clear();
        self.cached_plan_hash = 0;
        self.cached_width = 0;
    }

    /// Animate width towards target
    /// Returns true if animation is still in progress
    pub fn tick(&mut self) -> bool {
        if self.current_width == self.target_width {
            return false;
        }

        // Adaptive animation speed: faster when far from target
        let remaining = (self.target_width as i16 - self.current_width as i16).unsigned_abs();
        let step = (remaining / 5).clamp(2, 8);

        if self.current_width < self.target_width {
            self.current_width = (self.current_width + step).min(self.target_width);
        } else {
            self.current_width = self.current_width.saturating_sub(step);
            if self.current_width < step {
                self.current_width = self.target_width;
            }
        }

        self.current_width != self.target_width
    }

    /// Get current width for layout calculations
    pub fn width(&self) -> u16 {
        self.current_width
    }

    /// Check if animation is currently in progress
    pub fn is_animating(&self) -> bool {
        self.current_width != self.target_width
    }

    /// Scroll up by one line
    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Scroll down by one line
    pub fn scroll_down(&mut self, visible_height: usize) {
        let max_offset = self.total_lines.saturating_sub(visible_height);
        if self.scroll_offset < max_offset {
            self.scroll_offset += 1;
        }
    }

    /// Scroll up by a page
    pub fn page_up(&mut self, visible_height: usize) {
        self.scroll_offset = self
            .scroll_offset
            .saturating_sub(visible_height.saturating_sub(2));
    }

    /// Scroll down by a page
    pub fn page_down(&mut self, visible_height: usize) {
        let max_offset = self.total_lines.saturating_sub(visible_height);
        self.scroll_offset =
            (self.scroll_offset + visible_height.saturating_sub(2)).min(max_offset);
    }

    /// Handle scrollbar click - jump to position
    pub fn handle_scrollbar_click(&mut self, click_y: u16, area: Rect) {
        if self.total_lines == 0 {
            return;
        }

        let relative_y = click_y.saturating_sub(area.y) as f32;
        let height = area.height as f32;
        let visible_height = area.height as usize;
        let max_offset = self.total_lines.saturating_sub(visible_height);

        if max_offset == 0 {
            return;
        }

        let new_offset = ((relative_y / height) * max_offset as f32).round() as usize;
        self.scroll_offset = new_offset.min(max_offset);
    }
}

/// Render the plan sidebar
/// Returns render result with scrollbar area for hit testing
/// Uses caching to avoid rebuilding content every frame
pub fn render_plan_sidebar(
    buf: &mut Buffer,
    area: Rect,
    plan: &PlanFile,
    theme: &Theme,
    state: &mut PlanSidebarState,
) -> PlanSidebarRenderResult {
    if area.width < 10 || area.height < 5 {
        return PlanSidebarRenderResult {
            scrollbar_area: None,
        };
    }

    // Draw clean border (no title)
    let block = Block::default()
        .borders(Borders::LEFT | Borders::TOP | Borders::BOTTOM)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_color))
        .style(Style::default().bg(theme.bg_color));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.width < 5 || inner.height < 3 {
        return PlanSidebarRenderResult {
            scrollbar_area: None,
        };
    }

    let visible_height = inner.height as usize;

    // Always reserve scrollbar column to avoid reflow jitter when content grows
    let content_area_width = inner.width.saturating_sub(1); // 1 for scrollbar
    let wrap_width = content_area_width.saturating_sub(PAD_X * 2) as usize;

    // Guard against extremely narrow widths
    if wrap_width < 10 {
        return PlanSidebarRenderResult {
            scrollbar_area: None,
        };
    }

    // Check if we need to rebuild the cache
    let plan_hash = hash_plan(plan);
    let cache_valid =
        state.cached_plan_hash == plan_hash && state.cached_width == wrap_width as u16;

    if !cache_valid {
        // Rebuild cached lines with proper text wrapping
        state.cached_lines.clear();

        // Plan title header (wrapped, bold, with separator)
        let title_style = Style::default()
            .fg(theme.title_color)
            .add_modifier(Modifier::BOLD);

        for wrapped_line in wrap_text(&plan.title, wrap_width) {
            state
                .cached_lines
                .push(Line::from(Span::styled(wrapped_line, title_style)));
        }

        // Separator line after title
        let separator = "─".repeat(wrap_width.min(40));
        state.cached_lines.push(Line::from(Span::styled(
            separator,
            Style::default().fg(theme.border_color),
        )));

        // Blank line after separator
        state.cached_lines.push(Line::from(""));

        // Task prefix width: "  " + checkbox + " " = 4 columns
        let task_prefix_width = 4usize;
        let task_wrap_width = wrap_width.saturating_sub(task_prefix_width).max(10);

        for (i, phase) in plan.phases.iter().enumerate() {
            // Phase header (wrapped)
            let phase_title = format!("Phase {}: {}", phase.number, phase.name);
            let header_style = Style::default()
                .fg(theme.accent_color)
                .add_modifier(Modifier::BOLD);

            for wrapped_line in wrap_text(&phase_title, wrap_width) {
                state
                    .cached_lines
                    .push(Line::from(Span::styled(wrapped_line, header_style)));
            }

            // Tasks
            for task in &phase.tasks {
                let checkbox = if task.completed { "✓" } else { "○" };
                let checkbox_color = if task.completed {
                    theme.success_color
                } else {
                    theme.dim_color
                };
                let task_style = if task.completed {
                    Style::default().fg(theme.dim_color)
                } else {
                    Style::default().fg(theme.text_color)
                };

                let wrapped_desc = wrap_text(&task.description, task_wrap_width);

                for (line_idx, desc_line) in wrapped_desc.into_iter().enumerate() {
                    if line_idx == 0 {
                        // First line: checkbox prefix
                        state.cached_lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(checkbox, Style::default().fg(checkbox_color)),
                            Span::raw(" "),
                            Span::styled(desc_line, task_style),
                        ]));
                    } else {
                        // Continuation lines: indented to align with description
                        state.cached_lines.push(Line::from(vec![
                            Span::raw(TASK_CONTINUATION_INDENT),
                            Span::styled(desc_line, task_style),
                        ]));
                    }
                }
            }

            // Space between phases (not after last)
            if i < plan.phases.len() - 1 {
                for _ in 0..PHASE_GAP_LINES {
                    state.cached_lines.push(Line::from(""));
                }
            }
        }

        state.cached_plan_hash = plan_hash;
        state.cached_width = wrap_width as u16;
    }

    // Total lines is now based on actual wrapped content
    state.total_lines = state.cached_lines.len();

    // Clamp scroll offset
    let max_offset = state.total_lines.saturating_sub(visible_height);
    if state.scroll_offset > max_offset {
        state.scroll_offset = max_offset;
    }

    // Render visible lines from cache using slice (with horizontal padding)
    let start = state.scroll_offset;
    let end = (start + visible_height).min(state.cached_lines.len());
    let mut y = inner.y;
    let content_x = inner.x + PAD_X;
    // Use explicit area boundary to prevent any possibility of overflow
    // The right boundary is inner.x + content_area_width (excludes scrollbar column)
    let area_max_x = inner.x + content_area_width;

    for line in &state.cached_lines[start..end] {
        render_line(buf, content_x, y, area_max_x, line);
        y += 1;
    }

    // Render scrollbar if needed
    let scrollbar_area = if state.total_lines > visible_height {
        let scrollbar_rect = Rect::new(inner.x + inner.width - 1, inner.y, 1, inner.height);
        render_scrollbar(
            buf,
            scrollbar_rect,
            state.scroll_offset,
            state.total_lines,
            visible_height,
            theme.accent_color,
            theme.scrollbar_bg_color,
        );
        Some(scrollbar_rect)
    } else {
        None
    };

    PlanSidebarRenderResult { scrollbar_area }
}

/// Render a line directly to the buffer without cloning
/// Uses explicit max_x boundary to prevent any overflow outside the intended area
fn render_line(buf: &mut Buffer, x: u16, y: u16, max_x: u16, line: &Line) {
    let mut cx = x;

    for span in &line.spans {
        for ch in span.content.chars() {
            if cx >= max_x {
                return;
            }
            let char_width = ch.width().unwrap_or(1) as u16;
            if cx + char_width > max_x {
                return;
            }
            if let Some(cell) = buf.cell_mut((cx, y)) {
                cell.set_char(ch);
                cell.set_style(span.style);
            }
            cx += char_width;
        }
    }
}

/// Compute a hash of the plan content for cache invalidation
fn hash_plan(plan: &PlanFile) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    // Hash title and phase count and each phase's content
    plan.title.hash(&mut hasher);
    plan.phases.len().hash(&mut hasher);
    for phase in &plan.phases {
        phase.number.hash(&mut hasher);
        phase.name.hash(&mut hasher);
        phase.tasks.len().hash(&mut hasher);
        for task in &phase.tasks {
            task.description.hash(&mut hasher);
            task.completed.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Word-wrap text to fit within max_width display columns
/// Returns a vector of wrapped lines
fn wrap_text(s: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0usize;

    for word in s.split_whitespace() {
        let word_width = word.width();

        if word_width > max_width {
            // Word is longer than max_width, need to hard-break it
            if !current_line.is_empty() {
                lines.push(std::mem::take(&mut current_line));
            }

            // Break the long word character by character
            let mut word_part = String::new();
            let mut part_width = 0usize;

            for c in word.chars() {
                let char_width = c.width().unwrap_or(1);
                if part_width + char_width > max_width {
                    if !word_part.is_empty() {
                        lines.push(std::mem::take(&mut word_part));
                    }
                    part_width = 0;
                }
                word_part.push(c);
                part_width += char_width;
            }

            // Remaining part becomes current line
            current_line = word_part;
            current_width = part_width;
        } else if current_width == 0 {
            // First word on line
            current_line = word.to_string();
            current_width = word_width;
        } else if current_width + 1 + word_width <= max_width {
            // Word fits on current line with space
            current_line.push(' ');
            current_line.push_str(word);
            current_width += 1 + word_width;
        } else {
            // Word doesn't fit, start new line
            lines.push(std::mem::take(&mut current_line));
            current_line = word.to_string();
            current_width = word_width;
        }
    }

    // Don't forget the last line
    if !current_line.is_empty() {
        lines.push(current_line);
    }

    // Ensure at least one line (for empty strings)
    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}
