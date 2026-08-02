//! Canonical code-session picker.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui_v2::{
    app::state::PickerUiState,
    components::primitive::overlay_chrome::{paint_crossbar, OverlayChromeLayout},
    model::capability::{CapabilityProfile, GlyphMode},
    presentation::theme::SemanticTheme,
    services::RecentSession,
};

pub fn filtered<'a>(
    sessions: &'a [RecentSession],
    query: &str,
) -> impl Iterator<Item = &'a RecentSession> {
    let query = query.trim().to_lowercase();
    sessions.iter().filter(move |session| {
        query.is_empty()
            || session.title.to_lowercase().contains(&query)
            || session
                .model
                .as_deref()
                .is_some_and(|model| model.to_lowercase().contains(&query))
    })
}

pub fn render(
    frame: &mut Frame,
    chrome: OverlayChromeLayout,
    sessions: &[RecentSession],
    picker: &PickerUiState,
    capability: CapabilityProfile,
    theme: SemanticTheme,
) {
    let glyph_mode = capability.glyph_mode;
    let pointer = if glyph_mode == GlyphMode::Ascii {
        "> "
    } else {
        "› "
    };
    let (search_area, crossbar_y, list_area) = chrome.search_list().unwrap_or((
        Rect::new(chrome.body.x, chrome.body.y, chrome.body.width, 0),
        chrome.body.y,
        chrome.body,
    ));

    if search_area.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("search  ", Style::default().fg(theme.foreground_muted)),
                Span::styled(
                    if picker.query.is_empty() {
                        "type to filter"
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
    }

    let matches = filtered(sessions, &picker.query).collect::<Vec<_>>();
    let mut lines = Vec::new();
    if matches.is_empty() {
        lines.push(Line::styled(
            if sessions.is_empty() {
                "No code conversations in this workspace yet."
            } else {
                "No conversations match this filter."
            },
            Style::default().fg(theme.foreground_muted),
        ));
    } else {
        let reserved = usize::from(picker.error.is_some());
        let available = usize::from(list_area.height)
            .saturating_sub(reserved)
            .max(1);
        let selected = picker.selected.min(matches.len().saturating_sub(1));
        let start = selected.saturating_sub(available.saturating_sub(1));
        for (index, session) in matches.iter().enumerate().skip(start).take(available) {
            let active = index == selected;
            lines.push(Line::from(vec![
                Span::styled(
                    if active { pointer } else { "  " },
                    Style::default().fg(theme.identity),
                ),
                Span::styled(
                    session.title.as_str(),
                    Style::default()
                        .fg(theme.foreground)
                        .add_modifier(if active {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    session
                        .model
                        .as_ref()
                        .map(|model| format!("  {model}"))
                        .unwrap_or_default(),
                    Style::default().fg(theme.foreground_muted),
                ),
            ]));
        }
    }
    if let Some(error) = &picker.error {
        lines.push(Line::styled(error, Style::default().fg(theme.error)));
    }
    if list_area.height > 0 {
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().fg(theme.foreground).bg(theme.surface)),
            list_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtering_matches_titles_and_models_without_changing_identity() {
        let sessions = vec![
            RecentSession {
                session_id: "one".to_owned(),
                title: "Polish setup".to_owned(),
                model: Some("GPT Alpha".to_owned()),
            },
            RecentSession {
                session_id: "two".to_owned(),
                title: "Fix artifacts".to_owned(),
                model: Some("Claude Beta".to_owned()),
            },
        ];
        assert_eq!(
            filtered(&sessions, "claude")
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["two"]
        );
        assert_eq!(
            filtered(&sessions, "POLISH")
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["one"]
        );
    }
}
