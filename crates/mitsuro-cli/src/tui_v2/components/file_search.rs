//! Composer-owned project-file picker (`@`) — popup chrome, scrollable, no title.

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
    input::file_search,
    model::capability::GlyphMode,
    presentation::theme::SemanticTheme,
    services::ProjectEntry,
};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &UiState,
    entries: &[ProjectEntry],
    theme: SemanticTheme,
) {
    if area.is_empty() {
        return;
    }
    let chrome = AssistChrome.render(frame, area, theme, state.capability);
    if chrome.body.is_empty() {
        return;
    }

    let matches =
        file_search::suggestions(entries, &state.composer.text(), state.composer.cursor_byte());
    if matches.is_empty() {
        frame.render_widget(
            Paragraph::new("No project entries match.").style(
                Style::default()
                    .fg(theme.foreground_muted)
                    .bg(theme.surface),
            ),
            chrome.body,
        );
        return;
    }

    let selected = state
        .composer
        .file_search_selected
        .min(matches.len().saturating_sub(1));
    let list_height = usize::from(chrome.body.height.max(1));
    let need_scrollbar = matches.len() > list_height && chrome.body.width > 2;
    let list_area = if need_scrollbar {
        Rect {
            width: chrome.body.width.saturating_sub(1),
            ..chrome.body
        }
    } else {
        chrome.body
    };
    let window = visible_range(matches.len(), selected, usize::from(list_area.height.max(1)));
    let pointer = if state.capability.glyph_mode == GlyphMode::Ascii {
        "> "
    } else {
        "› "
    };
    let lines = matches
        .iter()
        .enumerate()
        .skip(window.start)
        .take(window.len())
        .map(|(index, entry)| {
            let active = index == selected;
            let label = if entry.is_directory() {
                format!("{}/", entry.path)
            } else {
                entry.path.clone()
            };
            Line::from(vec![
                Span::styled(
                    if active { pointer } else { "  " },
                    Style::default().fg(theme.identity),
                ),
                Span::styled(
                    label,
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
            matches.len() as u32,
            list_height as u32,
            theme,
            true,
            ascii,
        );
    }
}

/// Map a screen Y to a match index inside the painted assist panel, if any.
pub fn index_at_y(area: Rect, y: u16, total: usize, selected: usize) -> Option<usize> {
    use crate::tui_v2::components::primitive::assist_chrome::ASSIST_CHROME_ROWS;
    if total == 0 || area.height < ASSIST_CHROME_ROWS {
        return None;
    }
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
