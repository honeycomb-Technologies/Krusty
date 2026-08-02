//! Compact expandable row shared by tools, thinking, notices, and plans.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::tui_v2::{
    model::capability::CapabilityProfile,
    presentation::{symbols::Symbols, theme::SemanticTheme},
};

use super::{
    status_glyph::StatusGlyph,
    text_style::{truncate_to_width, TextRole},
};

pub struct ExpandableRow<'a> {
    pub indent: u16,
    pub status: StatusGlyph,
    pub family: &'a str,
    pub summary: &'a str,
    pub metadata: Option<&'a str>,
    pub expandable: bool,
    pub expanded: bool,
    pub focused: bool,
}

impl ExpandableRow<'_> {
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        capability: CapabilityProfile,
        theme: SemanticTheme,
    ) {
        if area.is_empty() {
            return;
        }
        let symbols = Symbols::for_mode(capability.glyph_mode);
        let disclosure = self.expandable.then_some(if self.expanded {
            symbols.expanded
        } else {
            symbols.collapsed
        });
        let fixed_width = usize::from(self.indent)
            .saturating_add(2)
            .saturating_add(disclosure.map_or(0, |_| 2));
        let family_width = UnicodeWidthStr::width(self.family).min(12);
        let metadata_width = self
            .metadata
            .map(UnicodeWidthStr::width)
            .filter(|width| {
                usize::from(area.width)
                    >= fixed_width
                        .saturating_add(family_width)
                        .saturating_add(*width)
                        .saturating_add(12)
            })
            .unwrap_or(0);
        let summary_width = usize::from(area.width)
            .saturating_sub(fixed_width)
            .saturating_sub(family_width)
            .saturating_sub(metadata_width);
        let row_style = if self.focused {
            TextRole::Selection.style(theme)
        } else {
            Style::default().bg(theme.canvas)
        };
        let mut spans = vec![
            Span::raw(" ".repeat(usize::from(self.indent))),
            self.status.span(capability, theme),
            Span::raw(" "),
            Span::styled(
                truncate_to_width(self.family, family_width, "…"),
                TextRole::Label.style(theme),
            ),
            Span::raw(" "),
            Span::styled(
                truncate_to_width(self.summary, summary_width, "…"),
                TextRole::Body.style(theme),
            ),
        ];
        if metadata_width > 0 {
            spans.push(Span::styled(
                format!(" {}", self.metadata.unwrap_or_default()),
                TextRole::Muted.style(theme),
            ));
        }
        if let Some(disclosure) = disclosure {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(disclosure, TextRole::Muted.style(theme)));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)).style(row_style), area);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::tui_v2::{
        model::capability::{CapabilityProfile, ColorDepth, GlyphMode},
        presentation::theme::{SemanticTheme, ThemeKind},
    };

    use super::*;
    use crate::tui_v2::components::primitive::status_glyph::StatusKind;

    #[test]
    fn narrow_row_elides_metadata_before_the_summary() {
        let mut terminal = Terminal::new(TestBackend::new(30, 1)).expect("terminal");
        let capability = CapabilityProfile {
            glyph_mode: GlyphMode::Ascii,
            color_depth: ColorDepth::Monochrome,
        };
        let theme = SemanticTheme::resolve(ThemeKind::MitsuroDark, capability.color_depth);
        terminal
            .draw(|frame| {
                ExpandableRow {
                    indent: 1,
                    status: StatusGlyph {
                        kind: StatusKind::Running,
                        phase: 0,
                    },
                    family: "bash",
                    summary: "cargo test workspace",
                    metadata: Some("9999 lines"),
                    expandable: true,
                    expanded: false,
                    focused: false,
                }
                .render(frame, frame.area(), capability, theme);
            })
            .expect("row");
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("cargo test"));
        assert!(!text.contains("9999 lines"));
    }
}
