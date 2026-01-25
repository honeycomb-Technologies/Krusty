//! Toolbar component - top bar with mode indicator and session title

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_width::UnicodeWidthStr;

use crate::tui::app::WorkMode;
use crate::tui::themes::Theme;

/// Shark spinner frames - swimming back and forth
const SHARK_FRAMES: &[&str] = &[
    "▐|\\____________▌",
    "▐_|\\___________▌",
    "▐__|\\__________▌",
    "▐___|\\_________▌",
    "▐____|\\________▌",
    "▐_____|\\_______▌",
    "▐______|\\______▌",
    "▐_______|\\_____▌",
    "▐________|\\____▌",
    "▐_________|\\___▌",
    "▐__________|\\__▌",
    "▐___________|\\_▌",
    "▐____________|\\▌",
    "▐____________/|▌",
    "▐___________/|_▌",
    "▐__________/|__▌",
    "▐_________/|___▌",
    "▐________/|____▌",
    "▐_______/|_____▌",
    "▐______/|______▌",
    "▐_____/|_______▌",
    "▐____/|________▌",
    "▐___/|_________▌",
    "▐__/|__________▌",
    "▐_/|___________▌",
    "▐/|____________▌",
];

/// Get the current spinner frame based on time
fn get_spinner_frame() -> &'static str {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let frame_idx = (millis / 120) as usize % SHARK_FRAMES.len(); // 120ms per frame
    SHARK_FRAMES[frame_idx]
}

/// Plan info for toolbar display
pub struct PlanInfo<'a> {
    #[allow(dead_code)]
    pub title: &'a str,
    pub completed: usize,
    pub total: usize,
}

/// Render the toolbar at the top of the screen
/// Returns the clickable area for the session title (if in chat mode)
pub fn render_toolbar(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    work_mode: WorkMode,
    session_title: Option<&str>,
    is_editing: bool,
    edit_buffer: &str,
    is_busy: bool,
    plan_info: Option<PlanInfo<'_>>,
) -> Option<Rect> {
    // Toolbar block with rounded borders
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_color))
        .style(Style::default().bg(theme.bg_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Split into left, center, right
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(50),
            Constraint::Percentage(25),
        ])
        .split(inner);

    // Get color and label based on work mode
    let (mode_color, mode_label) = match work_mode {
        WorkMode::Build => (theme.success_color, " BUILD "),
        WorkMode::Plan => (theme.mode_plan_color, " PLAN "),
    };

    // Left side: Shark spinner
    if is_busy {
        let frame = get_spinner_frame();
        let spinner_line = Line::from(vec![
            Span::raw(" "),
            Span::styled(frame, Style::default().fg(theme.accent_color)),
        ]);
        f.render_widget(Paragraph::new(spinner_line), chunks[0]);
    }

    // Center: Session title (clickable when in chat)
    let title_area = if session_title.is_some() || is_editing {
        let title_text = if is_editing {
            format!("{}|", edit_buffer) // Show cursor when editing
        } else {
            session_title.unwrap_or("New Chat").to_string()
        };

        let title_style = if is_editing {
            Style::default().fg(theme.text_color).bg(theme.border_color) // Highlight when editing
        } else {
            Style::default()
                .fg(theme.title_color)
                .add_modifier(Modifier::BOLD)
        };

        let title_widget = Paragraph::new(Span::styled(title_text.clone(), title_style))
            .alignment(Alignment::Center);
        f.render_widget(title_widget, chunks[1]);

        // Calculate clickable area (approximate center position)
        let title_len = title_text.width().min(chunks[1].width as usize) as u16;
        let title_start_x = chunks[1].x + (chunks[1].width.saturating_sub(title_len)) / 2;
        Some(Rect::new(title_start_x, chunks[1].y, title_len, 1))
    } else {
        None
    };

    // Right side: mode badge with optional plan progress
    // Plan title is shown in the sidebar header, not here
    let right_spans = if let Some(ref info) = plan_info {
        // Show: MODE 3/7 (progress only, title in sidebar)
        let progress = format!(" {} {}/{} ", mode_label.trim(), info.completed, info.total);
        vec![
            Span::styled(
                progress,
                Style::default()
                    .bg(mode_color)
                    .fg(theme.bg_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]
    } else {
        // No active plan - just show mode
        vec![
            Span::styled(
                mode_label,
                Style::default()
                    .bg(mode_color)
                    .fg(theme.bg_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]
    };

    let mode_badge = Line::from(right_spans);
    let mode_widget = Paragraph::new(mode_badge).alignment(Alignment::Right);
    f.render_widget(mode_widget, chunks[2]);

    title_area
}
