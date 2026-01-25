//! Edit block core functionality
//!
//! Contains EditBlock struct, state management, diff computation, and event handling.

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{buffer::Buffer, layout::Rect};
use std::time::Instant;
use unicode_width::UnicodeWidthChar;

use super::{DiffLine, DiffMode, CONTEXT_LINES, MAX_VISIBLE_LINES, SYMBOL_TOGGLE_INTERVAL};
use crate::tui::blocks::{BlockEvent, ClipContext, EventResult, StreamBlock};
use crate::tui::themes::Theme;

/// Edit block showing file diff
pub struct EditBlock {
    /// Tool use ID for state persistence
    pub(super) tool_use_id: Option<String>,
    /// File path being edited
    pub(super) file_path: String,
    /// Original text
    pub(super) old_string: String,
    /// New text
    pub(super) new_string: String,
    /// Starting line number in file
    pub(super) start_line: usize,
    /// Computed diff lines
    pub(super) diff_lines: Vec<DiffLine>,
    /// Pre-computed side-by-side indices (left_idx, right_idx) into diff_lines
    pub(super) sbs_pairs: Vec<(Option<usize>, Option<usize>)>,
    /// Scroll offset
    pub(super) scroll_offset: u16,
    /// Current display mode (synced from global)
    pub(super) diff_mode: DiffMode,
    /// Whether the edit is still in progress (for animation)
    pub(super) streaming: bool,
    /// Whether the block is collapsed (shows only header)
    pub(super) collapsed: bool,
    /// Animation: frame counter for ±/∓ toggle
    pub(super) symbol_frame: u8,
    /// Last animation update time
    pub(super) last_animation: Instant,
    /// Cached height (computed once when diff changes)
    pub(super) cached_height: u16,
}

impl EditBlock {
    /// Create a pending edit block (collapsed, no diff yet)
    pub fn new_pending(file_path: String) -> Self {
        let now = Instant::now();
        Self {
            tool_use_id: None,
            file_path,
            old_string: String::new(),
            new_string: String::new(),
            start_line: 1,
            diff_lines: Vec::new(),
            sbs_pairs: Vec::new(),
            scroll_offset: 0,
            diff_mode: DiffMode::default(),
            streaming: true,
            collapsed: true,
            symbol_frame: 0,
            last_animation: now,
            cached_height: 1, // Collapsed height
        }
    }

    /// Set the tool use ID (for state persistence)
    pub fn set_tool_use_id(&mut self, id: String) {
        self.tool_use_id = Some(id);
    }

    /// Set collapsed state directly (for session restoration)
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    /// Set scroll offset (for session restoration)
    pub fn set_scroll_offset(&mut self, offset: u16) {
        self.scroll_offset = offset;
    }

    /// Set the diff data (called when tool arguments are parsed)
    pub fn set_diff_data(
        &mut self,
        file_path: String,
        old_string: String,
        new_string: String,
        start_line: usize,
    ) {
        self.file_path = file_path;
        self.old_string = old_string;
        self.new_string = new_string;
        self.start_line = start_line;
        self.compute_diff();
        self.compute_sbs_pairs();
        let content_lines = self.visible_lines();
        self.cached_height = (content_lines + 2).max(4);
    }

    /// Pre-compute side-by-side pairing (indices into diff_lines)
    /// Groups consecutive removed/added lines together on the same row
    fn compute_sbs_pairs(&mut self) {
        self.sbs_pairs.clear();

        let mut i = 0;
        while i < self.diff_lines.len() {
            match &self.diff_lines[i] {
                DiffLine::Context { .. } => {
                    // Context lines appear on both sides
                    self.sbs_pairs.push((Some(i), Some(i)));
                    i += 1;
                }
                DiffLine::Removed { .. } => {
                    // Collect consecutive removed lines
                    let mut removed_indices = vec![i];
                    i += 1;
                    while i < self.diff_lines.len() {
                        if matches!(&self.diff_lines[i], DiffLine::Removed { .. }) {
                            removed_indices.push(i);
                            i += 1;
                        } else {
                            break;
                        }
                    }

                    // Collect consecutive added lines that follow
                    let mut added_indices = Vec::new();
                    while i < self.diff_lines.len() {
                        if matches!(&self.diff_lines[i], DiffLine::Added { .. }) {
                            added_indices.push(i);
                            i += 1;
                        } else {
                            break;
                        }
                    }

                    // Pair up removed and added lines on the same row
                    let max_len = removed_indices.len().max(added_indices.len());
                    for j in 0..max_len {
                        let left = removed_indices.get(j).copied();
                        let right = added_indices.get(j).copied();
                        self.sbs_pairs.push((left, right));
                    }
                }
                DiffLine::Added { .. } => {
                    // Added lines without preceding removed lines
                    self.sbs_pairs.push((None, Some(i)));
                    i += 1;
                }
            }
        }
    }

    /// Mark the edit as complete (stops animation and expands)
    pub fn complete(&mut self) {
        self.streaming = false;
        self.collapsed = false;
    }

    /// Check if this is a pending block (no diff data yet)
    pub fn is_pending(&self) -> bool {
        self.diff_lines.is_empty() && self.old_string.is_empty() && self.new_string.is_empty()
    }

    /// Set the diff display mode (called when global mode changes)
    pub fn set_diff_mode(&mut self, mode: DiffMode) {
        self.diff_mode = mode;
    }

    /// Get animated diff symbol based on current state
    pub(super) fn get_symbol(&self) -> &'static str {
        if !self.streaming {
            return "✓";
        }
        match self.symbol_frame % 2 {
            0 => "±",
            _ => "∓",
        }
    }

    /// Update animation state, returns true if needs redraw
    fn tick_animation(&mut self) -> bool {
        if !self.streaming {
            return false;
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_animation);

        if elapsed >= SYMBOL_TOGGLE_INTERVAL {
            self.symbol_frame = (self.symbol_frame + 1) % 2;
            self.last_animation = now;
            return true;
        }
        false
    }

    /// Compute the diff between old and new strings
    fn compute_diff(&mut self) {
        let old_lines: Vec<&str> = self.old_string.lines().collect();
        let new_lines: Vec<&str> = self.new_string.lines().collect();

        self.diff_lines.clear();

        let lcs = self.compute_lcs(&old_lines, &new_lines);

        let mut old_idx = 0;
        let mut new_idx = 0;
        // Track line numbers separately for old and new files
        let mut old_line_num = self.start_line;
        let mut new_line_num = self.start_line;

        for (oi, ni) in lcs {
            // Removed lines from old file (before this LCS match)
            while old_idx < oi {
                self.diff_lines.push(DiffLine::Removed {
                    line_num: old_line_num,
                    content: old_lines[old_idx].to_string(),
                });
                old_idx += 1;
                old_line_num += 1;
            }

            // Added lines from new file (before this LCS match)
            while new_idx < ni {
                self.diff_lines.push(DiffLine::Added {
                    line_num: new_line_num,
                    content: new_lines[new_idx].to_string(),
                });
                new_idx += 1;
                new_line_num += 1;
            }

            // Context line (matching line in both files)
            self.diff_lines.push(DiffLine::Context {
                line_num: old_line_num,
                content: old_lines[oi].to_string(),
            });
            old_idx = oi + 1;
            new_idx = ni + 1;
            old_line_num += 1;
            new_line_num += 1;
        }

        // Remaining removed lines from old file
        while old_idx < old_lines.len() {
            self.diff_lines.push(DiffLine::Removed {
                line_num: old_line_num,
                content: old_lines[old_idx].to_string(),
            });
            old_idx += 1;
            old_line_num += 1;
        }

        // Remaining added lines from new file
        while new_idx < new_lines.len() {
            self.diff_lines.push(DiffLine::Added {
                line_num: new_line_num,
                content: new_lines[new_idx].to_string(),
            });
            new_idx += 1;
            new_line_num += 1;
        }

        self.filter_with_context();
    }

    /// Compute longest common subsequence indices using optimized hybrid approach
    fn compute_lcs<'a>(&self, old: &[&'a str], new: &[&'a str]) -> Vec<(usize, usize)> {
        let m = old.len();
        let n = new.len();

        if m == 0 || n == 0 {
            return Vec::new();
        }

        // Find common prefix
        let mut prefix_len = 0;
        while prefix_len < m && prefix_len < n && old[prefix_len] == new[prefix_len] {
            prefix_len += 1;
        }

        // Find common suffix
        let mut suffix_len = 0;
        while suffix_len < (m - prefix_len)
            && suffix_len < (n - prefix_len)
            && old[m - 1 - suffix_len] == new[n - 1 - suffix_len]
        {
            suffix_len += 1;
        }

        let mut result: Vec<(usize, usize)> = (0..prefix_len).map(|i| (i, i)).collect();

        let old_mid_start = prefix_len;
        let old_mid_end = m - suffix_len;
        let new_mid_start = prefix_len;
        let new_mid_end = n - suffix_len;

        let old_mid_len = old_mid_end - old_mid_start;
        let new_mid_len = new_mid_end - new_mid_start;

        if old_mid_len > 0 && new_mid_len > 0 && old_mid_len < 100 && new_mid_len < 100 {
            let old_mid = &old[old_mid_start..old_mid_end];
            let new_mid = &new[new_mid_start..new_mid_end];

            let mid_lcs = self.compute_lcs_middle(old_mid, new_mid);

            for (oi, ni) in mid_lcs {
                result.push((old_mid_start + oi, new_mid_start + ni));
            }
        }

        for i in 0..suffix_len {
            result.push((m - suffix_len + i, n - suffix_len + i));
        }

        result
    }

    /// LCS for middle section only
    fn compute_lcs_middle<'a>(&self, old: &[&'a str], new: &[&'a str]) -> Vec<(usize, usize)> {
        let m = old.len();
        let n = new.len();

        if m == 0 || n == 0 {
            return Vec::new();
        }

        let mut dp = vec![vec![0usize; n + 1]; m + 1];
        for i in 1..=m {
            for j in 1..=n {
                if old[i - 1] == new[j - 1] {
                    dp[i][j] = dp[i - 1][j - 1] + 1;
                } else {
                    dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
                }
            }
        }

        let mut result = Vec::new();
        let mut i = m;
        let mut j = n;
        while i > 0 && j > 0 {
            if old[i - 1] == new[j - 1] {
                result.push((i - 1, j - 1));
                i -= 1;
                j -= 1;
            } else if dp[i - 1][j] > dp[i][j - 1] {
                i -= 1;
            } else {
                j -= 1;
            }
        }

        result.reverse();
        result
    }

    /// Filter diff to show only changes with surrounding context
    fn filter_with_context(&mut self) {
        if self.diff_lines.is_empty() {
            return;
        }

        let mut visible = vec![false; self.diff_lines.len()];

        for (i, line) in self.diff_lines.iter().enumerate() {
            match line {
                DiffLine::Removed { .. } | DiffLine::Added { .. } => {
                    let start = i.saturating_sub(CONTEXT_LINES);
                    let end = (i + CONTEXT_LINES + 1).min(self.diff_lines.len());
                    for vis in visible.iter_mut().take(end).skip(start) {
                        *vis = true;
                    }
                }
                _ => {}
            }
        }

        self.diff_lines = self
            .diff_lines
            .iter()
            .enumerate()
            .filter(|(i, _)| visible[*i])
            .map(|(_, line)| line.clone())
            .collect();
    }

    /// Get short path for display
    pub(super) fn short_path(&self) -> &str {
        self.file_path.rsplit('/').next().unwrap_or(&self.file_path)
    }

    /// Total lines in diff
    pub(super) fn total_lines(&self) -> u16 {
        self.diff_lines.len() as u16
    }

    /// Visible lines (capped to max)
    pub(super) fn visible_lines(&self) -> u16 {
        self.diff_lines.len().min(MAX_VISIBLE_LINES as usize) as u16
    }

    /// Maximum scroll offset
    pub(super) fn max_scroll(&self) -> u16 {
        self.total_lines().saturating_sub(MAX_VISIBLE_LINES)
    }

    /// Whether scrollbar should be shown
    pub fn needs_scrollbar(&self) -> bool {
        self.total_lines() > MAX_VISIBLE_LINES
    }

    /// Get scroll info for drag handling: (total_lines, visible_lines, scrollbar_height)
    pub fn get_scroll_info(&self) -> (u16, u16, u16) {
        let total = self.total_lines();
        let visible = total.min(MAX_VISIBLE_LINES);
        (total, visible, visible)
    }

    /// Scroll up
    pub(super) fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Scroll down
    pub(super) fn scroll_down(&mut self) {
        let max = self.max_scroll();
        if self.scroll_offset < max {
            self.scroll_offset += 1;
        }
    }

    /// Check if click is on the toggle button
    pub(super) fn is_toggle_click(&self, area: Rect, column: u16, row: u16) -> bool {
        if row != area.y {
            return false;
        }
        let right_x = area.x + area.width - 1;
        let needs_scrollbar = self.needs_scrollbar();
        let content_end_x = if needs_scrollbar {
            right_x - 1
        } else {
            right_x
        };
        let toggle_start = content_end_x - 4;
        let toggle_end = content_end_x - 1;
        column >= toggle_start && column <= toggle_end
    }
}

impl StreamBlock for EditBlock {
    fn height(&self, _width: u16, _theme: &Theme) -> u16 {
        if self.collapsed {
            return 1;
        }
        self.cached_height
    }

    fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        _focused: bool,
        clip: Option<ClipContext>,
    ) {
        if self.collapsed {
            let symbol = self.get_symbol();
            let text = format!("{} Editing {}", symbol, self.short_path());
            let mut x = area.x;
            let max_x = area.x + area.width;
            for ch in text.chars() {
                let char_width = ch.width().unwrap_or(0);
                if x + char_width as u16 > max_x {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, area.y)) {
                    cell.set_char(ch);
                    if ch == '±' || ch == '∓' || ch == '✓' {
                        cell.set_fg(theme.accent_color);
                    } else {
                        cell.set_fg(theme.text_color);
                    }
                }
                x += char_width as u16;
            }
            return;
        }
        match self.diff_mode {
            DiffMode::Unified => self.render_unified(area, buf, theme, self.diff_mode, clip),
            DiffMode::SideBySide => {
                self.render_side_by_side(area, buf, theme, self.diff_mode, clip)
            }
        }
    }

    fn handle_event(
        &mut self,
        event: &Event,
        area: Rect,
        _clip: Option<ClipContext>,
    ) -> EventResult {
        match event {
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

                if in_area {
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

                if in_area {
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
                if self.is_toggle_click(area, *column, *row) {
                    return EventResult::Action(BlockEvent::ToggleDiffMode);
                }

                let in_area = *row >= area.y
                    && *row < area.y + area.height
                    && *column >= area.x
                    && *column < area.x + area.width;

                if in_area && self.needs_scrollbar() {
                    let scrollbar_x = area.x + area.width - 2;
                    if *column >= scrollbar_x {
                        let click_y = (*row - area.y).saturating_sub(1) as usize;
                        let total = self.total_lines() as usize;
                        let visible = MAX_VISIBLE_LINES as usize;
                        let max_scroll = total.saturating_sub(visible);
                        let new_offset = if visible > 0 {
                            (click_y * max_scroll) / visible
                        } else {
                            0
                        };
                        self.scroll_offset = new_offset.min(max_scroll) as u16;
                        return EventResult::Consumed;
                    }
                }

                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }

    fn tick(&mut self) -> bool {
        self.tick_animation()
    }

    fn get_text_content(&self) -> Option<String> {
        // When collapsed, only return header (matches rendered height of 1)
        if self.collapsed {
            return Some(format!("Edit {}", self.file_path));
        }

        let mut result = String::new();

        // Header with file path
        result.push_str(&format!("Edit {}\n", self.file_path));

        // Show the diff in a unified format
        if !self.old_string.is_empty() || !self.new_string.is_empty() {
            result.push_str("--- old\n");
            result.push_str("+++ new\n");

            // Show removed lines
            for line in self.old_string.lines() {
                result.push_str(&format!("-{}\n", line));
            }

            // Show added lines
            for line in self.new_string.lines() {
                result.push_str(&format!("+{}\n", line));
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result.trim_end().to_string())
        }
    }
}
