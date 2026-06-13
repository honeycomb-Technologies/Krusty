//! Help popup with tabbed content

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::common::{
    center_rect, popup_block, render_popup_background, scroll_indicator, PopupSize,
};
use crate::tui::themes::Theme;

/// Help popup state
pub struct HelpPopup {
    pub tab_index: usize,
    pub scroll_offset: usize,
}

impl Default for HelpPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl HelpPopup {
    pub fn new() -> Self {
        Self {
            tab_index: 0,
            scroll_offset: 0,
        }
    }

    pub fn next_tab(&mut self) {
        self.tab_index = (self.tab_index + 1) % 2;
        self.scroll_offset = 0;
    }

    pub fn render(&self, f: &mut Frame, theme: &Theme) {
        let (w, h) = PopupSize::Large.dimensions();
        let area = center_rect(w, h, f.area());
        render_popup_background(f, area, theme);

        let block = popup_block(theme);
        let inner = block.inner(area);
        f.render_widget(block, area);

        // Tab headers
        let tabs = ["Commands", "Keybinds"];
        let tab_spans: Vec<Span> = tabs
            .iter()
            .enumerate()
            .flat_map(|(i, tab)| {
                let mut spans = Vec::new();
                if i > 0 {
                    spans.push(Span::styled(" | ", Style::default().fg(theme.text_color)));
                }
                let style = if i == self.tab_index {
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(theme.text_color)
                };
                spans.push(Span::styled(tab.to_string(), style));
                spans
            })
            .collect();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Tabs
                Constraint::Min(5),    // Content
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Render tabs
        let tabs_widget = Paragraph::new(Line::from(tab_spans)).alignment(Alignment::Center);
        f.render_widget(tabs_widget, chunks[0]);

        // Content based on tab with scroll indicators
        let all_content = match self.tab_index {
            0 => self.commands_content(theme),
            1 => self.keybinds_content(theme),
            _ => vec![],
        };

        let total_lines = all_content.len();
        // Reserve space for scroll indicators
        let visible_height = (chunks[1].height as usize).saturating_sub(2);

        let mut display_lines: Vec<Line> = Vec::new();

        // Scroll indicator (up)
        if self.scroll_offset > 0 {
            display_lines.push(scroll_indicator("up", self.scroll_offset, theme));
        }

        // Visible content
        for line in all_content
            .into_iter()
            .skip(self.scroll_offset)
            .take(visible_height)
        {
            display_lines.push(line);
        }

        // Scroll indicator (down)
        let remaining = total_lines.saturating_sub(self.scroll_offset + visible_height);
        if remaining > 0 {
            display_lines.push(scroll_indicator("down", remaining, theme));
        }

        let content_widget =
            Paragraph::new(display_lines).style(Style::default().bg(theme.bg_color));
        f.render_widget(content_widget, chunks[1]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "Tab",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": switch tabs  ", Style::default().fg(theme.text_color)),
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

    fn commands_content(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from("")];

        let commands = [
            ("/home", "Return to start menu"),
            ("/load", "Load previous session"),
            ("/model", "Select AI model"),
            ("/fast", "Toggle fast service tier"),
            ("/auth", "Manage API providers"),
            ("/theme", "Change color theme"),
            ("/clear", "Clear chat messages"),
            ("/pinch", "Compact this session in place"),
            ("/plan", "View/manage active plan"),
            ("/mcp", "Browse and manage MCP servers"),
            ("/skills", "Browse skills"),
            ("/plugins", "Browse and manage installable plugins"),
            ("/ps", "View background processes"),
            ("/terminal", "Open interactive terminal"),
            ("/init", "Generate KRAB.md"),
            ("/permissions", "Toggle supervised/autonomous mode"),
            ("/cmd", "Show this help"),
        ];

        for (cmd, desc) in commands {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<12}", cmd),
                    Style::default().fg(theme.accent_color),
                ),
                Span::styled(desc.to_string(), Style::default().fg(theme.text_color)),
            ]));
        }

        lines
    }

    fn keybinds_content(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from("")];

        let sections = [
            (
                "Global",
                vec![
                    ("Ctrl+Q", "Quit"),
                    ("Ctrl+G", "Toggle BUILD/PLAN mode"),
                    ("Ctrl+T", "Toggle plan sidebar"),
                    ("Ctrl+B", "Open process list"),
                    ("Ctrl+P", "Toggle plugin window"),
                    ("Tab", "Cycle thinking intensity"),
                    ("Esc", "Cancel AI / close popup"),
                ],
            ),
            (
                "Input",
                vec![
                    ("Enter", "Send message"),
                    ("Shift+Enter", "New line"),
                    ("Ctrl+V", "Paste text or image"),
                    ("Ctrl+C", "Clear input"),
                    ("Ctrl+W", "Delete word"),
                    ("@", "Search files to attach"),
                ],
            ),
            (
                "Navigation",
                vec![("↑/↓", "Autocomplete"), ("PgUp/Dn", "Scroll chat")],
            ),
        ];

        for (section, bindings) in sections {
            lines.push(Line::from(Span::styled(
                format!("{}:", section),
                Style::default()
                    .fg(theme.title_color)
                    .add_modifier(Modifier::BOLD),
            )));

            for (key, desc) in bindings {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<14}", key),
                        Style::default().fg(theme.accent_color),
                    ),
                    Span::styled(desc.to_string(), Style::default().fg(theme.text_color)),
                ]));
            }
            lines.push(Line::from(""));
        }

        lines
    }
}
