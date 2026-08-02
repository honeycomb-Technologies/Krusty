//! Footer hints generated from the canonical action registry.

use ratatui::{
    layout::{Alignment, Rect},
    widgets::Paragraph,
    Frame,
};

use crate::tui_v2::{
    input::action::{ActionContext, ActionRegistry},
    model::capability::GlyphMode,
    presentation::theme::SemanticTheme,
};

use super::text_style::TextRole;

pub struct ActionFooter;

impl ActionFooter {
    pub fn render(
        frame: &mut Frame,
        area: Rect,
        context: ActionContext,
        glyph_mode: GlyphMode,
        theme: SemanticTheme,
    ) {
        let separator = if glyph_mode == GlyphMode::Ascii {
            " | "
        } else {
            " · "
        };
        let hints = ActionRegistry::footer_hints_with_separator(
            context,
            usize::from(area.width),
            separator,
        );
        frame.render_widget(
            Paragraph::new(hints)
                .alignment(Alignment::Right)
                .style(TextRole::Muted.style(theme)),
            area,
        );
    }
}
