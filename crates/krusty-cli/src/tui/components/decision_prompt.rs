//! Decision prompt widget
//!
//! A unified prompt widget for user decisions:
//! - Plan confirmation (Execute/Modify/Abandon)
//! - AskUserQuestion tool (Claude's questions with options)

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Widget},
};

use crate::tui::themes::Theme;

/// Type of prompt being shown
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptType {
    /// Plan confirmation after plan creation
    PlanConfirm,
    /// AskUserQuestion tool from Claude
    AskUserQuestion,
}

/// A single option in a question
#[derive(Debug, Clone)]
pub struct PromptOption {
    /// Display label
    pub label: String,
    /// Optional description
    pub description: Option<String>,
}

impl PromptOption {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

/// A single question with options
#[derive(Debug, Clone)]
pub struct PromptQuestion {
    /// Short header for title bar
    pub header: String,
    /// Full question text
    pub question: String,
    /// Available options
    pub options: Vec<PromptOption>,
    /// Allow multiple selections
    pub multi_select: bool,
}

impl PromptQuestion {
    pub fn new(header: impl Into<String>, question: impl Into<String>) -> Self {
        Self {
            header: header.into(),
            question: question.into(),
            options: Vec::new(),
            multi_select: false,
        }
    }

    pub fn add_option(mut self, option: PromptOption) -> Self {
        self.options.push(option);
        self
    }
}

/// User's answer to a question
#[derive(Debug, Clone)]
pub enum PromptAnswer {
    /// Single option selected (index)
    Selected(usize),
    /// Multiple options selected (indices)
    MultiSelected(Vec<usize>),
    /// Custom text response
    Custom(String),
}

/// Decision prompt state
#[derive(Debug, Clone)]
pub struct DecisionPrompt {
    /// Whether the prompt is visible
    pub visible: bool,
    /// All questions to ask
    pub questions: Vec<PromptQuestion>,
    /// Current question index
    pub current_index: usize,
    /// Currently highlighted option (0-indexed)
    pub selected_option: usize,
    /// For multi-select: which options are toggled
    pub toggled_options: Vec<usize>,
    /// Collected answers
    pub answers: Vec<PromptAnswer>,
    /// Type of prompt
    pub prompt_type: PromptType,
    /// Tool use ID (for AskUserQuestion result)
    pub tool_use_id: Option<String>,
    /// Whether user is typing custom response
    pub custom_input_mode: bool,
    /// Scroll offset for options (when more than fit on screen)
    pub scroll_offset: usize,
}

impl Default for DecisionPrompt {
    fn default() -> Self {
        Self {
            visible: false,
            questions: Vec::new(),
            current_index: 0,
            selected_option: 0,
            toggled_options: Vec::new(),
            answers: Vec::new(),
            prompt_type: PromptType::AskUserQuestion,
            tool_use_id: None,
            custom_input_mode: false,
            scroll_offset: 0,
        }
    }
}

impl DecisionPrompt {
    /// Show plan confirmation prompt
    pub fn show_plan_confirm(&mut self, title: &str, task_count: usize) {
        self.questions = vec![PromptQuestion::new(
            "Execute Plan?",
            format!("Plan: \"{}\" ({} tasks)", title, task_count),
        )
        .add_option(PromptOption::new("Execute").with_description("Switch to BUILD mode and start"))
        .add_option(PromptOption::new("Abandon").with_description("Discard plan"))];

        self.current_index = 0;
        self.selected_option = 0;
        self.scroll_offset = 0;
        self.toggled_options.clear();
        self.answers.clear();
        self.prompt_type = PromptType::PlanConfirm;
        self.tool_use_id = None;
        self.custom_input_mode = false;
        self.visible = true;
    }

    /// Show AskUserQuestion prompt
    pub fn show_ask_user(&mut self, questions: Vec<PromptQuestion>, tool_use_id: String) {
        self.questions = questions;
        self.current_index = 0;
        self.selected_option = 0;
        self.scroll_offset = 0;
        self.toggled_options.clear();
        self.answers.clear();
        self.prompt_type = PromptType::AskUserQuestion;
        self.tool_use_id = Some(tool_use_id);
        self.custom_input_mode = false;
        self.visible = true;
    }

    /// Hide the prompt
    pub fn hide(&mut self) {
        self.visible = false;
        self.custom_input_mode = false;
    }

    /// Get current question
    pub fn current_question(&self) -> Option<&PromptQuestion> {
        self.questions.get(self.current_index)
    }

    /// Maximum visible options (widget height cap minus overhead)
    const MAX_VISIBLE_OPTIONS: usize = 8;

    /// Get current question's option count
    fn option_count(&self) -> usize {
        self.questions
            .get(self.current_index)
            .map(|q| q.options.len())
            .unwrap_or(0)
    }

    /// Move to next option
    pub fn next_option(&mut self) {
        let count = self.option_count();
        if count > 0 {
            self.selected_option = (self.selected_option + 1) % count;
            self.ensure_selected_visible(count);
        }
    }

    /// Move to previous option
    pub fn prev_option(&mut self) {
        let count = self.option_count();
        if count > 0 {
            self.selected_option = self.selected_option.checked_sub(1).unwrap_or(count - 1);
            self.ensure_selected_visible(count);
        }
    }

    /// Ensure selected option is visible (auto-scroll)
    fn ensure_selected_visible(&mut self, total_options: usize) {
        let max_visible = Self::MAX_VISIBLE_OPTIONS.min(total_options);

        // Scroll down if selected is below visible area
        if self.selected_option >= self.scroll_offset + max_visible {
            self.scroll_offset = self.selected_option - max_visible + 1;
        }
        // Scroll up if selected is above visible area
        if self.selected_option < self.scroll_offset {
            self.scroll_offset = self.selected_option;
        }
    }

    /// Scroll up one page
    pub fn page_up(&mut self) {
        let count = self.option_count();
        if count > 0 {
            let page_size = Self::MAX_VISIBLE_OPTIONS.min(count);
            self.selected_option = self.selected_option.saturating_sub(page_size);
            self.ensure_selected_visible(count);
        }
    }

    /// Scroll down one page
    pub fn page_down(&mut self) {
        let count = self.option_count();
        if count > 0 {
            let page_size = Self::MAX_VISIBLE_OPTIONS.min(count);
            self.selected_option = (self.selected_option + page_size).min(count - 1);
            self.ensure_selected_visible(count);
        }
    }

    /// Select option by number (1-indexed)
    pub fn select_by_number(&mut self, num: usize) -> bool {
        if let Some(q) = self.current_question() {
            if num > 0 && num <= q.options.len() {
                self.selected_option = num - 1;
                return true;
            }
        }
        false
    }

    /// Toggle option for multi-select
    pub fn toggle_current(&mut self) {
        if let Some(q) = self.current_question() {
            if q.multi_select {
                if self.toggled_options.contains(&self.selected_option) {
                    self.toggled_options.retain(|&x| x != self.selected_option);
                } else {
                    self.toggled_options.push(self.selected_option);
                }
            }
        }
    }

    /// Confirm current selection and advance
    /// Returns true if all questions answered
    pub fn confirm_selection(&mut self) -> bool {
        let answer = if let Some(q) = self.current_question() {
            if q.multi_select {
                PromptAnswer::MultiSelected(self.toggled_options.clone())
            } else {
                PromptAnswer::Selected(self.selected_option)
            }
        } else {
            return true;
        };

        self.answers.push(answer);
        self.current_index += 1;
        self.selected_option = 0;
        self.toggled_options.clear();

        self.current_index >= self.questions.len()
    }

    /// Submit custom text response
    /// Returns true if all questions answered
    pub fn submit_custom(&mut self, text: String) -> bool {
        self.answers.push(PromptAnswer::Custom(text));
        self.current_index += 1;
        self.selected_option = 0;
        self.toggled_options.clear();
        self.custom_input_mode = false;

        self.current_index >= self.questions.len()
    }

    /// Go back to previous question
    pub fn go_back(&mut self) -> bool {
        if self.current_index > 0 {
            self.current_index -= 1;
            self.answers.pop();
            self.selected_option = 0;
            self.toggled_options.clear();
            true
        } else {
            false
        }
    }

    /// Enter custom input mode
    pub fn enter_custom_mode(&mut self) {
        self.custom_input_mode = true;
    }

    /// Exit custom input mode
    pub fn exit_custom_mode(&mut self) {
        self.custom_input_mode = false;
    }

    /// Calculate widget height
    pub fn calculate_height(&self) -> u16 {
        if !self.visible {
            return 0;
        }

        let Some(q) = self.current_question() else {
            return 0;
        };

        // Border (2) + question (1) + blank (1) + options (capped) + footer hint (1) + padding (1)
        let visible_options = q.options.len().min(Self::MAX_VISIBLE_OPTIONS) as u16;
        2 + 1 + 1 + visible_options + 1 + 1
    }

    /// Render the decision prompt
    pub fn render(&self, buf: &mut Buffer, area: Rect, theme: &Theme) {
        if !self.visible || area.height < 5 || area.width < 20 {
            return;
        }

        let Some(question) = self.current_question() else {
            return;
        };

        // Build title
        let title = if self.questions.len() > 1 {
            format!(
                " {} ({}/{}) ",
                question.header,
                self.current_index + 1,
                self.questions.len()
            )
        } else {
            format!(" {} ", question.header)
        };

        // Draw border with title
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent_color))
            .style(Style::default().bg(theme.bg_color));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 || inner.width < 10 {
            return;
        }

        let mut y = inner.y;

        // Question text
        let question_line = Line::from(Span::styled(
            truncate(&question.question, inner.width as usize - 2),
            Style::default().fg(theme.text_color),
        ));
        render_line(buf, inner.x + 1, y, inner.width - 2, &question_line);
        y += 1;

        // Blank line
        y += 1;

        // Calculate visible range
        let total_options = question.options.len();
        let max_visible = Self::MAX_VISIBLE_OPTIONS.min(total_options);
        let has_scroll_up = self.scroll_offset > 0;
        let has_scroll_down = self.scroll_offset + max_visible < total_options;

        // Show scroll up indicator
        if has_scroll_up {
            let indicator = Line::from(Span::styled(
                format!("     ▲ {} more above", self.scroll_offset),
                Style::default().fg(theme.dim_color),
            ));
            render_line(buf, inner.x, y, inner.width, &indicator);
            y += 1;
        }

        // Options (with scroll)
        let visible_end = (self.scroll_offset + max_visible).min(total_options);
        for i in self.scroll_offset..visible_end {
            if y >= inner.y + inner.height - 1 {
                break;
            }

            let option = &question.options[i];
            let is_selected = i == self.selected_option;
            let is_toggled = self.toggled_options.contains(&i);

            // Build option line
            let prefix = if question.multi_select {
                if is_toggled {
                    "[x]"
                } else {
                    "[ ]"
                }
            } else if is_selected {
                ">"
            } else {
                " "
            };

            let number = format!("{}.", i + 1);

            let label_style = if is_selected {
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_color)
            };

            let mut spans = vec![
                Span::styled(
                    format!(" {} ", prefix),
                    Style::default().fg(if is_selected || is_toggled {
                        theme.accent_color
                    } else {
                        theme.dim_color
                    }),
                ),
                Span::styled(format!("{} ", number), Style::default().fg(theme.dim_color)),
                Span::styled(&option.label, label_style),
            ];

            // Add description if present
            if let Some(ref desc) = option.description {
                spans.push(Span::styled(
                    format!(" - {}", desc),
                    Style::default().fg(theme.dim_color),
                ));
            }

            let option_line = Line::from(spans);
            render_line(buf, inner.x, y, inner.width, &option_line);
            y += 1;
        }

        // Show scroll down indicator
        if has_scroll_down {
            let remaining = total_options - visible_end;
            let indicator = Line::from(Span::styled(
                format!("     ▼ {} more below", remaining),
                Style::default().fg(theme.dim_color),
            ));
            render_line(buf, inner.x, y, inner.width, &indicator);
        }

        // Footer hint (at bottom)
        if inner.y + inner.height > y {
            let hint = if self.custom_input_mode {
                if self.prompt_type == PromptType::PlanConfirm {
                    "typing modification... (Esc to cancel)"
                } else {
                    "typing custom response... (Esc to cancel)"
                }
            } else if self.prompt_type == PromptType::PlanConfirm {
                "press 1/2, click, or type to modify plan"
            } else {
                "type number, click, or enter custom response"
            };

            let hint_line = Line::from(Span::styled(
                hint,
                Style::default()
                    .fg(theme.dim_color)
                    .add_modifier(Modifier::ITALIC),
            ));

            let hint_y = inner.y + inner.height - 1;
            // Center the hint
            let hint_x = inner.x + (inner.width.saturating_sub(hint.len() as u16)) / 2;
            render_line(buf, hint_x, hint_y, inner.width, &hint_line);
        }
    }
}

/// Render a line to the buffer
fn render_line(buf: &mut Buffer, x: u16, y: u16, width: u16, line: &Line) {
    let area = Rect::new(x, y, width, 1);
    ratatui::widgets::Paragraph::new(line.clone()).render(area, buf);
}

/// Truncate string to fit width
fn truncate(s: &str, max_width: usize) -> String {
    if s.len() <= max_width {
        s.to_string()
    } else if max_width > 1 {
        format!("{}…", &s[..max_width - 1])
    } else {
        String::new()
    }
}
