//! Authentication popups (provider selection, API key input, OAuth flows)

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use super::common::{
    center_rect, popup_block, popup_title, render_popup_background, scroll_indicator, PopupSize,
};
use crate::ai::providers::{builtin_providers, ProviderId};
use crate::tui::themes::Theme;
use krusty_core::auth::AuthMethod;

/// Auth popup states
#[derive(Debug, Clone)]
pub enum AuthState {
    /// Select which provider to configure
    ProviderSelection {
        selected_index: usize,
        scroll_offset: usize,
    },
    /// Select authentication method for OAuth providers
    AuthMethodSelection {
        provider: ProviderId,
        selected_index: usize,
    },
    /// Enter API key for any provider
    ApiKeyInput {
        provider: ProviderId,
        input: String,
        error: Option<String>,
    },
    /// OAuth browser flow - waiting for browser auth
    OAuthBrowserWaiting {
        provider: ProviderId,
        status: String,
    },
    /// OAuth device code flow - showing code to user
    OAuthDeviceCode {
        provider: ProviderId,
        user_code: String,
        verification_uri: String,
    },
    /// Authentication saved successfully
    Complete { provider: ProviderId },
}

impl Default for AuthState {
    fn default() -> Self {
        Self::ProviderSelection {
            selected_index: 0,
            scroll_offset: 0,
        }
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
        if let AuthState::ProviderSelection { selected_index, .. } = &mut self.state {
            if *selected_index > 0 {
                *selected_index -= 1;
                self.ensure_visible(10); // Use reasonable visible height
            }
        }
    }

    /// Navigate down in provider list
    pub fn next_provider(&mut self) {
        if let AuthState::ProviderSelection { selected_index, .. } = &mut self.state {
            let providers = builtin_providers();
            if *selected_index < providers.len() - 1 {
                *selected_index += 1;
                self.ensure_visible(10); // Use reasonable visible height
            }
        }
    }

    /// Ensure selected item is visible within scroll window
    fn ensure_visible(&mut self, visible_height: usize) {
        if let AuthState::ProviderSelection {
            selected_index,
            scroll_offset,
        } = &mut self.state
        {
            if *selected_index < *scroll_offset {
                *scroll_offset = *selected_index;
            } else if *selected_index >= *scroll_offset + visible_height {
                *scroll_offset = *selected_index - visible_height + 1;
            }
        }
    }

    /// Confirm provider selection - go to auth method selection or API key input
    pub fn confirm_provider(&mut self) {
        if let AuthState::ProviderSelection { selected_index, .. } = &self.state {
            let providers = builtin_providers();
            if let Some(provider) = providers.get(*selected_index) {
                // Check if provider supports OAuth
                if provider.id.supports_oauth() {
                    // Show auth method selection
                    self.state = AuthState::AuthMethodSelection {
                        provider: provider.id,
                        selected_index: 0,
                    };
                } else {
                    // Go directly to API key input
                    self.state = AuthState::ApiKeyInput {
                        provider: provider.id,
                        input: String::new(),
                        error: None,
                    };
                }
            }
        }
    }

    /// Navigate up in auth method list
    pub fn prev_auth_method(&mut self) {
        if let AuthState::AuthMethodSelection { selected_index, .. } = &mut self.state {
            if *selected_index > 0 {
                *selected_index -= 1;
            }
        }
    }

    /// Navigate down in auth method list
    pub fn next_auth_method(&mut self) {
        if let AuthState::AuthMethodSelection {
            provider,
            selected_index,
        } = &mut self.state
        {
            let methods = provider.auth_methods();
            if *selected_index < methods.len() - 1 {
                *selected_index += 1;
            }
        }
    }

    /// Confirm auth method selection
    /// Returns the selected auth method if confirmed
    pub fn confirm_auth_method(&mut self) -> Option<(ProviderId, AuthMethod)> {
        if let AuthState::AuthMethodSelection {
            provider,
            selected_index,
        } = &self.state
        {
            let methods = provider.auth_methods();
            if let Some(method) = methods.get(*selected_index) {
                let provider = *provider;
                let method = *method;

                match method {
                    AuthMethod::ApiKey => {
                        self.state = AuthState::ApiKeyInput {
                            provider,
                            input: String::new(),
                            error: None,
                        };
                        None // Handled internally
                    }
                    AuthMethod::OAuthBrowser => {
                        self.state = AuthState::OAuthBrowserWaiting {
                            provider,
                            status: "Opening browser...".to_string(),
                        };
                        Some((provider, method))
                    }
                    AuthMethod::OAuthDevice => {
                        self.state = AuthState::OAuthDeviceCode {
                            provider,
                            user_code: "Loading...".to_string(),
                            verification_uri: "".to_string(),
                        };
                        Some((provider, method))
                    }
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Update OAuth browser flow status
    pub fn set_oauth_browser_status(&mut self, status: &str) {
        if let AuthState::OAuthBrowserWaiting {
            status: current_status,
            ..
        } = &mut self.state
        {
            *current_status = status.to_string();
        }
    }

    /// Set device code info for display
    pub fn set_device_code(&mut self, user_code: &str, verification_uri: &str) {
        if let AuthState::OAuthDeviceCode {
            user_code: current_code,
            verification_uri: current_uri,
            ..
        } = &mut self.state
        {
            *current_code = user_code.to_string();
            *current_uri = verification_uri.to_string();
        }
    }

    /// Mark OAuth authentication as complete
    pub fn set_oauth_complete(&mut self) {
        let provider = match &self.state {
            AuthState::OAuthBrowserWaiting { provider, .. } => *provider,
            AuthState::OAuthDeviceCode { provider, .. } => *provider,
            _ => return,
        };
        self.state = AuthState::Complete { provider };
    }

    /// Set OAuth error
    pub fn set_oauth_error(&mut self, error: &str) {
        let provider = match &self.state {
            AuthState::OAuthBrowserWaiting { provider, .. } => *provider,
            AuthState::OAuthDeviceCode { provider, .. } => *provider,
            _ => return,
        };
        self.state = AuthState::ApiKeyInput {
            provider,
            input: String::new(),
            error: Some(error.to_string()),
        };
    }

    /// Go back to previous state
    pub fn go_back(&mut self) {
        match &self.state {
            AuthState::AuthMethodSelection { .. } => {
                self.state = AuthState::ProviderSelection {
                    selected_index: 0,
                    scroll_offset: 0,
                };
            }
            AuthState::ApiKeyInput { provider, .. } => {
                // If provider supports OAuth, go back to method selection
                if provider.supports_oauth() {
                    self.state = AuthState::AuthMethodSelection {
                        provider: *provider,
                        selected_index: 0,
                    };
                } else {
                    self.state = AuthState::ProviderSelection {
                        selected_index: 0,
                        scroll_offset: 0,
                    };
                }
            }
            AuthState::OAuthBrowserWaiting { provider, .. }
            | AuthState::OAuthDeviceCode { provider, .. } => {
                self.state = AuthState::AuthMethodSelection {
                    provider: *provider,
                    selected_index: 0,
                };
            }
            _ => {
                self.state = AuthState::ProviderSelection {
                    selected_index: 0,
                    scroll_offset: 0,
                };
            }
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
            AuthState::ProviderSelection {
                selected_index,
                scroll_offset,
            } => self.render_provider_selection(f, theme, *selected_index, *scroll_offset),
            AuthState::AuthMethodSelection {
                provider,
                selected_index,
            } => self.render_auth_method_selection(f, theme, *provider, *selected_index),
            AuthState::ApiKeyInput {
                provider,
                input,
                error,
            } => self.render_api_key_input(f, theme, *provider, input, error.as_deref()),
            AuthState::OAuthBrowserWaiting { provider, status } => {
                self.render_oauth_browser_waiting(f, theme, *provider, status)
            }
            AuthState::OAuthDeviceCode {
                provider,
                user_code,
                verification_uri,
            } => self.render_oauth_device_code(f, theme, *provider, user_code, verification_uri),
            AuthState::Complete { provider } => self.render_complete(f, theme, *provider),
        }
    }

    fn render_provider_selection(
        &self,
        f: &mut Frame,
        theme: &Theme,
        selected_index: usize,
        scroll_offset: usize,
    ) {
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

        // Provider list - simplified to one line per provider
        let providers = builtin_providers();
        let mut lines = Vec::new();

        // Calculate visible height (content area minus potential scroll indicators)
        let content_height = chunks[1].height as usize;
        let visible_height = content_height.saturating_sub(2); // Leave room for scroll indicators

        // Scroll indicator (up)
        if scroll_offset > 0 {
            lines.push(scroll_indicator("up", scroll_offset, theme));
        } else {
            lines.push(Line::from("")); // Empty line to maintain spacing
        }

        // Render visible providers
        for (i, provider) in providers
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_height)
        {
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

            lines.push(Line::from(vec![
                Span::styled(prefix.to_string(), style),
                Span::styled(provider.name.clone(), style),
                Span::styled(suffix.to_string(), configured_style),
            ]));
        }

        // Scroll indicator (down)
        let remaining = providers
            .len()
            .saturating_sub(scroll_offset + visible_height);
        if remaining > 0 {
            lines.push(scroll_indicator("down", remaining, theme));
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

    fn render_auth_method_selection(
        &self,
        f: &mut Frame,
        theme: &Theme,
        provider: ProviderId,
        selected_index: usize,
    ) {
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
                Constraint::Length(2), // Subtitle
                Constraint::Min(6),    // Methods
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_text = format!("{} Authentication", provider);
        let title_lines = popup_title(&title_text, theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Subtitle
        let subtitle = Paragraph::new("Choose authentication method:")
            .style(Style::default().fg(theme.text_color))
            .alignment(Alignment::Center);
        f.render_widget(subtitle, chunks[1]);

        // Auth methods
        let methods = provider.auth_methods();
        let mut lines = Vec::new();
        lines.push(Line::from(""));

        for (i, method) in methods.iter().enumerate() {
            let is_selected = i == selected_index;
            let prefix = if is_selected { "  › " } else { "    " };

            let style = if is_selected {
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_color)
            };

            let description = match method {
                AuthMethod::OAuthBrowser => " (recommended - opens browser)",
                AuthMethod::OAuthDevice => " (for SSH/headless)",
                AuthMethod::ApiKey => " (manual key entry)",
            };

            lines.push(Line::from(vec![
                Span::styled(prefix.to_string(), style),
                Span::styled(method.to_string(), style),
                Span::styled(description, Style::default().fg(theme.text_color)),
            ]));
        }

        let content = Paragraph::new(lines);
        f.render_widget(content, chunks[2]);

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
            Span::styled(": confirm  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": back", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[3]);
    }

    fn render_oauth_browser_waiting(
        &self,
        f: &mut Frame,
        theme: &Theme,
        provider: ProviderId,
        status: &str,
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
                Constraint::Min(6),    // Content
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_text = format!("{} OAuth", provider);
        let title_lines = popup_title(&title_text, theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Content
        let content = vec![
            Line::from(""),
            Line::from(Span::styled(
                status,
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Complete authentication in your browser."),
            Line::from("This window will update automatically."),
        ];
        let status_widget = Paragraph::new(content)
            .style(Style::default().fg(theme.text_color))
            .alignment(Alignment::Center);
        f.render_widget(status_widget, chunks[1]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
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

    fn render_oauth_device_code(
        &self,
        f: &mut Frame,
        theme: &Theme,
        provider: ProviderId,
        user_code: &str,
        verification_uri: &str,
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
                Constraint::Min(8),    // Content
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_text = format!("{} Device Code", provider);
        let title_lines = popup_title(&title_text, theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Content
        let content = vec![
            Line::from(""),
            Line::from("Visit this URL:"),
            Line::from(Span::styled(
                verification_uri,
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::UNDERLINED),
            )),
            Line::from(""),
            Line::from("And enter this code:"),
            Line::from(Span::styled(
                user_code,
                Style::default()
                    .fg(theme.success_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Waiting for authorization..."),
        ];
        let code_widget = Paragraph::new(content)
            .style(Style::default().fg(theme.text_color))
            .alignment(Alignment::Center);
        f.render_widget(code_widget, chunks[1]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
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
            ProviderId::OpenAI => "https://platform.openai.com/api-keys",
            ProviderId::ClaudeCodeAcp => "https://console.anthropic.com/", // Uses Anthropic API
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
