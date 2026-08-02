//! Skills browser popup
//!
//! Browse and manage available skills.

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
use mitsuro_core::skills::{SkillInfo, SkillPermission, SkillSource};

/// Skills browser popup state
pub struct SkillsBrowserPopup {
    pub skills: Vec<SkillInfo>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub search_query: String,
    pub search_active: bool,
}

impl Default for SkillsBrowserPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillsBrowserPopup {
    pub fn new() -> Self {
        Self {
            skills: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            search_query: String::new(),
            search_active: false,
        }
    }

    /// Set the skills list
    pub fn set_skills(&mut self, skills: Vec<SkillInfo>) {
        self.skills = skills;
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Replace the list while keeping the same selected skill when possible.
    pub fn set_skills_preserving(&mut self, skills: Vec<SkillInfo>, selected: Option<&str>) {
        self.skills = skills;
        self.selected_index = selected
            .and_then(|name| {
                self.filtered_skills()
                    .iter()
                    .position(|(_, skill)| skill.name == name)
            })
            .unwrap_or(0);
        self.scroll_offset = 0;
        self.ensure_visible();
    }

    pub fn selected_skill(&self) -> Option<&SkillInfo> {
        self.filtered_skills()
            .get(self.selected_index)
            .map(|(_, skill)| *skill)
    }

    /// Navigate to next skill
    pub fn next(&mut self) {
        let filtered = self.filtered_skills();
        if self.selected_index < filtered.len().saturating_sub(1) {
            self.selected_index += 1;
            self.ensure_visible();
        }
    }

    /// Navigate to previous skill
    pub fn prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.ensure_visible();
        }
    }

    fn ensure_visible(&mut self) {
        self.ensure_visible_with_height(8);
    }

    fn ensure_visible_with_height(&mut self, visible_height: usize) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index - visible_height + 1;
        }
    }

    /// Toggle search mode
    pub fn toggle_search(&mut self) {
        self.search_active = !self.search_active;
        if !self.search_active {
            self.search_query.clear();
            self.selected_index = 0;
            self.scroll_offset = 0;
        }
    }

    /// Add character to search query
    pub fn add_search_char(&mut self, c: char) {
        if self.search_active {
            self.search_query.push(c);
            self.selected_index = 0;
            self.scroll_offset = 0;
        }
    }

    /// Handle backspace in search
    pub fn backspace_search(&mut self) {
        if self.search_active {
            self.search_query.pop();
        }
    }

    /// Get filtered skills based on search query
    fn filtered_skills(&self) -> Vec<(usize, &SkillInfo)> {
        if self.search_query.is_empty() {
            self.skills.iter().enumerate().collect()
        } else {
            let query = self.search_query.to_lowercase();
            self.skills
                .iter()
                .enumerate()
                .filter(|(_, skill)| {
                    skill.name.to_lowercase().contains(&query)
                        || skill.description.to_lowercase().contains(&query)
                        || skill.origin.to_lowercase().contains(&query)
                        || skill
                            .tags
                            .iter()
                            .any(|tag| tag.to_lowercase().contains(&query))
                })
                .collect()
        }
    }

    /// Render the popup
    pub fn render(&self, f: &mut Frame, theme: &Theme) {
        let (w, h) = PopupSize::Large.dimensions();
        let area = center_rect(w, h, f.area());
        render_popup_background(f, area, theme);

        let block = popup_block(theme);
        let inner = block.inner(area);
        f.render_widget(block, area);

        // Calculate dynamic heights
        let search_height = if self.search_active { 2 } else { 0 };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),             // Title
                Constraint::Length(search_height), // Search
                Constraint::Min(5),                // Content
                Constraint::Length(2),             // Footer
            ])
            .split(inner);

        let visible_height = (chunks[2].height as usize).saturating_sub(2) / 2;

        // Title
        let filtered = self.filtered_skills();
        let title_text = if !self.search_query.is_empty() {
            format!("Skills ({}/{})", filtered.len(), self.skills.len())
        } else {
            format!("Skills ({})", self.skills.len())
        };
        let title_lines = popup_title(&title_text, theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Search bar
        if self.search_active {
            let search = Paragraph::new(Line::from(vec![
                Span::styled("  Search: ", Style::default().fg(theme.accent_color)),
                Span::styled(&self.search_query, Style::default().fg(theme.text_color)),
                Span::styled(
                    "_",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ]));
            f.render_widget(search, chunks[1]);
        }

        // Skills list
        let mut lines: Vec<Line> = Vec::new();

        if self.skills.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "  No skills installed.",
                Style::default().fg(theme.dim_color),
            )]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "  Add skills to:",
                Style::default().fg(theme.text_color),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "  • ~/.mitsuro/skills/<name>/SKILL.md (global)",
                Style::default().fg(theme.dim_color),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "  • .mitsuro/skills/<name>/SKILL.md (project)",
                Style::default().fg(theme.dim_color),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "  • .agents/skills/<name>/SKILL.md (compatible)",
                Style::default().fg(theme.dim_color),
            )]));
        } else if filtered.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "  No skills match your search.",
                Style::default().fg(theme.dim_color),
            )]));
        } else {
            // Scroll up indicator
            if self.scroll_offset > 0 {
                lines.push(scroll_indicator("up", self.scroll_offset, theme));
            }

            // Visible skills
            let visible_end = (self.scroll_offset + visible_height).min(filtered.len());
            for (display_idx, (_, skill)) in filtered
                .iter()
                .enumerate()
                .skip(self.scroll_offset)
                .take(visible_height)
            {
                let is_selected = display_idx == self.selected_index;

                // Source indicator
                let (source_icon, source_color) = match skill.source {
                    SkillSource::Global => ("○", theme.text_color),
                    SkillSource::Project => ("●", theme.success_color),
                    SkillSource::Package => ("◆", theme.accent_color),
                };

                let prefix = if is_selected { " › " } else { "   " };
                let name_style = if !skill.enabled || skill.permission == SkillPermission::Deny {
                    Style::default().fg(theme.dim_color)
                } else if is_selected {
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_color)
                };

                let source_label = match skill.source {
                    SkillSource::Global => "global",
                    SkillSource::Project => "project",
                    SkillSource::Package => "package",
                };
                let status = if !skill.enabled {
                    "disabled"
                } else {
                    skill.permission.as_str()
                };

                lines.push(Line::from(vec![
                    Span::styled(prefix.to_string(), name_style),
                    Span::styled(source_icon.to_string(), Style::default().fg(source_color)),
                    Span::raw(" "),
                    Span::styled(skill.name.clone(), name_style),
                    Span::styled(
                        format!(" [{}:{} · {}]", source_label, skill.origin, status),
                        Style::default().fg(theme.dim_color),
                    ),
                ]));

                // Description line
                let desc = truncate_ellipsis(&skill.description, 70);
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(desc, Style::default().fg(theme.dim_color)),
                ]));
            }

            // Scroll down indicator
            let remaining = filtered.len().saturating_sub(visible_end);
            if remaining > 0 {
                lines.push(scroll_indicator("down", remaining, theme));
            }
        }

        let content = Paragraph::new(lines).style(Style::default().bg(theme.bg_color));
        let content_area = center_content(chunks[2], 4);
        f.render_widget(content, content_area);

        // Footer
        let footer = if self.search_active {
            Paragraph::new(Line::from(vec![
                Span::styled("Type to search  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": close search", Style::default().fg(theme.text_color)),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "/",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": search  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "↑↓",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": nav  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "Enter",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": invoke  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "e",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": enable  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "p",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": policy  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "r",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": refresh  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": close", Style::default().fg(theme.text_color)),
            ]))
        };
        f.render_widget(footer.alignment(Alignment::Center), chunks[3]);
    }
}
