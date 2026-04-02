//! MCP server browser popup
//!
//! Browse and manage MCP servers.

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
use crate::tui::themes::Theme;
use crate::tui::utils::truncate_ellipsis;
use krusty_core::mcp::{McpServerInfo, McpServerStatus, McpToolDef};

/// MCP browser popup state
pub struct McpBrowserPopup {
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub servers: Vec<McpServerInfo>,
    pub expanded_server: Option<String>,
    pub status_message: Option<String>,
}

impl Default for McpBrowserPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl McpBrowserPopup {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            scroll_offset: 0,
            servers: Vec::new(),
            expanded_server: None,
            status_message: None,
        }
    }

    pub fn update(&mut self, servers: Vec<McpServerInfo>) {
        self.servers = servers;
        if self.selected_index >= self.servers.len() && !self.servers.is_empty() {
            self.selected_index = self.servers.len() - 1;
        }
    }

    pub fn next(&mut self) {
        if self.selected_index < self.servers.len().saturating_sub(1) {
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
        let visible_height = 8;
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index - visible_height + 1;
        }
    }

    pub fn toggle_expand(&mut self) {
        if let Some(server) = self.get_selected() {
            let name = server.name.clone();
            if self.expanded_server.as_ref() == Some(&name) {
                self.expanded_server = None;
            } else {
                self.expanded_server = Some(name);
            }
        }
    }

    pub fn get_selected(&self) -> Option<&McpServerInfo> {
        self.servers.get(self.selected_index)
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
    }

    pub fn render(&self, f: &mut Frame, theme: &Theme) {
        let (w, h) = PopupSize::Large.dimensions();
        let area = center_rect(w, h, f.area());
        render_popup_background(f, area, theme);

        let block = popup_block(theme);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let status_height = if self.status_message.is_some() { 2 } else { 0 };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(status_height),
                Constraint::Min(5),
                Constraint::Length(2),
            ])
            .split(inner);

        // Title
        let connected = self
            .servers
            .iter()
            .filter(|s| matches!(s.status, McpServerStatus::Connected))
            .count();
        let title_text = if self.servers.is_empty() {
            "MCP Servers - No servers configured".to_string()
        } else {
            format!(
                "MCP Servers ({} total, {} connected)",
                self.servers.len(),
                connected
            )
        };
        let title = Paragraph::new(popup_title(&title_text, theme)).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Status message
        if let Some(ref status) = self.status_message {
            let color = if status.starts_with('✓') {
                theme.success_color
            } else if status.starts_with('✗') {
                theme.error_color
            } else {
                theme.warning_color
            };
            let status_widget = Paragraph::new(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(status.clone(), Style::default().fg(color)),
            ]));
            f.render_widget(status_widget, chunks[1]);
        }

        // Content
        let visible_height = (chunks[2].height as usize).saturating_sub(2);
        let mut lines: Vec<Line> = Vec::new();

        if self.servers.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "  No MCP servers configured.",
                Style::default().fg(theme.dim_color),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "  Create .mcp.json in your project root to add servers.",
                Style::default().fg(theme.dim_color),
            )]));
        } else {
            if self.scroll_offset > 0 {
                lines.push(scroll_indicator("up", self.scroll_offset, theme));
            }

            let mut displayed = 0;
            for (idx, server) in self.servers.iter().enumerate().skip(self.scroll_offset) {
                if displayed >= visible_height {
                    break;
                }

                let is_selected = idx == self.selected_index;
                let is_expanded = self.expanded_server.as_ref() == Some(&server.name);

                lines.push(render_server_line(server, is_selected, theme));
                displayed += 1;

                if is_expanded && displayed < visible_height {
                    for (tool_idx, tool) in server.tools.iter().enumerate() {
                        if displayed >= visible_height {
                            break;
                        }
                        let is_last = tool_idx == server.tools.len() - 1;
                        lines.push(render_tool_line(tool, is_last, theme));
                        displayed += 1;
                    }
                }
            }

            let remaining = self
                .servers
                .len()
                .saturating_sub(self.scroll_offset + visible_height);
            if remaining > 0 {
                lines.push(scroll_indicator("down", remaining, theme));
            }
        }

        let content = Paragraph::new(lines).style(Style::default().bg(theme.bg_color));
        f.render_widget(content, center_content(chunks[2], 4));

        // Footer - compact
        let footer = if self.servers.is_empty() {
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" close", Style::default().fg(theme.dim_color)),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "↑↓",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" nav  ", Style::default().fg(theme.dim_color)),
                Span::styled(
                    "⏎",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" expand  ", Style::default().fg(theme.dim_color)),
                Span::styled(
                    "c",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" connect  ", Style::default().fg(theme.dim_color)),
                Span::styled(
                    "d",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" disconnect  ", Style::default().fg(theme.dim_color)),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" close", Style::default().fg(theme.dim_color)),
            ]))
        };
        f.render_widget(footer.alignment(Alignment::Center), chunks[3]);
    }
}

fn render_server_line<'a>(server: &McpServerInfo, is_selected: bool, theme: &Theme) -> Line<'a> {
    let (icon, icon_color) = match &server.status {
        McpServerStatus::Connected => ("●", theme.success_color),
        McpServerStatus::Disconnected => ("○", theme.dim_color),
        McpServerStatus::Error(_) => ("✗", theme.error_color),
    };

    let prefix = if is_selected { " › " } else { "   " };
    let name_style = if is_selected {
        Style::default()
            .fg(theme.accent_color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text_color)
    };

    let status_text = match &server.status {
        McpServerStatus::Connected => format!("{} tools", server.tool_count),
        McpServerStatus::Disconnected => "disconnected".to_string(),
        McpServerStatus::Error(e) => {
            let msg = server.error.as_deref().unwrap_or(e);
            if msg.chars().count() > 35 {
                format!("error: {}", truncate_ellipsis(msg, 35))
            } else {
                format!("error: {}", msg)
            }
        }
    };

    Line::from(vec![
        Span::styled(prefix.to_string(), name_style),
        Span::styled(icon.to_string(), Style::default().fg(icon_color)),
        Span::raw(" "),
        Span::styled(server.name.clone(), name_style),
        Span::styled(
            format!(" ({})", server.server_type),
            Style::default().fg(theme.dim_color),
        ),
        Span::styled(
            format!(" - {}", status_text),
            Style::default().fg(theme.dim_color),
        ),
    ])
}

fn render_tool_line<'a>(tool: &McpToolDef, is_last: bool, theme: &Theme) -> Line<'a> {
    let prefix = if is_last {
        "    └─ "
    } else {
        "    ├─ "
    };

    let desc = tool
        .description
        .as_deref()
        .map(|d| {
            if d.chars().count() > 40 {
                format!(" - {}", truncate_ellipsis(d, 40))
            } else {
                format!(" - {}", d)
            }
        })
        .unwrap_or_default();

    Line::from(vec![
        Span::styled(prefix.to_string(), Style::default().fg(theme.dim_color)),
        Span::styled(tool.name.clone(), Style::default().fg(theme.text_color)),
        Span::styled(desc, Style::default().fg(theme.dim_color)),
    ])
}
