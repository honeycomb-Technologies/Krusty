//! User hooks configuration popup
//!
//! Multi-stage wizard for creating and managing user-defined hooks.
//! Stages: List → SelectType → EnterMatcher → EnterCommand → Confirm

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
use crate::agent::{UserHook, UserHookType};
use crate::tui::themes::Theme;
use crate::tui::utils::truncate_ellipsis;

/// Stages of the hooks configuration wizard
#[derive(Debug, Clone, PartialEq, Default)]
pub enum HooksStage {
    /// List existing hooks with toggle/delete options
    #[default]
    List,
    /// Select hook type (PreToolUse, PostToolUse, etc.)
    SelectType { selected_index: usize },
    /// Enter tool pattern (regex)
    EnterMatcher {
        hook_type: UserHookType,
        input: String,
    },
    /// Enter shell command
    EnterCommand {
        hook_type: UserHookType,
        tool_pattern: String,
        input: String,
    },
    /// Confirm before saving
    Confirm {
        hook_type: UserHookType,
        tool_pattern: String,
        command: String,
    },
}

/// Hooks configuration popup
pub struct HooksPopup {
    /// Current wizard stage
    pub stage: HooksStage,
    /// Cached hooks for display
    pub hooks: Vec<UserHook>,
    /// Selected index in list view
    pub selected_index: usize,
    /// Scroll offset for long lists
    pub scroll_offset: usize,
    /// Error message if any
    pub error: Option<String>,
}

impl Default for HooksPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl HooksPopup {
    pub fn new() -> Self {
        Self {
            stage: HooksStage::List,
            hooks: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            error: None,
        }
    }

    /// Reset to initial state
    pub fn reset(&mut self) {
        self.stage = HooksStage::List;
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.error = None;
    }

    /// Set hooks from manager
    pub fn set_hooks(&mut self, hooks: Vec<UserHook>) {
        self.hooks = hooks;
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    // =========================================================================
    // Navigation
    // =========================================================================

    pub fn next(&mut self) {
        match &self.stage {
            HooksStage::List => {
                // In list, navigate hooks (+ "Add new" option)
                let max = self.hooks.len(); // "Add new" is at the end
                if self.selected_index < max {
                    self.selected_index += 1;
                    self.ensure_visible();
                }
            }
            HooksStage::SelectType { selected_index } => {
                let types = UserHookType::all();
                if *selected_index < types.len() - 1 {
                    self.stage = HooksStage::SelectType {
                        selected_index: selected_index + 1,
                    };
                }
            }
            _ => {}
        }
    }

    pub fn prev(&mut self) {
        match &self.stage {
            HooksStage::List if self.selected_index > 0 => {
                self.selected_index -= 1;
                self.ensure_visible();
            }
            HooksStage::SelectType { selected_index } if *selected_index > 0 => {
                self.stage = HooksStage::SelectType {
                    selected_index: selected_index - 1,
                };
            }
            _ => {}
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

    // =========================================================================
    // Stage transitions
    // =========================================================================

    /// Start adding a new hook
    pub fn start_add(&mut self) {
        self.stage = HooksStage::SelectType { selected_index: 0 };
        self.error = None;
    }

    /// Confirm selection in SelectType stage
    pub fn confirm_type(&mut self) {
        if let HooksStage::SelectType { selected_index } = &self.stage {
            let types = UserHookType::all();
            if let Some(hook_type) = types.get(*selected_index) {
                self.stage = HooksStage::EnterMatcher {
                    hook_type: *hook_type,
                    input: String::new(),
                };
            }
        }
    }

    /// Confirm matcher and move to command
    pub fn confirm_matcher(&mut self) {
        if let HooksStage::EnterMatcher { hook_type, input } = &self.stage {
            if input.is_empty() {
                self.error = Some("Pattern cannot be empty".to_string());
                return;
            }
            // Validate regex
            if regex::Regex::new(input).is_err() {
                self.error = Some("Invalid regex pattern".to_string());
                return;
            }
            self.stage = HooksStage::EnterCommand {
                hook_type: *hook_type,
                tool_pattern: input.clone(),
                input: String::new(),
            };
            self.error = None;
        }
    }

    /// Confirm command and move to confirm stage
    pub fn confirm_command(&mut self) {
        if let HooksStage::EnterCommand {
            hook_type,
            tool_pattern,
            input,
        } = &self.stage
        {
            if input.is_empty() {
                self.error = Some("Command cannot be empty".to_string());
                return;
            }
            self.stage = HooksStage::Confirm {
                hook_type: *hook_type,
                tool_pattern: tool_pattern.clone(),
                command: input.clone(),
            };
            self.error = None;
        }
    }

    /// Get the hook to save (from Confirm stage)
    pub fn get_pending_hook(&self) -> Option<UserHook> {
        if let HooksStage::Confirm {
            hook_type,
            tool_pattern,
            command,
        } = &self.stage
        {
            Some(UserHook::new(
                *hook_type,
                tool_pattern.clone(),
                command.clone(),
            ))
        } else {
            None
        }
    }

    /// Go back one stage
    pub fn go_back(&mut self) {
        self.error = None;
        match &self.stage {
            HooksStage::SelectType { .. } => {
                self.stage = HooksStage::List;
            }
            HooksStage::EnterMatcher { .. } => {
                self.stage = HooksStage::SelectType { selected_index: 0 };
            }
            HooksStage::EnterCommand {
                hook_type,
                tool_pattern,
                ..
            } => {
                self.stage = HooksStage::EnterMatcher {
                    hook_type: *hook_type,
                    input: tool_pattern.clone(),
                };
            }
            HooksStage::Confirm {
                hook_type,
                tool_pattern,
                command,
            } => {
                self.stage = HooksStage::EnterCommand {
                    hook_type: *hook_type,
                    tool_pattern: tool_pattern.clone(),
                    input: command.clone(),
                };
            }
            HooksStage::List => {} // Can't go back from list
        }
    }

    // =========================================================================
    // Text input
    // =========================================================================

    pub fn add_char(&mut self, c: char) {
        match &mut self.stage {
            HooksStage::EnterMatcher { input, .. } => {
                input.push(c);
                self.error = None;
            }
            HooksStage::EnterCommand { input, .. } => {
                input.push(c);
                self.error = None;
            }
            _ => {}
        }
    }

    pub fn backspace(&mut self) {
        match &mut self.stage {
            HooksStage::EnterMatcher { input, .. } => {
                input.pop();
            }
            HooksStage::EnterCommand { input, .. } => {
                input.pop();
            }
            _ => {}
        }
    }

    // =========================================================================
    // List operations
    // =========================================================================

    /// Get selected hook ID (for toggle/delete)
    pub fn get_selected_hook_id(&self) -> Option<&str> {
        if matches!(self.stage, HooksStage::List) {
            self.hooks.get(self.selected_index).map(|h| h.id.as_str())
        } else {
            None
        }
    }

    /// Check if "Add new" is selected
    pub fn is_add_new_selected(&self) -> bool {
        matches!(self.stage, HooksStage::List) && self.selected_index == self.hooks.len()
    }

    // =========================================================================
    // Rendering
    // =========================================================================

    pub fn render(&self, f: &mut Frame, theme: &Theme) {
        match &self.stage {
            HooksStage::List => self.render_list(f, theme),
            HooksStage::SelectType { selected_index } => {
                self.render_select_type(f, theme, *selected_index)
            }
            HooksStage::EnterMatcher { hook_type, input } => {
                self.render_enter_matcher(f, theme, *hook_type, input)
            }
            HooksStage::EnterCommand {
                hook_type,
                tool_pattern,
                input,
            } => self.render_enter_command(f, theme, *hook_type, tool_pattern, input),
            HooksStage::Confirm {
                hook_type,
                tool_pattern,
                command,
            } => self.render_confirm(f, theme, *hook_type, tool_pattern, command),
        }
    }

    fn render_list(&self, f: &mut Frame, theme: &Theme) {
        let (w, h) = PopupSize::Large.dimensions();
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

        // Title
        let title_lines = popup_title("Hooks", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Content - list of hooks + "Add new"
        let mut lines = Vec::new();

        // Calculate visible items (each hook takes 2 lines, plus "Add new" takes 2 lines)
        let content_height = chunks[1].height as usize;
        // Reserve space for scroll indicators and "Add new" section
        let visible_item_height = content_height.saturating_sub(4) / 2; // Each hook takes 2 lines
        let visible_height = visible_item_height.max(1);

        // Scroll indicator (up)
        if self.scroll_offset > 0 {
            lines.push(scroll_indicator("up", self.scroll_offset, theme));
        } else {
            lines.push(Line::from("")); // Maintain spacing
        }

        if self.hooks.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No hooks configured",
                Style::default().fg(theme.dim_color),
            )));
            lines.push(Line::from(""));
        } else {
            for (i, hook) in self
                .hooks
                .iter()
                .enumerate()
                .skip(self.scroll_offset)
                .take(visible_height)
            {
                let is_selected = i == self.selected_index;
                let prefix = if is_selected { "› " } else { "  " };
                let status = if hook.enabled { "●" } else { "○" };
                let status_color = if hook.enabled {
                    theme.success_color
                } else {
                    theme.dim_color
                };

                let style = if is_selected {
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_color)
                };

                lines.push(Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(format!("{} ", status), Style::default().fg(status_color)),
                    Span::styled(format!("{} ", hook.hook_type), style),
                    Span::styled(
                        format!("[{}]", hook.tool_pattern),
                        Style::default().fg(theme.dim_color),
                    ),
                ]));

                // Show command on second line (truncated)
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(
                        truncate_ellipsis(&hook.command, 43).into_owned(),
                        Style::default().fg(theme.dim_color),
                    ),
                ]));
            }

            // Scroll indicator (down) - for hooks only, not including "Add new"
            let remaining = self
                .hooks
                .len()
                .saturating_sub(self.scroll_offset + visible_height);
            if remaining > 0 {
                lines.push(scroll_indicator("down", remaining, theme));
            }
        }

        // "Add new" option (always visible at bottom)
        lines.push(Line::from(""));
        let is_add_selected = self.selected_index == self.hooks.len();
        let add_style = if is_add_selected {
            Style::default()
                .fg(theme.accent_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_color)
        };
        let add_prefix = if is_add_selected { "› " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(add_prefix, add_style),
            Span::styled("+ Add new hook", add_style),
        ]));

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
            Span::styled(": navigate  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": select  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Space",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": toggle  ", Style::default().fg(theme.text_color)),
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
            Span::styled(": close", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[2]);
    }

    fn render_select_type(&self, f: &mut Frame, theme: &Theme, selected_index: usize) {
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

        // Title
        let title_lines = popup_title("Select Hook Type", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Hook type options
        let types = UserHookType::all();
        let mut lines = Vec::new();

        for (i, hook_type) in types.iter().enumerate() {
            let is_selected = i == selected_index;
            let prefix = if is_selected { "  › " } else { "    " };
            let style = if is_selected {
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_color)
            };

            lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("{}. ", i + 1), Style::default().fg(theme.dim_color)),
                Span::styled(hook_type.display_name(), style),
                Span::styled(
                    format!(" - {}", hook_type.description()),
                    Style::default().fg(theme.dim_color),
                ),
            ]));
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
            Span::styled(": confirm  ", Style::default().fg(theme.text_color)),
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

    fn render_enter_matcher(
        &self,
        f: &mut Frame,
        theme: &Theme,
        hook_type: UserHookType,
        input: &str,
    ) {
        let (w, h) = PopupSize::Large.dimensions();
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
                Constraint::Length(2), // Hook type
                Constraint::Length(3), // Input
                Constraint::Length(2), // Error
                Constraint::Min(3),    // Hints
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_lines = popup_title("Enter Tool Matcher", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Hook type indicator
        let type_line = Paragraph::new(Line::from(vec![
            Span::styled("Hook type: ", Style::default().fg(theme.dim_color)),
            Span::styled(
                hook_type.display_name(),
                Style::default().fg(theme.accent_color),
            ),
        ]));
        f.render_widget(type_line, chunks[1]);

        // Input field
        let input_block = Block::default()
            .title("Tool pattern (regex)")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_color));

        let display_text = if input.is_empty() { ".*" } else { input };
        let text_style = if input.is_empty() {
            Style::default().fg(theme.dim_color)
        } else {
            Style::default().fg(theme.text_color)
        };

        let input_widget = Paragraph::new(display_text)
            .style(text_style)
            .block(input_block);
        f.render_widget(input_widget, chunks[2]);

        // Error message
        if let Some(err) = &self.error {
            let error_widget = Paragraph::new(err.as_str())
                .style(Style::default().fg(theme.error_color))
                .alignment(Alignment::Center);
            f.render_widget(error_widget, chunks[3]);
        }

        // Available tools hint
        let mut hint_lines = vec![Line::from(Span::styled(
            "Example matchers:",
            Style::default().fg(theme.dim_color),
        ))];
        hint_lines.push(Line::from(vec![
            Span::styled("  Write", Style::default().fg(theme.text_color)),
            Span::styled(" (single tool)", Style::default().fg(theme.dim_color)),
        ]));
        hint_lines.push(Line::from(vec![
            Span::styled("  Write|Edit", Style::default().fg(theme.text_color)),
            Span::styled(" (multiple tools)", Style::default().fg(theme.dim_color)),
        ]));
        hint_lines.push(Line::from(vec![
            Span::styled("  .*", Style::default().fg(theme.text_color)),
            Span::styled(" (all tools)", Style::default().fg(theme.dim_color)),
        ]));

        let hints = Paragraph::new(hint_lines);
        f.render_widget(hints, chunks[4]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": continue  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": back", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[5]);
    }

    fn render_enter_command(
        &self,
        f: &mut Frame,
        theme: &Theme,
        hook_type: UserHookType,
        tool_pattern: &str,
        input: &str,
    ) {
        let (w, h) = PopupSize::Large.dimensions();
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
                Constraint::Length(3), // Config summary
                Constraint::Length(3), // Input
                Constraint::Length(2), // Error
                Constraint::Min(3),    // Info
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_lines = popup_title("Enter Command", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Config summary
        let summary = Paragraph::new(vec![Line::from(vec![
            Span::styled("Type: ", Style::default().fg(theme.dim_color)),
            Span::styled(
                hook_type.display_name(),
                Style::default().fg(theme.accent_color),
            ),
            Span::styled("  Pattern: ", Style::default().fg(theme.dim_color)),
            Span::styled(tool_pattern, Style::default().fg(theme.accent_color)),
        ])]);
        f.render_widget(summary, chunks[1]);

        // Input field
        let input_block = Block::default()
            .title("Shell command")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_color));

        let display_text = if input.is_empty() {
            "./my-hook.sh"
        } else {
            input
        };
        let text_style = if input.is_empty() {
            Style::default().fg(theme.dim_color)
        } else {
            Style::default().fg(theme.text_color)
        };

        let input_widget = Paragraph::new(display_text)
            .style(text_style)
            .block(input_block);
        f.render_widget(input_widget, chunks[2]);

        // Error message
        if let Some(err) = &self.error {
            let error_widget = Paragraph::new(err.as_str())
                .style(Style::default().fg(theme.error_color))
                .alignment(Alignment::Center);
            f.render_widget(error_widget, chunks[3]);
        }

        // Exit code info
        let info_lines = vec![
            Line::from(Span::styled(
                "Exit code protocol:",
                Style::default().fg(theme.dim_color),
            )),
            Line::from(vec![
                Span::styled("  0", Style::default().fg(theme.success_color)),
                Span::styled(" = continue silently", Style::default().fg(theme.dim_color)),
            ]),
            Line::from(vec![
                Span::styled("  2", Style::default().fg(theme.error_color)),
                Span::styled(
                    " = block tool, show stderr to model",
                    Style::default().fg(theme.dim_color),
                ),
            ]),
            Line::from(vec![
                Span::styled("  other", Style::default().fg(theme.warning_color)),
                Span::styled(
                    " = warn user, continue",
                    Style::default().fg(theme.dim_color),
                ),
            ]),
        ];

        let info = Paragraph::new(info_lines);
        f.render_widget(info, chunks[4]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": continue  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": back", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[5]);
    }

    fn render_confirm(
        &self,
        f: &mut Frame,
        theme: &Theme,
        hook_type: UserHookType,
        tool_pattern: &str,
        command: &str,
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
                Constraint::Min(6),    // Summary
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_lines = popup_title("Confirm Hook", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Summary
        let summary_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Type:    ", Style::default().fg(theme.dim_color)),
                Span::styled(
                    hook_type.display_name(),
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Pattern: ", Style::default().fg(theme.dim_color)),
                Span::styled(tool_pattern, Style::default().fg(theme.text_color)),
            ]),
            Line::from(vec![
                Span::styled("Command: ", Style::default().fg(theme.dim_color)),
                Span::styled(command, Style::default().fg(theme.text_color)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter to save, Esc to go back",
                Style::default().fg(theme.dim_color),
            )),
        ];

        let summary = Paragraph::new(summary_lines);
        f.render_widget(summary, chunks[1]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
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
            Span::styled(": back", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[2]);
    }
}
