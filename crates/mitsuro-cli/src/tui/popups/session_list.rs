//! Session list popup - simple list for current directory
//!
//! TUI shows sessions for the current working directory only.
//! User already knows where they are (they launched from there).

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::path::PathBuf;

use super::common::{
    center_content, center_rect, popup_block, popup_title, render_popup_background,
    scroll_indicator, PopupSize,
};
use super::scroll::ScrollState;
use crate::tui::themes::Theme;

/// Session metadata for display
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

/// Session list popup state
pub struct SessionListPopup {
    /// Scroll state for navigation
    scroll: ScrollState,
    pub sessions: Vec<SessionInfo>,
    /// Current working directory (for title display)
    current_dir: Option<String>,
}

impl Default for SessionListPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionListPopup {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(0),
            sessions: Vec::new(),
            current_dir: None,
        }
    }

    /// Set the current working directory (shown in title)
    pub fn set_current_directory(&mut self, dir: &str) {
        self.current_dir = Some(dir.to_string());
    }

    /// Set sessions for current directory
    pub fn set_sessions(&mut self, sessions: Vec<SessionInfo>) {
        let count = sessions.len();
        self.sessions = sessions;
        self.scroll = ScrollState::new(count);
    }

    pub fn next(&mut self) {
        self.scroll.next();
    }

    pub fn prev(&mut self) {
        self.scroll.prev();
    }

    /// Get selected session
    pub fn get_selected_session(&self) -> Option<&SessionInfo> {
        self.sessions.get(self.scroll.selected)
    }

    pub fn delete_selected(&mut self) -> Option<SessionInfo> {
        if self.sessions.is_empty() || self.scroll.selected >= self.sessions.len() {
            return None;
        }

        let session = self.sessions.remove(self.scroll.selected);
        tracing::info!(session_id = %session.id, title = %session.title, "Removed session from list");

        // Update scroll state with new count
        self.scroll.set_total(self.sessions.len());
        Some(session)
    }

    pub fn render(&mut self, f: &mut Frame, theme: &Theme) {
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
                Constraint::Min(5),    // Content
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        let visible_height = (chunks[1].height as usize).saturating_sub(2);
        self.scroll.set_visible_height(visible_height);

        // Title with directory context
        let title_text = if let Some(ref dir) = self.current_dir {
            let short_dir = shorten_path(dir);
            format!("Sessions in {}", short_dir)
        } else {
            "Sessions".to_string()
        };
        let title_lines = popup_title(&title_text, theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Session list
        let mut lines: Vec<Line> = Vec::new();

        if self.sessions.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No sessions in this directory".to_string(),
                Style::default()
                    .fg(theme.dim_color)
                    .add_modifier(Modifier::ITALIC),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Start a chat to create one".to_string(),
                Style::default().fg(theme.dim_color),
            )));
        } else {
            // Scroll up indicator
            let items_above = self.scroll.items_above();
            if items_above > 0 {
                lines.push(scroll_indicator("up", items_above, theme));
            }

            // Render visible sessions
            for session_idx in self.scroll.visible_range() {
                let session = &self.sessions[session_idx];
                let is_selected = self.scroll.is_selected(session_idx);

                let style = if is_selected {
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_color)
                };

                let prefix = if is_selected { "▶ " } else { "  " };
                lines.push(Line::from(vec![
                    Span::styled(prefix.to_string(), style),
                    Span::styled(session.title.clone(), style),
                    Span::styled(
                        format!("  {}", session.updated_at),
                        Style::default().fg(theme.dim_color),
                    ),
                ]));
            }

            // Scroll down indicator
            let items_below = self.scroll.items_below();
            if items_below > 0 {
                lines.push(scroll_indicator("down", items_below, theme));
            }
        }

        let content = Paragraph::new(lines).style(Style::default().bg(theme.bg_color));
        let content_area = center_content(chunks[1], 4);
        f.render_widget(content, content_area);

        // Footer
        let footer_text = vec![
            Span::styled(
                "↑↓",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": navigate  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": load  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "d",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": delete  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": cancel", Style::default().fg(theme.text_color)),
        ];

        let footer = Paragraph::new(Line::from(footer_text)).alignment(Alignment::Center);
        f.render_widget(footer, chunks[2]);
    }
}

/// Shorten a path for display (show last 2-3 components)
fn shorten_path(path: &str) -> String {
    let path = PathBuf::from(path);
    let components: Vec<_> = path.components().collect();

    if components.len() <= 3 {
        return path.display().to_string();
    }

    // Show last 3 components with ... prefix
    let last_three: PathBuf = components[components.len() - 3..].iter().collect();
    format!(".../{}", last_three.display())
}
