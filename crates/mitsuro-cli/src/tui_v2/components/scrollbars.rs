//! Compact 1-cell scrollbars for transcript and composer.
//!
//! Dock-channel tracks paint as a continuous solid rail (same cell width for
//! track and thumb) so the bar never looks like a thin smashed line under a
//! fat block.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
    Frame,
};

use crate::tui_v2::presentation::theme::SemanticTheme;

/// Render a 1-column scrollbar. Clears the column first to avoid stale thumbs.
pub fn render_scrollbar(
    frame: &mut Frame,
    area: Rect,
    offset: u32,
    total: u32,
    visible: u32,
    theme: SemanticTheme,
    focused: bool,
) {
    render_scrollbar_glyphs(frame, area, offset, total, visible, theme, focused, false);
}

/// Like [`render_scrollbar`], with optional ASCII-only glyphs for monochrome terminals.
pub fn render_scrollbar_glyphs(
    frame: &mut Frame,
    area: Rect,
    offset: u32,
    total: u32,
    visible: u32,
    theme: SemanticTheme,
    focused: bool,
    ascii: bool,
) {
    if area.is_empty() {
        return;
    }
    // Solid rail: muted track + focused thumb, both full-cell glyphs so the
    // column reads as one continuous bar matching the dock height.
    let thumb = theme.border_focused;
    let track = if focused {
        theme.border_focused
    } else {
        theme.border
    };
    ScrollbarWidget {
        offset,
        total,
        visible,
        thumb,
        track,
        // Channel is page canvas; surface would paint a mismatched strip.
        background: theme.canvas,
        ascii,
    }
    .render(area, frame.buffer_mut());
}

struct ScrollbarWidget {
    offset: u32,
    total: u32,
    visible: u32,
    thumb: Color,
    track: Color,
    background: Color,
    ascii: bool,
}

impl Widget for ScrollbarWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Always paint the full column so height is visibly continuous even
        // when content does not overflow (caller may still hide the region).
        let clear = Style::default().bg(self.background);
        for y in 0..area.height {
            if let Some(cell) = buf.cell_mut((area.x, area.y.saturating_add(y))) {
                cell.set_char(' ');
                cell.set_style(clear);
            }
        }
        if self.total <= self.visible || area.height == 0 {
            return;
        }
        let height = usize::from(area.height);
        let visible = self.visible.max(1) as f32;
        let total = self.total.max(1) as f32;
        // Prefer a readable thumb: at least ~3 cells, at most the track.
        let thumb_size = ((visible / total) * height as f32)
            .max(3.0)
            .min(height as f32)
            .round() as usize;
        let max_offset = self.total.saturating_sub(self.visible).max(1);
        let travel = height.saturating_sub(thumb_size);
        let thumb_pos = ((self.offset as f32 / max_offset as f32) * travel as f32).round() as usize;
        // Full-cell glyphs for both so track and thumb share the same visual weight.
        let (thumb_ch, track_ch) = if self.ascii {
            ('#', ':')
        } else {
            ('█', '│')
        };
        for y in 0..height {
            let is_thumb = y >= thumb_pos && y < thumb_pos.saturating_add(thumb_size);
            if let Some(cell) = buf.cell_mut((area.x, area.y.saturating_add(y as u16))) {
                if is_thumb {
                    // Solid filled cell for the thumb.
                    cell.set_char(if self.ascii { thumb_ch } else { ' ' });
                    cell.set_fg(self.thumb);
                    cell.set_bg(self.thumb);
                } else {
                    cell.set_char(track_ch);
                    cell.set_fg(self.track);
                    cell.set_bg(self.background);
                }
            }
        }
    }
}

/// Map a click Y on a scrollbar track to a scroll offset.
pub fn offset_from_track_y(area: Rect, y: u16, total: u32, visible: u32) -> u32 {
    let max_offset = total.saturating_sub(visible);
    if max_offset == 0 || area.height <= 1 {
        return 0;
    }
    let relative = y.saturating_sub(area.y).min(area.height.saturating_sub(1));
    let ratio = f32::from(relative) / f32::from(area.height.saturating_sub(1));
    ((ratio * max_offset as f32).round() as u32).min(max_offset)
}
