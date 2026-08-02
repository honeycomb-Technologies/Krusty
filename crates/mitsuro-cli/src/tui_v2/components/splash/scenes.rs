//! Home load scene: box-drawing mitsuro with a linear stroke-in.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::mark::{self, MARK_HEIGHT, MARK_WIDTH};
use crate::tui_v2::{model::capability::GlyphMode, presentation::theme::SemanticTheme};

/// Wordmark stroke-in duration (ms). Linear — no ease-out slowdown.
const TRACE_MS: u64 = 1_100;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    elapsed_ms: u64,
    complete: bool,
    glyph_mode: GlyphMode,
    theme: SemanticTheme,
) {
    if area.is_empty() {
        return;
    }

    let ascii = matches!(glyph_mode, GlyphMode::Ascii);
    let mark_w = MARK_WIDTH.min(area.width);
    let mark_h = MARK_HEIGHT.min(area.height.max(1));
    let mark_x0 = area.width.saturating_sub(mark_w) / 2;
    let mark_y0 = area.height.saturating_sub(mark_h) / 2;

    let total_ink = mark::ink_cells(ascii).max(1);
    let revealed = if complete {
        total_ink
    } else {
        let t = (elapsed_ms as f64 / TRACE_MS as f64).clamp(0.0, 1.0);
        ((t * total_ink as f64).floor() as usize).min(total_ink)
    };

    let mut lines = Vec::with_capacity(area.height as usize);
    for row in 0..area.height {
        let mut spans = Vec::with_capacity(area.width as usize);
        for col in 0..area.width {
            if row >= mark_y0
                && row < mark_y0 + mark_h
                && col >= mark_x0
                && col < mark_x0 + mark_w
            {
                let sx = (col - mark_x0) as usize;
                let sy = (row - mark_y0) as usize;
                let ch = mark::char_at(ascii, sy, sx);
                if ch != ' ' {
                    if let Some(index) = mark::stroke_index(ascii, sy, sx) {
                        if index < revealed {
                            spans.push(Span::styled(
                                ch.to_string(),
                                Style::default()
                                    .fg(theme.thinking)
                                    .add_modifier(Modifier::BOLD),
                            ));
                            continue;
                        }
                    }
                }
            }
            spans.push(Span::raw(" "));
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.canvas)),
        area,
    );
}
