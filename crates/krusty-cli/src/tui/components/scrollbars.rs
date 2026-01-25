//! Scrollbar rendering components
//!
//! Unified scrollbar design: 2-char wide, filled track with thumb inside.
//! Used across input, messages, and collapsible blocks.

use ratatui::{buffer::Buffer, layout::Rect, style::Color, Frame};

use crate::tui::themes::Theme;

/// Unified scrollbar renderer (Buffer-based)
///
/// Renders a 1-character wide scrollbar with filled track and solid thumb.
/// Visual: ░ (track) and █ (thumb)
pub fn render_scrollbar(
    buf: &mut Buffer,
    area: Rect,
    offset: usize,
    total: usize,
    visible: usize,
    thumb_color: Color,
    track_color: Color,
) {
    // Always clear the scrollbar area first to prevent stale glyphs
    // This is critical: without clearing, old █/░ chars remain when scrollbar disappears
    for y in 0..area.height as usize {
        let screen_y = area.y + y as u16;
        if let Some(cell) = buf.cell_mut((area.x, screen_y)) {
            cell.set_char(' ');
            cell.set_fg(Color::Reset);
        }
    }

    // Don't render track/thumb if no scrolling needed
    if total <= visible || area.height == 0 {
        return;
    }

    let height = area.height as usize;

    // Calculate thumb size (minimum 2 for visibility)
    let thumb_size = ((visible as f32 / total as f32) * height as f32)
        .max(2.0)
        .min(height as f32)
        .round() as usize;

    // Calculate thumb position
    let max_offset = total.saturating_sub(visible);
    let thumb_pos = if max_offset > 0 {
        ((offset as f32 / max_offset as f32) * (height.saturating_sub(thumb_size)) as f32).round()
            as usize
    } else {
        0
    };

    // Render scrollbar (1 char wide)
    for y in 0..height {
        let is_thumb = y >= thumb_pos && y < thumb_pos + thumb_size;
        let (ch, color) = if is_thumb {
            ('█', thumb_color)
        } else {
            ('░', track_color)
        };

        let screen_y = area.y + y as u16;
        if let Some(cell) = buf.cell_mut((area.x, screen_y)) {
            cell.set_char(ch).set_fg(color);
        }
    }
}

/// Render a scrollbar overlay for the input area
pub fn render_input_scrollbar(
    f: &mut Frame,
    area: Rect,
    total_lines: usize,
    visible_lines: usize,
    viewport_offset: usize,
    theme: &Theme,
) {
    // Area is already positioned, use full width (should be 2)
    render_scrollbar(
        f.buffer_mut(),
        area,
        viewport_offset,
        total_lines,
        visible_lines,
        theme.accent_color,
        theme.scrollbar_bg_color,
    );
}

/// Render scrollbar for messages area
/// Convention: offset=0 means at TOP (oldest), offset=MAX means at BOTTOM (newest)
pub fn render_messages_scrollbar(
    f: &mut Frame,
    area: Rect,
    offset: usize,
    total_lines: usize,
    visible_height: usize,
    theme: &Theme,
) {
    render_scrollbar(
        f.buffer_mut(),
        area,
        offset,
        total_lines,
        visible_height,
        theme.assistant_msg_color,
        theme.scrollbar_bg_color,
    );
}
