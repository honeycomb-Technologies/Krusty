//! Width-aware row used by pickers and inspectors.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::tui_v2::presentation::theme::SemanticTheme;

use super::text_style::{truncate_to_width, TextRole};

pub struct ListRow<'a> {
    pub primary: &'a str,
    pub secondary: Option<&'a str>,
    pub leading: Option<&'a str>,
    pub metadata: Option<&'a str>,
    pub selected: bool,
    pub focused: bool,
    pub disabled: bool,
    pub destructive: bool,
    pub disclosure: Option<&'a str>,
}

impl ListRow<'_> {
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: SemanticTheme) {
        if area.is_empty() {
            return;
        }
        let prefix = self
            .leading
            .map_or(0, |value| UnicodeWidthStr::width(value) + 1);
        let suffix = self
            .metadata
            .map_or(0, |value| UnicodeWidthStr::width(value) + 1)
            + self
                .disclosure
                .map_or(0, |value| UnicodeWidthStr::width(value) + 1);
        let primary_width = usize::from(area.width)
            .saturating_sub(prefix)
            .saturating_sub(suffix);
        let primary_role = if self.destructive {
            TextRole::Error
        } else if self.disabled {
            TextRole::Muted
        } else {
            TextRole::Body
        };
        let mut spans = Vec::new();
        if let Some(leading) = self.leading {
            spans.push(Span::styled(
                format!("{leading} "),
                TextRole::Muted.style(theme),
            ));
        }
        spans.push(Span::styled(
            truncate_to_width(self.primary, primary_width, "…"),
            primary_role.style(theme),
        ));
        if let Some(secondary) = self.secondary {
            spans.push(Span::styled(
                format!(" · {secondary}"),
                TextRole::Muted.style(theme),
            ));
        }
        if let Some(metadata) = self.metadata {
            spans.push(Span::styled(
                format!(" {metadata}"),
                TextRole::Muted.style(theme),
            ));
        }
        if let Some(disclosure) = self.disclosure {
            spans.push(Span::styled(
                format!(" {disclosure}"),
                TextRole::Muted.style(theme),
            ));
        }
        let style = if self.selected || self.focused {
            TextRole::Selection.style(theme)
        } else {
            TextRole::Body.style(theme)
        };
        frame.render_widget(Paragraph::new(Line::from(spans)).style(style), area);
    }
}
