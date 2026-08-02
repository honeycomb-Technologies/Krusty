//! Process list popup - view and manage running background processes

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::common::{
    center_rect, popup_block, popup_title, render_popup_background, scroll_indicator, PopupSize,
};
use crate::process::{ProcessInfo, ProcessStatus};
use crate::tui::themes::Theme;
use crate::tui::utils::truncate_ellipsis;

/// Process list popup state
pub struct ProcessListPopup {
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub processes: Vec<ProcessInfo>,
}

impl Default for ProcessListPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessListPopup {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            scroll_offset: 0,
            processes: Vec::new(),
        }
    }

    pub fn update(&mut self, mut processes: Vec<ProcessInfo>) {
        // Sort: running/suspended first, then by start time (newest first)
        processes.sort_by(|a, b| {
            let a_active = matches!(a.status, ProcessStatus::Running | ProcessStatus::Suspended);
            let b_active = matches!(b.status, ProcessStatus::Running | ProcessStatus::Suspended);
            match (a_active, b_active) {
                (true, true) => b.started_at.cmp(&a.started_at),
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (false, false) => b.started_at.cmp(&a.started_at),
            }
        });
        self.processes = processes;

        // Clamp selection
        if self.selected_index >= self.processes.len() && !self.processes.is_empty() {
            self.selected_index = self.processes.len() - 1;
        }
    }

    pub fn next(&mut self) {
        if self.selected_index < self.processes.len().saturating_sub(1) {
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
        self.ensure_visible_with_height(10); // Default fallback
    }

    fn ensure_visible_with_height(&mut self, visible_height: usize) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index - visible_height + 1;
        }
    }

    pub fn get_selected(&self) -> Option<&ProcessInfo> {
        self.processes.get(self.selected_index)
    }

    pub fn render(&self, f: &mut Frame, theme: &Theme) {
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

        // Calculate dynamic visible height (reserve 4 for scroll indicators + spacing)
        let visible_height = (chunks[1].height as usize).saturating_sub(4);

        // Title
        let title_lines = popup_title("Background Processes", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Process list
        let mut lines: Vec<Line> = Vec::new();

        if self.processes.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No background processes".to_string(),
                Style::default()
                    .fg(theme.dim_color)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else {
            // Scroll up indicator
            if self.scroll_offset > 0 {
                lines.push(scroll_indicator("up", self.scroll_offset, theme));
                lines.push(Line::from(""));
            }

            // Visible processes
            let visible_end = (self.scroll_offset + visible_height).min(self.processes.len());
            for idx in self.scroll_offset..visible_end {
                let proc = &self.processes[idx];
                let is_selected = idx == self.selected_index;

                let prefix = "  ";

                let status_color = match &proc.status {
                    ProcessStatus::Running => theme.success_color,
                    ProcessStatus::Suspended => theme.warning_color,
                    ProcessStatus::Completed { .. } => theme.dim_color,
                    ProcessStatus::Failed { .. } => theme.error_color,
                    ProcessStatus::Killed { .. } => theme.warning_color,
                };

                let status_char = match &proc.status {
                    ProcessStatus::Running => "●",
                    ProcessStatus::Suspended => "⏸",
                    ProcessStatus::Completed { .. } => "✓",
                    ProcessStatus::Failed { .. } => "✗",
                    ProcessStatus::Killed { .. } => "○",
                };

                let duration = format_duration(proc.duration());

                let display_text = proc.description.as_ref().unwrap_or(&proc.command);
                let truncated = truncate_ellipsis(display_text, 40);

                let style = if is_selected {
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_color)
                };

                let selector = if is_selected { "▶ " } else { "  " };

                lines.push(Line::from(vec![
                    Span::styled(prefix.to_string(), style),
                    Span::styled(selector.to_string(), style),
                    Span::styled(status_char.to_string(), Style::default().fg(status_color)),
                    Span::styled(" ".to_string(), Style::default()),
                    Span::styled(truncated, style),
                    Span::styled(
                        format!(" ({})", duration),
                        Style::default().fg(theme.dim_color),
                    ),
                ]));
            }

            // Scroll down indicator
            let remaining = self.processes.len().saturating_sub(visible_end);
            if remaining > 0 {
                lines.push(Line::from(""));
                lines.push(scroll_indicator("down", remaining, theme));
            }
        }

        let content = Paragraph::new(lines).style(Style::default().bg(theme.bg_color));
        f.render_widget(content, chunks[1]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "↑↓",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": nav  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "s",
                Style::default()
                    .fg(theme.warning_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": suspend  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "d",
                Style::default()
                    .fg(theme.error_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": kill  ", Style::default().fg(theme.text_color)),
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

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}
