//! Title-less purple workspace dock frames.
//!
//! Twin panels share popup containment language (focused border color, Clear,
//! rounded when capable) but never paint gold/identity titles on the border.

use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear},
    Frame,
};

use crate::tui_v2::{
    layout::snapshot::intersect,
    model::capability::CapabilityProfile,
    presentation::{symbols::ASCII_BORDER, theme::SemanticTheme},
};

/// Paint a dock panel frame and return the inner content rect.
///
/// Border always uses `border_focused` (purple). No top title. Optional focus
/// is reserved for future weight cues without reintroducing border labels.
pub fn paint_dock_panel(
    frame: &mut Frame,
    area: Rect,
    _focused: bool,
    theme: SemanticTheme,
    capability: CapabilityProfile,
) -> Rect {
    let area = intersect(area, frame.area());
    if area.is_empty() {
        return area;
    }
    let border_set = if capability.supports_rounded_borders() {
        ratatui::symbols::border::ROUNDED
    } else {
        ASCII_BORDER
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(theme.surface).fg(theme.foreground));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    inner
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    use crate::tui_v2::{
        model::capability::{CapabilityProfile, ColorDepth, GlyphMode},
        presentation::theme::{SemanticTheme, ThemeKind},
    };

    use super::*;

    #[test]
    fn dock_panel_is_purple_and_titleless() {
        let mut terminal = Terminal::new(TestBackend::new(24, 10)).expect("terminal");
        let capability = CapabilityProfile {
            glyph_mode: GlyphMode::Ascii,
            color_depth: ColorDepth::TrueColor,
        };
        let theme = SemanticTheme::resolve(ThemeKind::MitsuroDark, capability.color_depth);
        let area = Rect::new(1, 1, 20, 8);
        terminal
            .draw(|frame| {
                let _ = paint_dock_panel(frame, area, false, theme, capability);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((1, 1)).expect("tl").symbol(), "+");
        assert_eq!(
            buffer.cell((1, 1)).expect("tl").style().fg,
            Some(theme.border_focused)
        );
        // Top edge is pure border — no title glyphs between corners.
        for x in 2..20 {
            assert_eq!(
                buffer.cell((x, 1)).expect("top").symbol(),
                "-",
                "titleless top border at x={x}"
            );
        }
    }
}
