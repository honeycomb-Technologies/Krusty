//! Pinch popup
//!
//! Two-stage popup for pinch (context continuation):
//! 1. User provides preservation hints before summarization
//! 2. User reviews summary and provides direction for next phase

use std::time::Instant;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use super::common::{
    center_rect, popup_block, popup_title, render_popup_background, scroll_indicator, PopupSize,
};
use crate::agent::SummarizationResult;
use crate::tui::animation::menu::CrabAnimator;
use crate::tui::themes::Theme;

/// Pinch popup stages
#[derive(Debug, Clone)]
pub enum PinchStage {
    /// Stage 1: User specifies what to preserve
    PreservationInput {
        input: String,
        context_usage_percent: u8,
        top_files: Vec<(String, f64)>,
    },

    /// Summarization in progress
    Summarizing { dots: usize },

    /// Stage 2: User reviews summary and provides direction
    DirectionInput {
        summary: String,
        key_files: Vec<String>,
        input: String,
        scroll_offset: usize,
    },

    /// Creating new session
    Creating,

    /// Complete - show link to new session
    Complete {
        new_session_id: String,
        new_session_title: String,
        /// Whether to auto-start AI response (when direction was provided)
        auto_continue: bool,
    },

    /// Error state
    Error { message: String },
}

impl Default for PinchStage {
    fn default() -> Self {
        Self::PreservationInput {
            input: String::new(),
            context_usage_percent: 0,
            top_files: Vec::new(),
        }
    }
}

/// Pinch popup
pub struct PinchPopup {
    pub stage: PinchStage,
    /// Cached preservation input for stage 2
    pub preservation_input: Option<String>,
    /// Full summarization result (stored after AI summarization completes)
    pub summarization_result: Option<SummarizationResult>,
    /// Crab animator for summarizing stage
    crab: CrabAnimator,
    /// Last animation update time
    last_update: Instant,
}

impl Default for PinchPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl PinchPopup {
    pub fn new() -> Self {
        Self {
            stage: PinchStage::default(),
            preservation_input: None,
            summarization_result: None,
            crab: CrabAnimator::new(10.0, 0.0),
            last_update: Instant::now(),
        }
    }

    pub fn reset(&mut self) {
        self.stage = PinchStage::default();
        self.preservation_input = None;
        self.summarization_result = None;
        self.crab = CrabAnimator::new(10.0, 0.0);
    }

    /// Store the full summarization result for use when completing pinch
    pub fn set_summarization_result(&mut self, result: SummarizationResult) {
        self.summarization_result = Some(result);
    }

    /// Get the stored summarization result
    pub fn get_summarization_result(&self) -> Option<&SummarizationResult> {
        self.summarization_result.as_ref()
    }

    /// Start the pinch flow
    pub fn start(&mut self, context_usage_percent: u8, top_files: Vec<(String, f64)>) {
        self.stage = PinchStage::PreservationInput {
            input: String::new(),
            context_usage_percent,
            top_files,
        };
        self.preservation_input = None;
    }

    /// Move to summarizing stage
    pub fn start_summarizing(&mut self) {
        // Cache the preservation input
        if let PinchStage::PreservationInput { input, .. } = &self.stage {
            if !input.is_empty() {
                self.preservation_input = Some(input.clone());
            }
        }
        self.stage = PinchStage::Summarizing { dots: 0 };
    }

    /// Show summary and move to direction input
    pub fn show_summary(&mut self, summary: String, key_files: Vec<String>) {
        self.stage = PinchStage::DirectionInput {
            summary,
            key_files,
            input: String::new(),
            scroll_offset: 0,
        };
    }

    /// Start creating session
    pub fn start_creating(&mut self) {
        self.stage = PinchStage::Creating;
    }

    /// Complete with new session info
    pub fn complete(
        &mut self,
        new_session_id: String,
        new_session_title: String,
        auto_continue: bool,
    ) {
        self.stage = PinchStage::Complete {
            new_session_id,
            new_session_title,
            auto_continue,
        };
    }

    /// Set error state
    pub fn set_error(&mut self, message: String) {
        self.stage = PinchStage::Error { message };
    }

    /// Get preservation hints (stage 1 input)
    pub fn get_preservation_input(&self) -> Option<&str> {
        self.preservation_input.as_deref()
    }

    /// Get direction input (stage 2 input)
    pub fn get_direction_input(&self) -> Option<&str> {
        if let PinchStage::DirectionInput { input, .. } = &self.stage {
            Some(input.as_str())
        } else {
            None
        }
    }

    /// Add character to current input
    pub fn add_char(&mut self, c: char) {
        match &mut self.stage {
            PinchStage::PreservationInput { input, .. } => input.push(c),
            PinchStage::DirectionInput { input, .. } => input.push(c),
            _ => {}
        }
    }

    /// Backspace in current input
    pub fn backspace(&mut self) {
        match &mut self.stage {
            PinchStage::PreservationInput { input, .. } => {
                input.pop();
            }
            PinchStage::DirectionInput { input, .. } => {
                input.pop();
            }
            _ => {}
        }
    }

    /// Scroll up in summary view
    pub fn scroll_up(&mut self) {
        if let PinchStage::DirectionInput { scroll_offset, .. } = &mut self.stage {
            *scroll_offset = scroll_offset.saturating_sub(1);
        }
    }

    /// Scroll down in summary view
    pub fn scroll_down(&mut self) {
        if let PinchStage::DirectionInput { scroll_offset, .. } = &mut self.stage {
            *scroll_offset += 1;
        }
    }

    /// Tick the animation (for summarizing crab)
    pub fn tick(&mut self) {
        if matches!(self.stage, PinchStage::Summarizing { .. }) {
            let dt = self.last_update.elapsed();
            self.last_update = Instant::now();
            // Use a reasonable width for the popup content area
            self.crab.update(dt, 50);
        }
    }

    pub fn render(&self, f: &mut Frame, theme: &Theme) {
        match &self.stage {
            PinchStage::PreservationInput {
                input,
                context_usage_percent,
                top_files,
            } => self.render_preservation_input(f, theme, input, *context_usage_percent, top_files),
            PinchStage::Summarizing { dots } => self.render_summarizing(f, theme, *dots),
            PinchStage::DirectionInput {
                summary,
                key_files,
                input,
                scroll_offset,
            } => self.render_direction_input(f, theme, summary, key_files, input, *scroll_offset),
            PinchStage::Creating => self.render_creating(f, theme),
            PinchStage::Complete {
                new_session_id: _,
                new_session_title,
                auto_continue: _,
            } => self.render_complete(f, theme, new_session_title),
            PinchStage::Error { message } => self.render_error(f, theme, message),
        }
    }

    fn render_preservation_input(
        &self,
        f: &mut Frame,
        theme: &Theme,
        input: &str,
        context_percent: u8,
        top_files: &[(String, f64)],
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
                Constraint::Length(3), // Context usage bar
                Constraint::Length(2), // Spacer
                Constraint::Min(8),    // Top files
                Constraint::Length(2), // Prompt
                Constraint::Length(3), // Input
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_lines = popup_title("Pinch", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Context usage bar
        let bar_width: usize = 30;
        let filled = ((bar_width as f64 * context_percent as f64 / 100.0) as usize).min(bar_width);
        let empty = bar_width - filled;
        let bar_color = if context_percent >= 90 {
            theme.error_color
        } else if context_percent >= 80 {
            theme.warning_color
        } else {
            theme.success_color
        };

        let bar = format!(
            "Context Usage: [{}{}] {}%",
            "█".repeat(filled),
            "░".repeat(empty),
            context_percent
        );
        let usage = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(bar, Style::default().fg(bar_color))),
        ])
        .alignment(Alignment::Center);
        f.render_widget(usage, chunks[1]);

        // Top files section
        let mut file_lines = vec![
            Line::from(Span::styled(
                "Top Files by Activity:",
                Style::default()
                    .fg(theme.text_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

        for (i, (path, score)) in top_files.iter().take(5).enumerate() {
            // Truncate long paths (char-boundary safe)
            let display_path = if path.len() > 45 {
                // Find valid char boundary for truncation
                let target = path.len().saturating_sub(42);
                let start = path
                    .char_indices()
                    .find(|(i, _)| *i >= target)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                format!("...{}", &path[start..])
            } else {
                path.clone()
            };
            file_lines.push(Line::from(vec![
                Span::styled(
                    format!("  {}. ", i + 1),
                    Style::default().fg(theme.dim_color),
                ),
                Span::styled(display_path, Style::default().fg(theme.accent_color)),
                Span::styled(
                    format!(" ({:.1})", score),
                    Style::default().fg(theme.dim_color),
                ),
            ]));
        }

        if top_files.is_empty() {
            file_lines.push(Line::from(Span::styled(
                "  No file activity tracked yet",
                Style::default().fg(theme.dim_color),
            )));
        }

        let files = Paragraph::new(file_lines);
        f.render_widget(files, chunks[3]);

        // Prompt
        let prompt = Paragraph::new(Line::from(Span::styled(
            "What should I focus on preserving?",
            Style::default().fg(theme.text_color),
        )));
        f.render_widget(prompt, chunks[4]);

        // Input field
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_color));

        let display = if input.is_empty() {
            "(optional - press Enter to skip)"
        } else {
            input
        };
        let style = if input.is_empty() {
            Style::default().fg(theme.dim_color)
        } else {
            Style::default().fg(theme.text_color)
        };

        let input_widget = Paragraph::new(display).style(style).block(input_block);
        f.render_widget(input_widget, chunks[5]);

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
            Span::styled(": cancel", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[6]);
    }

    fn render_summarizing(&self, f: &mut Frame, theme: &Theme, _dots: usize) {
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
                Constraint::Length(1), // Spacer
                Constraint::Length(5), // Crab (5 lines)
                Constraint::Length(2), // Status text
                Constraint::Min(1),    // Spacer
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_lines = popup_title("Pinch", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Animated crab - render in chunks[2] (the 5-line crab area)
        // Crab lines have varying leading spaces for shape alignment.
        // We must pad ALL lines to the same width (25 chars = widest line)
        // so that centering the paragraph doesn't destroy the shape.
        let crab_lines = self.crab.render();
        let max_width = 25; // Widest crab line
        let crab_content: Vec<Line> = crab_lines
            .iter()
            .map(|line| {
                // Pad to fixed width so centering preserves relative alignment
                let padded = format!("{:width$}", line, width = max_width);
                Line::from(Span::styled(
                    padded,
                    Style::default().fg(theme.accent_color),
                ))
            })
            .collect();
        let crab_widget = Paragraph::new(crab_content).alignment(Alignment::Center);
        f.render_widget(crab_widget, chunks[2]);

        // Status text - render in chunks[3]
        let status = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Summarizing conversation...",
                Style::default().fg(theme.text_color),
            )),
        ];
        let status_widget = Paragraph::new(status).alignment(Alignment::Center);
        f.render_widget(status_widget, chunks[3]);

        // Footer - render in chunks[5]
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
        f.render_widget(footer, chunks[5]);
    }

    fn render_direction_input(
        &self,
        f: &mut Frame,
        theme: &Theme,
        summary: &str,
        key_files: &[String],
        input: &str,
        scroll_offset: usize,
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
                Constraint::Min(10),   // Summary (scrollable)
                Constraint::Length(5), // Key files
                Constraint::Length(2), // Prompt
                Constraint::Length(3), // Input
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title
        let title_lines = popup_title("Summary Preview", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Summary (scrollable) with scroll indicators
        let total_lines = summary.lines().count();
        // Account for block borders (2 lines) and potential scroll indicators
        let visible_height = (chunks[1].height as usize).saturating_sub(4);

        let mut summary_lines: Vec<Line> = Vec::new();

        // Scroll indicator (up)
        if scroll_offset > 0 {
            summary_lines.push(scroll_indicator("up", scroll_offset, theme));
        }

        // Summary content
        for line in summary.lines().skip(scroll_offset).take(visible_height) {
            summary_lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(theme.text_color),
            )));
        }

        // Scroll indicator (down)
        let remaining = total_lines.saturating_sub(scroll_offset + visible_height);
        if remaining > 0 {
            summary_lines.push(scroll_indicator("down", remaining, theme));
        }

        let summary_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_color))
            .title(Span::styled(
                " Summary ",
                Style::default().fg(theme.dim_color),
            ));

        let summary_widget = Paragraph::new(summary_lines)
            .block(summary_block)
            .wrap(Wrap { trim: true });
        f.render_widget(summary_widget, chunks[1]);

        // Key files
        let mut file_lines = vec![Line::from(Span::styled(
            "Key Files:",
            Style::default()
                .fg(theme.text_color)
                .add_modifier(Modifier::BOLD),
        ))];

        for file in key_files.iter().take(3) {
            // Truncate long paths (char-boundary safe)
            let display = if file.len() > 50 {
                let target = file.len().saturating_sub(47);
                let start = file
                    .char_indices()
                    .find(|(i, _)| *i >= target)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                format!("...{}", &file[start..])
            } else {
                file.clone()
            };
            file_lines.push(Line::from(vec![
                Span::styled("  • ", Style::default().fg(theme.dim_color)),
                Span::styled(display, Style::default().fg(theme.accent_color)),
            ]));
        }

        if key_files.len() > 3 {
            file_lines.push(Line::from(Span::styled(
                format!("  ... and {} more", key_files.len() - 3),
                Style::default().fg(theme.dim_color),
            )));
        }

        let files = Paragraph::new(file_lines);
        f.render_widget(files, chunks[2]);

        // Prompt
        let prompt = Paragraph::new(Line::from(Span::styled(
            "What direction for next phase?",
            Style::default().fg(theme.text_color),
        )));
        f.render_widget(prompt, chunks[3]);

        // Input field
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_color));

        let display = if input.is_empty() {
            "(optional - press Enter to continue with summary only)"
        } else {
            input
        };
        let style = if input.is_empty() {
            Style::default().fg(theme.dim_color)
        } else {
            Style::default().fg(theme.text_color)
        };

        let input_widget = Paragraph::new(display).style(style).block(input_block);
        f.render_widget(input_widget, chunks[4]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "↑↓",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": scroll  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": create session  ", Style::default().fg(theme.text_color)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(": cancel", Style::default().fg(theme.text_color)),
        ]))
        .alignment(Alignment::Center);
        f.render_widget(footer, chunks[5]);
    }

    fn render_creating(&self, f: &mut Frame, theme: &Theme) {
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
            ])
            .split(inner);

        // Title
        let title_lines = popup_title("Pinch", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        let content = vec![
            Line::from(""),
            Line::from(Span::styled("⏳", Style::default().fg(theme.accent_color))),
            Line::from(""),
            Line::from(Span::styled(
                "Creating new session...",
                Style::default().fg(theme.text_color),
            )),
        ];
        let waiting = Paragraph::new(content).alignment(Alignment::Center);
        f.render_widget(waiting, chunks[1]);
    }

    fn render_complete(&self, f: &mut Frame, theme: &Theme, new_session_title: &str) {
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
        let title_lines = popup_title("Pinch", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        let content = vec![
            Line::from(""),
            Line::from(Span::styled(
                "✓ Session created!",
                Style::default()
                    .fg(theme.success_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                new_session_title.to_string(),
                Style::default().fg(theme.accent_color),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Your context has been preserved.",
                Style::default().fg(theme.text_color),
            )),
        ];
        let success = Paragraph::new(content).alignment(Alignment::Center);
        f.render_widget(success, chunks[1]);

        // Footer
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                ": switch to session  ",
                Style::default().fg(theme.text_color),
            ),
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

    fn render_error(&self, f: &mut Frame, theme: &Theme, message: &str) {
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
        let title_lines = popup_title("Pinch", theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        let content = vec![
            Line::from(""),
            Line::from(Span::styled(
                "✗ Pinch failed",
                Style::default()
                    .fg(theme.error_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                message.to_string(),
                Style::default().fg(theme.text_color),
            )),
        ];
        let error = Paragraph::new(content)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        f.render_widget(error, chunks[1]);

        // Footer
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
