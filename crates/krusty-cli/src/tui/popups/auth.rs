//! Authentication popups (provider selection, API key input)

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use super::common::{center_rect, popup_block, popup_title, render_popup_background, PopupSize};
use crate::ai::providers::{builtin_providers, ProviderId};
use crate::tui::themes::Theme;

/// Auth popup states
#[derive(Debug, Clone)]
pub enum AuthState {
    /// Select which provider to configure
    ProviderSelection { selected_index: usize },
    /// Enter API key for any provider
    ApiKeyInput {
        provider: ProviderId,
        input: String,
        error: Option<String>,
    },
    /// API key saved successfully
    Complete { provider: ProviderId },
}

impl Default for AuthState {
    fn default() -> Self {
        Self::ProviderSelection { selected_index: 0 }
    }
}

/// Auth popup
pub struct AuthPopup {
    pub state: AuthState,
    /// Track which providers have credentials configured
    pub configured_providers: Vec<ProviderId>,
}

impl Default for AuthPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthPopup {
    pub fn new() -> Self {
        Self {
            state: AuthState::default(),
            configured_providers: Vec::new(),
        }
    }

    /// Set which providers have credentials configured
    pub fn set_configured_providers(&mut self, providers: Vec<ProviderId>) {
        self.configured_providers = providers;
    }

    pub fn reset(&mut self) {
        self.state = AuthState::default();
    }

    /// Navigate up in provider list
    pub fn prev_provider(&mut self) {
        if let AuthState::ProviderSelection { selected_index } = &mut self.state {
            if *selected_index > 0 {
                *selected_index -= 1;
            }
        }
    }

    /// Navigate down in provider list
    pub fn next_provider(&mut self) {
        if let AuthState::ProviderSelection { selected_index } = &mut self.state {
            let providers = builtin_providers();
            if *selected_index < providers.len() - 1 {
                *selected_index += 1;
            }
        }
    }

    /// Confirm provider selection - go to API key input
    pub fn confirm_provider(&mut self) {
        if let AuthState::ProviderSelection { selected_index } = &self.state {
            let providers = builtin_providers();
            if let Some(provider) = providers.get(*selected_index) {
                self.state = AuthState::ApiKeyInput {
                    provider: provider.id,
                    input: String::new(),
                    error: None,
                };
            }
        }
    }

    /// Go back to provider selection
    pub fn go_back(&mut self) {
        if let AuthState::ApiKeyInput { .. } = &self.state {
            self.state = AuthState::ProviderSelection { selected_index: 0 };
        }
    }

    pub fn add_api_key_char(&mut self, c: char) {
        if let AuthState::ApiKeyInput { input, .. } = &mut self.state {
            input.push(c);
        }
    }

    pub fn backspace_api_key(&mut self) {
        if let AuthState::ApiKeyInput { input, .. } = &mut self.state {
            input.pop();
        }
    }

    pub fn get_api_key(&self) -> Option<&str> {
        if let AuthState::ApiKeyInput { input, .. } = &self.state {
            Some(input.as_str())
        } else {
            None
        }
    }

    /// Mark API key as successfully saved
    pub fn set_api_key_complete(&mut self) {
        if let AuthState::ApiKeyInput { provider, .. } = &self.state {
            self.state = AuthState::Complete {
                provider: *provider,
            };
        }
    }

    pub fn render(&self, f: &mut Frame, theme: &Theme) {
        match &self.state {
            AuthState::ProviderSelection { selected_index } => {
                self.render_provider_selection(f, theme, *selected_index)
            }
            AuthState::ApiKeyInput {
                provider,
                input,
                error,
            } => self.render_api_key_input(f, theme, *provider, input, error.as_deref()),
            AuthState::Complete { provider } => self.render_complete(f, theme, *provider),
        }
    }

    fn render_provider_selection(&self, f: &mut Frame, theme: &Theme, selected_index: usize) {
        let (w, h) = PopupSize::Medium.dimensions();
        let area = center_rect(w, h, f.area());
        render_popup_background(f, area, theme);

        let block = popup_block(theme);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Title
                Constraint::Min(8),    // Content
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_lines = popup_title("Select Provider", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Provider list
        let providers = builtin_providers();
        let mut lines = Vec::new();

        for (i, provider) in providers.iter().enumerate() {
            let is_selected = i == selected_index;
            let is_configured = self.configured_providers.contains(&provider.id);

            let prefix = if is_selected { "  › " } else { "    " };
            let suffix = if is_configured { " [configured]" } else { "" };

            let style = if is_selected {
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_color)
            };

            let configured_style = Style::default().fg(theme.success_color);

            // Provider name with pricing hint
            let pricing = provider
                .pricing_hint
                .as_ref()
                .map(|p| format!(" ({})", p))
                .unwrap_or_default();

            lines.push(Line::from(vec![
                Span::styled(prefix.to_string(), style),
                Span::styled(provider.name.clone(), style),
                Span::styled(pricing, Style::default().fg(theme.dim_color)),
                Span::styled(suffix.to_string(), configured_style),
            ]));

            // Description
            lines.push(Line::from(vec![
                Span::styled("      ".to_string(), Style::default()),
                Span::styled(
                    provider.description.clone(),
                    Style::default().fg(theme.dim_color),
                ),
            ]));

            lines.push(Line::from(""));
        }

        let content = Paragraph::new(lines);
        f.render_widget(content, chunks[1]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "↑↓",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": select  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": configure  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": cancel", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[2]);
    }

    fn render_api_key_input(
        &self,
        f: &mut Frame,
        theme: &Theme,
        provider: ProviderId,
        input: &str,
        error: Option<&str>,
    ) {
        let (w, h) = PopupSize::Medium.dimensions();
        let area = center_rect(w, h, f.area());
        render_popup_background(f, area, theme);

        let block = popup_block(theme);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3), // Title
                Constraint::Length(3), // Instructions
                Constraint::Length(3), // Input
                Constraint::Length(2), // Error
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_text = format!("{} API Key", provider);
        let title_lines = popup_title(&title_text, theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Instructions with provider-specific URL
        let url = match provider {
            ProviderId::Anthropic => "https://console.anthropic.com/",
            ProviderId::OpenRouter => "https://openrouter.ai/keys",
            ProviderId::OpenCodeZen => "https://opencode.ai/zen",
            ProviderId::ZAi => "https://z.ai/",
            ProviderId::MiniMax => "https://platform.minimax.io/",
            ProviderId::Kimi => "https://platform.moonshot.cn/",
        };

        let instructions = Paragraph::new(vec![
            Line::from(vec![
                Span::raw("Enter your "),
                Span::styled(
                    provider.to_string(),
                    Style::default().fg(theme.accent_color),
                ),
                Span::raw(" API key:"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("Get your key from: "),
                Span::styled(
                    url,
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::UNDERLINED),
                ),
            ]),
        ])
        .style(Style::default().fg(theme.text_color));
        f.render_widget(instructions, chunks[1]);

        // Input field
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_color));

        let masked = "*".repeat(input.len());
        let input_widget = Paragraph::new(masked)
            .style(Style::default().fg(theme.text_color))
            .block(input_block);
        f.render_widget(input_widget, chunks[2]);

        // Error message
        if let Some(err) = error {
            let error_widget = Paragraph::new(err)
                .style(Style::default().fg(theme.error_color))
                .alignment(Alignment::Center);
            f.render_widget(error_widget, chunks[3]);
        }

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "Ctrl+V",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": paste  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": save  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": cancel", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[4]);
    }

    fn render_complete(&self, f: &mut Frame, theme: &Theme, provider: ProviderId) {
        let (w, h) = PopupSize::Medium.dimensions();
        let area = center_rect(w, h, f.area());
        render_popup_background(f, area, theme);

        let block = popup_block(theme);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3), // Title
                Constraint::Min(4),    // Content
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_lines = popup_title("Authentication Complete", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        let content = vec![
            Line::from(""),
            Line::from(Span::styled(
                "API key saved successfully!",
                Style::default()
                    .fg(theme.success_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("{} is now configured.", provider),
                Style::default().fg(theme.text_color),
            )),
        ];
        let success = Paragraph::new(content).alignment(Alignment::Center);
        f.render_widget(success, chunks[1]);

        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": close", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[2]);
    }
}
