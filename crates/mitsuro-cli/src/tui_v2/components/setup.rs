//! First-run connection screen and Connections overlay body.
//!
//! Chrome owns the outer title and footer shelf. This module paints only the
//! step body: a short subtitle, a centered left-aligned list, and errors.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::tui_v2::{
    app::state::{SetupStep, UiState},
    model::capability::GlyphMode,
    presentation::theme::SemanticTheme,
    services::SetupSnapshot,
};

const CONTENT_WIDTH: u16 = 42;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &UiState,
    setup: Option<&SetupSnapshot>,
    theme: SemanticTheme,
) {
    if area.is_empty() {
        return;
    }
    // Match overlay surface so the body does not flash a second plate.
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(theme.surface)),
        area,
    );

    let ascii = state.capability.glyph_mode == GlyphMode::Ascii;
    let (pointer, connected, disconnected, separator) = if ascii {
        ("> ", "+ ", "o ", " | ")
    } else {
        ("› ", "● ", "○ ", " · ")
    };

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::styled(
        step_subtitle(state.setup.step),
        Style::default().fg(theme.foreground_muted),
    ));
    lines.push(Line::default());

    if let Some(setup) = setup {
        match state.setup.step {
            SetupStep::Provider => {
                for (index, provider) in setup.providers.iter().take(10).enumerate() {
                    let active = index == state.setup.provider_index;
                    let status = if provider.connected {
                        "connected"
                    } else {
                        "authenticate"
                    };
                    lines.push(provider_row(
                        if active { pointer } else { "  " },
                        if provider.connected {
                            connected
                        } else {
                            disconnected
                        },
                        provider.connected,
                        &provider.label,
                        status,
                        active,
                        theme,
                    ));
                }
            }
            SetupStep::Credential => {
                if let Some(provider) = setup.providers.get(state.setup.provider_index) {
                    lines.push(Line::styled(
                        format!("{}{separator}API key", provider.label),
                        Style::default()
                            .fg(theme.foreground)
                            .add_modifier(Modifier::BOLD),
                    ));
                    lines.push(Line::styled(
                        "Paste below. Value is masked and stored securely.",
                        Style::default().fg(theme.foreground_muted),
                    ));
                }
            }
            SetupStep::AuthMethod => {
                if let Some(provider) = setup.providers.get(state.setup.provider_index) {
                    lines.push(Line::styled(
                        provider.label.as_str(),
                        Style::default()
                            .fg(theme.identity)
                            .add_modifier(Modifier::BOLD),
                    ));
                    lines.push(Line::default());
                    for (index, method) in provider.auth_methods.iter().enumerate() {
                        let active = index == state.setup.auth_method_index;
                        lines.push(Line::from(vec![
                            Span::styled(
                                if active { pointer } else { "  " },
                                Style::default().fg(theme.identity),
                            ),
                            Span::styled(
                                method.to_string(),
                                Style::default()
                                    .fg(theme.foreground)
                                    .add_modifier(if active {
                                        Modifier::BOLD
                                    } else {
                                        Modifier::empty()
                                    }),
                            ),
                        ]));
                    }
                }
            }
            SetupStep::OAuthWaiting | SetupStep::OAuthPasteCode | SetupStep::CatalogLoading => {
                if let Some(message) = &state.setup.oauth_message {
                    lines.push(Line::styled(
                        if ascii {
                            message.replace('…', "...")
                        } else {
                            message.clone()
                        },
                        Style::default().fg(theme.foreground),
                    ));
                }
                if let Some(code) = &state.setup.device_code {
                    lines.push(Line::styled(
                        format!("device code{separator}{code}"),
                        Style::default()
                            .fg(theme.identity)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                if let Some(url) = &state.setup.oauth_url {
                    lines.push(Line::styled(
                        url.clone(),
                        Style::default().fg(theme.foreground_muted),
                    ));
                }
            }
            SetupStep::Model => {
                if let Some(provider) = setup.providers.get(state.setup.provider_index) {
                    lines.push(Line::styled(
                        provider.label.as_str(),
                        Style::default()
                            .fg(theme.identity)
                            .add_modifier(Modifier::BOLD),
                    ));
                    lines.push(Line::default());
                    for (index, model) in provider.models.iter().take(10).enumerate() {
                        let active = index == state.setup.model_index;
                        lines.push(Line::from(vec![
                            Span::styled(
                                if active { pointer } else { "  " },
                                Style::default().fg(theme.identity),
                            ),
                            Span::styled(
                                model.label.as_str(),
                                Style::default()
                                    .fg(theme.foreground)
                                    .add_modifier(if active {
                                        Modifier::BOLD
                                    } else {
                                        Modifier::empty()
                                    }),
                            ),
                        ]));
                    }
                }
            }
        }
    }

    if let Some(error) = &state.setup.error {
        lines.push(Line::default());
        lines.push(Line::styled(
            error.clone(),
            Style::default().fg(theme.error),
        ));
    }

    let content_height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(area.height);
    let content_width = CONTENT_WIDTH.min(area.width.saturating_sub(2)).max(1);
    let content = Rect::new(
        area.x
            .saturating_add(area.width.saturating_sub(content_width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(content_height) / 2),
        content_width,
        content_height,
    );
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .style(Style::default().bg(theme.surface)),
        content,
    );
}

fn step_subtitle(step: SetupStep) -> &'static str {
    match step {
        SetupStep::Provider => "Choose a provider",
        SetupStep::AuthMethod => "Choose how to authenticate",
        SetupStep::Credential => "Enter credential",
        SetupStep::OAuthWaiting => "Complete in browser",
        SetupStep::OAuthPasteCode => "Paste authorization code",
        SetupStep::CatalogLoading => "Refreshing model catalog",
        SetupStep::Model => "Choose a model",
    }
}

fn provider_row<'a>(
    pointer: &'a str,
    glyph: &'a str,
    connected: bool,
    label: &'a str,
    status: &'a str,
    active: bool,
    theme: SemanticTheme,
) -> Line<'a> {
    // Fixed columns so the centered block stays a clean vertical list:
    // pointer(2) + glyph(2) + label padded + status
    let label_width = 14usize;
    let mut padded = label.to_owned();
    let width = UnicodeWidthStr::width(padded.as_str());
    if width < label_width {
        padded.push_str(&" ".repeat(label_width - width));
    }
    Line::from(vec![
        Span::styled(pointer, Style::default().fg(theme.identity)),
        Span::styled(
            glyph,
            Style::default().fg(if connected {
                theme.success
            } else {
                theme.foreground_muted
            }),
        ),
        Span::styled(
            padded,
            Style::default()
                .fg(theme.foreground)
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            format!("  {status}"),
            Style::default().fg(theme.foreground_muted),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles_are_short_and_step_specific() {
        assert_eq!(step_subtitle(SetupStep::Provider), "Choose a provider");
        assert_eq!(
            step_subtitle(SetupStep::AuthMethod),
            "Choose how to authenticate"
        );
        assert_eq!(step_subtitle(SetupStep::Model), "Choose a model");
    }
}
