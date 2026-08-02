//! Theme selection popup

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::common::{
    center_content, center_rect, popup_block, popup_title, render_popup_background,
    scroll_indicator, PopupSize,
};
use crate::tui::themes::{Theme, THEME_REGISTRY};

/// Theme selection popup state
pub struct ThemeSelectPopup {
    pub selected_index: usize,
    pub scroll_offset: usize,
    /// Original theme name when popup was opened (for cancel restore)
    pub original_theme_name: Option<String>,
}

impl Default for ThemeSelectPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeSelectPopup {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            scroll_offset: 0,
            original_theme_name: None,
        }
    }

    /// Open the popup and store the current theme name for potential restore
    pub fn open(&mut self, current_theme: &str) {
        self.original_theme_name = Some(current_theme.to_string());
        let themes = THEME_REGISTRY.list();
        if let Some(idx) = themes.iter().position(|(name, _)| *name == current_theme) {
            self.selected_index = idx;
            self.ensure_visible();
        }
    }

    /// Get the original theme name (for restore on cancel)
    pub fn get_original_theme_name(&self) -> Option<&str> {
        self.original_theme_name.as_deref()
    }

    pub fn next(&mut self) {
        let themes = THEME_REGISTRY.list();
        if self.selected_index < themes.len() - 1 {
            self.selected_index += 1;
            self.ensure_visible();
        }
    }

    pub fn prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.ensure_visible();
        }
    }

    fn ensure_visible(&mut self) {
        self.ensure_visible_with_height(8); // Default fallback
    }

    fn ensure_visible_with_height(&mut self, visible_height: usize) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index - visible_height + 1;
        }
    }

    pub fn get_selected_theme_name(&self) -> Option<String> {
        let themes = THEME_REGISTRY.list();
        themes
            .get(self.selected_index)
            .map(|(name, _)| (*name).clone())
    }

    pub fn render(&self, f: &mut Frame, theme: &Theme, current_theme_name: &str) {
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

        // Calculate dynamic visible height (reserve 2 for scroll indicators)
        let visible_height = (chunks[1].height as usize).saturating_sub(2);

        // Title
        let title_lines = popup_title("Select Theme", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Theme list
        let themes = THEME_REGISTRY.list();
        let mut lines: Vec<Line> = Vec::new();

        // Scroll up indicator
        if self.scroll_offset > 0 {
            lines.push(scroll_indicator("up", self.scroll_offset, theme));
        }

        // Visible themes
        let visible_end = (self.scroll_offset + visible_height).min(themes.len());
        for (idx, (name, t)) in themes
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(visible_height)
        {
            let is_selected = idx == self.selected_index;
            let is_current = *name == current_theme_name;

            let prefix = if is_selected { "  › " } else { "    " };

            let display_name = if is_current {
                format!("{} (current)", t.display_name)
            } else {
                t.display_name.clone()
            };

            let name_style = if is_selected {
                Style::default()
                    .fg(if is_current {
                        theme.success_color
                    } else {
                        theme.accent_color
                    })
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(theme.success_color)
            } else {
                Style::default().fg(theme.text_color)
            };

            // Theme name + color preview blocks
            lines.push(Line::from(vec![
                Span::styled(prefix.to_string(), name_style),
                Span::styled(format!("{:<20}", display_name), name_style),
                Span::raw(" "),
                Span::styled("██", Style::default().fg(t.bg_color)),
                Span::styled("██", Style::default().fg(t.border_color)),
                Span::styled("██", Style::default().fg(t.accent_color)),
                Span::styled("██", Style::default().fg(t.text_color)),
                Span::styled("██", Style::default().fg(t.success_color)),
            ]));
        }

        // Scroll down indicator
        let remaining = themes.len().saturating_sub(visible_end);
        if remaining > 0 {
            lines.push(scroll_indicator("down", remaining, theme));
        }

        let content = Paragraph::new(lines).style(Style::default().bg(theme.bg_color));
        let content_area = center_content(chunks[1], 4);
        f.render_widget(content, content_area);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "↑/↓",
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
            Span::styled(": apply  ", Style::default().fg(theme.text_color)),
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
}
