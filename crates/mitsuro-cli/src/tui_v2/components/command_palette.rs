//! Command palette and keyboard help derived from the action registry.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui_v2::{
    app::state::PickerUiState,
    components::primitive::overlay_chrome::{paint_crossbar, OverlayChromeLayout},
    input::action::{ActionContext, ActionDefinition, ActionRegistry},
    model::capability::{CapabilityProfile, GlyphMode},
    presentation::theme::SemanticTheme,
};

pub fn filtered(query: &str) -> Vec<&'static ActionDefinition> {
    let query = query.trim().to_lowercase();
    ActionRegistry::active(ActionContext::Overlay)
        .filter(|action| query.is_empty() || action.label.to_lowercase().contains(&query))
        .collect()
}

pub fn render(
    frame: &mut Frame,
    chrome: OverlayChromeLayout,
    picker: &PickerUiState,
    help_only: bool,
    capability: CapabilityProfile,
    theme: SemanticTheme,
) {
    let glyph_mode = capability.glyph_mode;
    let pointer = if glyph_mode == GlyphMode::Ascii {
        "> "
    } else {
        "› "
    };
    let actions = filtered(if help_only { "" } else { &picker.query });
    let list_area = if help_only {
        chrome.body
    } else if let Some((search_area, crossbar_y, list_area)) = chrome.search_list() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("search  ", Style::default().fg(theme.foreground_muted)),
                Span::styled(
                    if picker.query.is_empty() {
                        "type a command"
                    } else {
                        picker.query.as_str()
                    },
                    Style::default().fg(if picker.query.is_empty() {
                        theme.foreground_muted
                    } else {
                        theme.foreground
                    }),
                ),
            ]))
            .style(Style::default().bg(theme.surface)),
            search_area,
        );
        paint_crossbar(frame, chrome.outer, crossbar_y, theme, capability);
        list_area
    } else {
        chrome.body
    };

    let available = usize::from(list_area.height).max(1);
    let selected = picker.selected.min(actions.len().saturating_sub(1));
    let start = selected.saturating_sub(available.saturating_sub(1));
    let mut lines = Vec::new();
    for (index, action) in actions.iter().enumerate().skip(start).take(available) {
        let active = !help_only && index == selected;
        let binding = action
            .primary_binding()
            .map(|binding| binding.label())
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(
                if active { pointer } else { "  " },
                Style::default().fg(theme.identity),
            ),
            Span::styled(
                action.label,
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                if binding.is_empty() {
                    String::new()
                } else {
                    format!("  {binding}")
                },
                Style::default().fg(theme.foreground_muted),
            ),
        ]));
    }
    if list_area.height > 0 {
        frame.render_widget(
            Paragraph::new(lines).style(
                Style::default()
                    .fg(theme.foreground)
                    .bg(theme.surface),
            ),
            list_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_v2::input::action::ActionId;

    #[test]
    fn palette_search_returns_the_registered_action_not_a_duplicate_command() {
        assert_eq!(
            filtered("choose model")
                .into_iter()
                .map(|action| action.id)
                .collect::<Vec<_>>(),
            vec![ActionId::OpenModelPicker]
        );
    }
}
