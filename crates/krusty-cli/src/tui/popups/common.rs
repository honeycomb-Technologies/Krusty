//! Common popup utilities

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear},
    Frame,
};

use crate::tui::themes::Theme;

/// Standard popup sizes (fixed width x height in characters)
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum PopupSize {
    /// Small popup - auth method, provider selection
    Small,
    /// Medium popup - theme, session list
    Medium,
    /// Large popup - help, model selection, lsp browser
    Large,
}

impl PopupSize {
    /// Get fixed dimensions (width, height) in characters
    pub fn dimensions(&self) -> (u16, u16) {
        match self {
            PopupSize::Small => (50, 18),
            PopupSize::Medium => (60, 22),
            PopupSize::Large => (70, 28),
        }
    }
}

/// Calculate centered popup area with fixed size (not percentage)
pub fn center_rect(width: u16, height: u16, area: Rect) -> Rect {
    // Clamp to available space
    let popup_width = width.min(area.width.saturating_sub(4));
    let popup_height = height.min(area.height.saturating_sub(2));

    // Center horizontally
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    // Center vertically
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;

    Rect::new(x, y, popup_width, popup_height)
}

/// Render popup background (clear + theme bg)
pub fn render_popup_background(f: &mut Frame, area: Rect, theme: &Theme) {
    f.render_widget(Clear, area);
    let bg = Block::default().style(Style::default().bg(theme.bg_color));
    f.render_widget(bg, area);
}

/// Create standard popup block with rounded borders
pub fn popup_block(theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_color))
        .style(Style::default().bg(theme.bg_color))
}

/// Create popup title lines (centered, with separator matching title width)
pub fn popup_title(title: &str, theme: &Theme) -> Vec<Line<'static>> {
    // Separator matches title length (min 16 chars for aesthetics)
    let sep_len = title.chars().count().max(16);
    let separator: String = "═".repeat(sep_len);

    vec![
        Line::from(""),
        Line::from(Span::styled(
            title.to_string(),
            Style::default()
                .fg(theme.title_color)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            separator,
            Style::default().fg(theme.border_color),
        )),
    ]
}

/// Create scroll indicator line (centered)
pub fn scroll_indicator(direction: &str, count: usize, theme: &Theme) -> Line<'static> {
    let arrow = if direction == "up" { "↑" } else { "↓" };
    let text = format!(
        "{} {} more {}",
        arrow,
        count,
        if direction == "up" { "above" } else { "below" }
    );

    Line::from(Span::styled(
        text,
        Style::default()
            .fg(theme.dim_color)
            .add_modifier(Modifier::DIM),
    ))
}

/// Add horizontal padding to center content within a rect
pub fn center_content(area: Rect, padding: u16) -> Rect {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(padding),
            Constraint::Min(0),
            Constraint::Length(padding),
        ])
        .split(area)[1]
}
