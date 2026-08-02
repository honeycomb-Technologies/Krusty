//! Composer-owned slash suggestions — popup chrome, scrollable list, no title.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui_v2::{
    app::state::UiState,
    components::{
        primitive::{
            assist_chrome::AssistChrome,
            list_window::visible_range,
        },
        scrollbars,
    },
    input::slash,
    model::capability::GlyphMode,
    presentation::theme::SemanticTheme,
};

pub fn render(frame: &mut Frame, area: Rect, state: &UiState, theme: SemanticTheme) {
    let suggestions = slash::suggestions(&state.composer.text());
    if suggestions.is_empty() || area.is_empty() {
        return;
    }
    let chrome = AssistChrome.render(frame, area, theme, state.capability);
    if chrome.body.is_empty() {
        return;
    }

    let selected = state
        .composer
        .autocomplete_selected
        .min(suggestions.len().saturating_sub(1));
    let list_height = usize::from(chrome.body.height.max(1));
    let need_scrollbar = suggestions.len() > list_height && chrome.body.width > 2;
    let list_area = if need_scrollbar {
        Rect {
            width: chrome.body.width.saturating_sub(1),
            ..chrome.body
        }
    } else {
        chrome.body
    };
    let window = visible_range(suggestions.len(), selected, usize::from(list_area.height.max(1)));
    let pointer = if state.capability.glyph_mode == GlyphMode::Ascii {
        "> "
    } else {
        "› "
    };
    let lines = suggestions
        .iter()
        .enumerate()
        .skip(window.start)
        .take(window.len())
        .map(|(index, suggestion)| {
            let active = index == selected;
            Line::from(vec![
                Span::styled(
                    if active { pointer } else { "  " },
                    Style::default().fg(theme.identity),
                ),
                Span::styled(
                    suggestion.primary,
                    Style::default()
                        .fg(if active {
                            theme.foreground
                        } else {
                            theme.foreground_muted
                        })
                        .add_modifier(if active {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!("  {}", suggestion.description),
                    Style::default().fg(theme.foreground_muted),
                ),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.foreground).bg(theme.surface)),
        list_area,
    );

    if need_scrollbar {
        let sb = Rect::new(
            chrome.body.right().saturating_sub(1),
            chrome.body.y,
            1,
            chrome.body.height,
        );
        let ascii = state.capability.glyph_mode == GlyphMode::Ascii;
        scrollbars::render_scrollbar_glyphs(
            frame,
            sb,
            window.start as u32,
            suggestions.len() as u32,
            list_height as u32,
            theme,
            true,
            ascii,
        );
    }
}

/// Map a screen Y to a suggestion index inside the painted assist panel, if any.
pub fn index_at_y(area: Rect, y: u16, total: usize, selected: usize) -> Option<usize> {
    use crate::tui_v2::components::primitive::assist_chrome::ASSIST_CHROME_ROWS;
    if total == 0 || area.height < ASSIST_CHROME_ROWS {
        return None;
    }
    // Body between top and bottom borders only.
    let body_y = area.y.saturating_add(1);
    let body_h = area.height.saturating_sub(ASSIST_CHROME_ROWS);
    if y < body_y || y >= body_y.saturating_add(body_h) {
        return None;
    }
    let rel = usize::from(y.saturating_sub(body_y));
    let window = visible_range(total, selected, usize::from(body_h.max(1)));
    let index = window.start.saturating_add(rel);
    (index < total).then_some(index)
}
