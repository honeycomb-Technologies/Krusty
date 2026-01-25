//! Edit block rendering
//!
//! Contains unified and side-by-side diff view renderers.

use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use unicode_width::UnicodeWidthChar;

use super::block::EditBlock;
use super::{DiffLine, DiffMode, MAX_VISIBLE_LINES};
use crate::tui::blocks::ClipContext;
use crate::tui::components::scrollbars::render_scrollbar;
use crate::tui::themes::Theme;

impl EditBlock {
    /// Render unified diff view
    pub(super) fn render_unified(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        diff_mode: DiffMode,
        clip: Option<ClipContext>,
    ) {
        if area.height < 3 || area.width < 20 {
            return;
        }

        let (clip_top, clip_bottom) = clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0));

        let border_color = theme.border_color;
        let context_color = theme.diff_context_color;
        let content_color = theme.text_color;
        let line_num_color = theme.line_number_color;
        let deletion_color = theme.diff_remove_color;
        let addition_color = theme.diff_add_color;

        let needs_scrollbar = self.needs_scrollbar();
        let right_x = area.x + area.width - 1;
        let content_end_x = if needs_scrollbar {
            right_x - 1
        } else {
            right_x
        };

        let mut render_y = area.y;

        // Header row
        if clip_top == 0 {
            if let Some(cell) = buf.cell_mut((area.x, render_y)) {
                cell.set_char('╭');
                cell.set_fg(border_color);
            }

            let mode_icon = diff_mode.icon();
            let symbol = self.get_symbol();
            let header = format!("─ {} Edit {} ", symbol, self.short_path());

            let mut x = area.x + 1;
            let max_header_x = content_end_x - 4;
            for ch in header.chars() {
                let char_width = ch.width().unwrap_or(0);
                if x + char_width as u16 > max_header_x {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, render_y)) {
                    cell.set_char(ch);
                    if ch == '─' {
                        cell.set_fg(border_color);
                    } else if ch == '◉' || ch == '◐' || ch == '◑' || ch == '^' || ch == '_' {
                        cell.set_fg(theme.accent_color);
                    } else {
                        cell.set_fg(theme.text_color);
                    }
                }
                x += char_width as u16;
            }

            let toggle_start = content_end_x - 4;
            while x < toggle_start {
                if let Some(cell) = buf.cell_mut((x, render_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
                x += 1;
            }

            // Toggle button [≡] or [║]
            if let Some(cell) = buf.cell_mut((x, render_y)) {
                cell.set_char('[');
                cell.set_fg(border_color);
            }
            x += 1;
            if let Some(cell) = buf.cell_mut((x, render_y)) {
                cell.set_char(mode_icon.chars().next().unwrap_or('≡'));
                cell.set_fg(theme.accent_color);
            }
            x += 1;
            if let Some(cell) = buf.cell_mut((x, render_y)) {
                cell.set_char(']');
                cell.set_fg(border_color);
            }
            x += 1;

            if needs_scrollbar && x < right_x {
                if let Some(cell) = buf.cell_mut((x, render_y)) {
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
        let reserved_bottom = if clip_bottom == 0 { 1 } else { 0 };
        let reserved_top = if clip_top == 0 { 1 } else { 0 };
        let content_rows = area.height.saturating_sub(reserved_top + reserved_bottom);

        for row_idx in 0..content_rows {
            let line_idx = start_line + row_idx as usize;
            let y = render_y + row_idx;

            if y >= area.y + area.height - reserved_bottom {
                break;
            }

            // Left border
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }

            if let Some(diff_line) = self.diff_lines.get(line_idx) {
                let (marker, marker_color, line_num, content, text_color) = match diff_line {
                    DiffLine::Context { line_num, content } => (
                        ' ',
                        border_color,
                        *line_num,
                        content.as_str(),
                        context_color,
                    ),
                    DiffLine::Removed { line_num, content } => (
                        '-',
                        deletion_color,
                        *line_num,
                        content.as_str(),
                        content_color,
                    ),
                    DiffLine::Added { line_num, content } => (
                        '+',
                        addition_color,
                        *line_num,
                        content.as_str(),
                        content_color,
                    ),
                };

                // Gutter marker
                if let Some(cell) = buf.cell_mut((area.x + 1, y)) {
                    cell.set_char(marker);
                    cell.set_fg(marker_color);
                }

                // Line number (4 chars)
                let line_num_str = format!("{:>4}", line_num);
                for (i, ch) in line_num_str.chars().enumerate() {
                    let x = area.x + 2 + i as u16;
                    if x < content_end_x {
                        if let Some(cell) = buf.cell_mut((x, y)) {
                            cell.set_char(ch);
                            cell.set_fg(line_num_color);
                        }
                    }
                }

                // Separator
                if let Some(cell) = buf.cell_mut((area.x + 6, y)) {
                    cell.set_char('│');
                    cell.set_fg(border_color);
                }

                // Content - render char by char with proper width tracking
                let content_start_x = area.x + 7;
                let max_content_width = (content_end_x - content_start_x) as usize;
                let mut x = content_start_x;
                let mut width_used = 0;

                for ch in content.chars() {
                    let char_width = ch.width().unwrap_or(0);
                    if width_used + char_width > max_content_width || x >= content_end_x {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(ch);
                        cell.set_fg(text_color);
                    }
                    x += char_width as u16;
                    width_used += char_width;
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

            if needs_scrollbar {
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
            let scrollbar_y = area.y + if clip_top == 0 { 1 } else { 0 };
            let scrollbar_h = content_rows;
            if scrollbar_h > 0 {
                let sb_area = Rect::new(content_end_x, scrollbar_y, 1, scrollbar_h);
                render_scrollbar(
                    buf,
                    sb_area,
                    self.scroll_offset as usize,
                    self.total_lines() as usize,
                    MAX_VISIBLE_LINES as usize,
                    theme.accent_color,
                    theme.scrollbar_bg_color,
                );
            }
        }
    }

    /// Render side-by-side diff view
    pub(super) fn render_side_by_side(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        diff_mode: DiffMode,
        clip: Option<ClipContext>,
    ) {
        if area.height < 3 || area.width < 40 {
            return self.render_unified(area, buf, theme, diff_mode, clip);
        }

        let (clip_top, clip_bottom) = clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0));

        let border_color = theme.border_color;
        let context_color = theme.diff_context_color;
        let content_color = theme.text_color;
        let line_num_color = theme.line_number_color;
        let deletion_color = theme.diff_remove_color;
        let deletion_bg = theme.diff_remove_bg_color;
        let addition_color = theme.diff_add_color;
        let addition_bg = theme.diff_add_bg_color;

        let needs_scrollbar = self.needs_scrollbar();
        let right_x = area.x + area.width - 1;
        let content_end_x = if needs_scrollbar {
            right_x - 1
        } else {
            right_x
        };

        let total_inner = content_end_x - area.x - 1;
        let mid_x = area.x + total_inner / 2;

        let mut render_y = area.y;

        // Header row
        if clip_top == 0 {
            if let Some(cell) = buf.cell_mut((area.x, render_y)) {
                cell.set_char('╭');
                cell.set_fg(border_color);
            }

            let mode_icon = diff_mode.icon();
            let symbol = self.get_symbol();
            let header = format!("─ {} Edit {} ", symbol, self.short_path());

            let mut x = area.x + 1;
            let max_header_x = content_end_x - 4;
            for ch in header.chars() {
                let char_width = ch.width().unwrap_or(0);
                if x + char_width as u16 > max_header_x {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, render_y)) {
                    cell.set_char(ch);
                    if ch == '─' {
                        cell.set_fg(border_color);
                    } else if ch == '◉' || ch == '◐' || ch == '◑' || ch == '^' || ch == '_' {
                        cell.set_fg(theme.accent_color);
                    } else {
                        cell.set_fg(theme.text_color);
                    }
                }
                x += char_width as u16;
            }

            let toggle_start = content_end_x - 4;
            while x < toggle_start {
                if let Some(cell) = buf.cell_mut((x, render_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
                x += 1;
            }

            // Toggle button
            if let Some(cell) = buf.cell_mut((x, render_y)) {
                cell.set_char('[');
                cell.set_fg(border_color);
            }
            x += 1;
            if let Some(cell) = buf.cell_mut((x, render_y)) {
                cell.set_char(mode_icon.chars().next().unwrap_or('║'));
                cell.set_fg(theme.accent_color);
            }
            x += 1;
            if let Some(cell) = buf.cell_mut((x, render_y)) {
                cell.set_char(']');
                cell.set_fg(border_color);
            }
            x += 1;

            if needs_scrollbar && x < right_x {
                if let Some(cell) = buf.cell_mut((x, render_y)) {
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

        let total_sbs_lines = self.sbs_pairs.len();
        let content_start_offset = if clip_top > 0 { clip_top - 1 } else { 0 };
        let start_line = (self.scroll_offset + content_start_offset) as usize;
        let reserved_bottom = if clip_bottom == 0 { 1 } else { 0 };
        let reserved_top = if clip_top == 0 { 1 } else { 0 };
        let content_rows = area.height.saturating_sub(reserved_top + reserved_bottom);

        for row_idx in 0..content_rows {
            let line_idx = start_line + row_idx as usize;
            let y = render_y + row_idx;

            if y >= area.y + area.height - reserved_bottom {
                break;
            }
            if line_idx >= total_sbs_lines {
                break;
            }

            // Left border
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }

            let (left_idx, right_idx) = self
                .sbs_pairs
                .get(line_idx)
                .copied()
                .unwrap_or((None, None));
            let left_line = left_idx.and_then(|i| self.diff_lines.get(i));
            let right_line = right_idx.and_then(|i| self.diff_lines.get(i));

            // Left panel (OLD)
            self.render_sbs_panel(
                buf,
                y,
                area.x + 1,
                mid_x - 1,
                left_line,
                deletion_color,
                deletion_bg,
                context_color,
                content_color,
                line_num_color,
                border_color,
                true,
            );

            // Center divider
            if let Some(cell) = buf.cell_mut((mid_x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }

            // Right panel (NEW)
            self.render_sbs_panel(
                buf,
                y,
                mid_x + 1,
                content_end_x,
                right_line,
                addition_color,
                addition_bg,
                context_color,
                content_color,
                line_num_color,
                border_color,
                false,
            );

            // Right border
            if let Some(cell) = buf.cell_mut((right_x, y)) {
                cell.set_char('│');
                cell.set_fg(border_color);
            }
        }

        // Bottom border
        if clip_bottom == 0 {
            let bottom_y = area.y + area.height - 1;

            if let Some(cell) = buf.cell_mut((area.x, bottom_y)) {
                cell.set_char('╰');
                cell.set_fg(border_color);
            }

            for x in (area.x + 1)..mid_x {
                if let Some(cell) = buf.cell_mut((x, bottom_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            if let Some(cell) = buf.cell_mut((mid_x, bottom_y)) {
                cell.set_char('┴');
                cell.set_fg(border_color);
            }

            for x in (mid_x + 1)..content_end_x {
                if let Some(cell) = buf.cell_mut((x, bottom_y)) {
                    cell.set_char('─');
                    cell.set_fg(border_color);
                }
            }

            if needs_scrollbar {
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
            let scrollbar_y = area.y + if clip_top == 0 { 1 } else { 0 };
            let scrollbar_h = content_rows;
            if scrollbar_h > 0 {
                let sb_area = Rect::new(content_end_x, scrollbar_y, 1, scrollbar_h);
                render_scrollbar(
                    buf,
                    sb_area,
                    self.scroll_offset as usize,
                    total_sbs_lines,
                    MAX_VISIBLE_LINES as usize,
                    theme.accent_color,
                    theme.scrollbar_bg_color,
                );
            }
        }
    }

    /// Render a single panel in side-by-side view
    #[allow(clippy::too_many_arguments)]
    fn render_sbs_panel(
        &self,
        buf: &mut Buffer,
        y: u16,
        start_x: u16,
        end_x: u16,
        line: Option<&DiffLine>,
        change_color: Color,
        change_bg: Color,
        context_color: Color,
        content_color: Color,
        line_num_color: Color,
        _border_color: Color,
        is_left: bool,
    ) {
        let panel_width = (end_x - start_x) as usize;
        if panel_width < 8 {
            return;
        }

        match line {
            Some(DiffLine::Context { line_num, content }) => {
                if let Some(cell) = buf.cell_mut((start_x, y)) {
                    cell.set_char(' ');
                }
                let ln_str = format!("{:>4}", line_num);
                for (i, ch) in ln_str.chars().enumerate() {
                    let x = start_x + 1 + i as u16;
                    if x < end_x {
                        if let Some(cell) = buf.cell_mut((x, y)) {
                            cell.set_char(ch);
                            cell.set_fg(line_num_color);
                        }
                    }
                }
                if let Some(cell) = buf.cell_mut((start_x + 5, y)) {
                    cell.set_char(' ');
                }
                let content_start = start_x + 6;
                let max_width = (end_x.saturating_sub(content_start)) as usize;
                let mut x = content_start;
                let mut width_used = 0;
                for ch in content.chars() {
                    let cw = ch.width().unwrap_or(0);
                    if width_used + cw > max_width || x >= end_x {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(ch);
                        cell.set_fg(context_color);
                    }
                    x += cw as u16;
                    width_used += cw;
                }
            }
            Some(DiffLine::Removed { line_num, content }) if is_left => {
                for x in start_x..end_x {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_bg(change_bg);
                    }
                }
                if let Some(cell) = buf.cell_mut((start_x, y)) {
                    cell.set_char('-');
                    cell.set_fg(change_color);
                }
                let ln_str = format!("{:>4}", line_num);
                for (i, ch) in ln_str.chars().enumerate() {
                    let x = start_x + 1 + i as u16;
                    if x < end_x {
                        if let Some(cell) = buf.cell_mut((x, y)) {
                            cell.set_char(ch);
                            cell.set_fg(change_color);
                        }
                    }
                }
                if let Some(cell) = buf.cell_mut((start_x + 5, y)) {
                    cell.set_char(' ');
                }
                let content_start = start_x + 6;
                let max_width = (end_x.saturating_sub(content_start)) as usize;
                let mut x = content_start;
                let mut width_used = 0;
                for ch in content.chars() {
                    let cw = ch.width().unwrap_or(0);
                    if width_used + cw > max_width || x >= end_x {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(ch);
                        cell.set_fg(content_color);
                    }
                    x += cw as u16;
                    width_used += cw;
                }
            }
            Some(DiffLine::Added { line_num, content }) if !is_left => {
                for x in start_x..end_x {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_bg(change_bg);
                    }
                }
                if let Some(cell) = buf.cell_mut((start_x, y)) {
                    cell.set_char('+');
                    cell.set_fg(change_color);
                }
                let ln_str = format!("{:>4}", line_num);
                for (i, ch) in ln_str.chars().enumerate() {
                    let x = start_x + 1 + i as u16;
                    if x < end_x {
                        if let Some(cell) = buf.cell_mut((x, y)) {
                            cell.set_char(ch);
                            cell.set_fg(change_color);
                        }
                    }
                }
                if let Some(cell) = buf.cell_mut((start_x + 5, y)) {
                    cell.set_char(' ');
                }
                let content_start = start_x + 6;
                let max_width = (end_x.saturating_sub(content_start)) as usize;
                let mut x = content_start;
                let mut width_used = 0;
                for ch in content.chars() {
                    let cw = ch.width().unwrap_or(0);
                    if width_used + cw > max_width || x >= end_x {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char(ch);
                        cell.set_fg(content_color);
                    }
                    x += cw as u16;
                    width_used += cw;
                }
            }
            None => {}
            _ => {}
        }
    }
}
