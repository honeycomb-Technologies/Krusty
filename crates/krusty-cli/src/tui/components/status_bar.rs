//! Status bar component - bottom bar with model, cwd, shortcuts

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::path::Path;
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

use crate::tui::themes::Theme;

/// Render the status bar at the bottom of the screen
pub fn render_status_bar(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    model: &str,
    cwd: &Path,
    context_tokens: Option<(usize, usize)>, // (used, max)
    running_processes: usize,
    process_elapsed: Option<Duration>,
) {
    // Background
    let bg = Paragraph::new("").style(Style::default().bg(theme.status_bar_bg_color));
    f.render_widget(bg, area);

    // Build left section content and calculate its width
    let model_short = shorten_model_name(model);
    let cwd_display = shorten_path(cwd, 30);

    let mut left_spans = vec![
        Span::raw(" "),
        Span::styled(&cwd_display, Style::default().fg(theme.dim_color)),
        Span::styled(" │ ", Style::default().fg(theme.dim_color)),
        Span::styled(&model_short, Style::default().fg(theme.dim_color)),
    ];

    // Calculate left width: space + cwd + " │ " + model
    let mut left_width: u16 = 1 + cwd_display.width() as u16 + 3 + model_short.width() as u16;

    // Add context indicator if available (fixed width to prevent flashing)
    if let Some((used, max)) = context_tokens {
        let used_k = used as f64 / 1000.0;
        let max_k = max as f64 / 1000.0;
        let percentage = (used as f64 / max as f64 * 100.0) as u8;

        let ctx_color = if percentage > 80 {
            theme.error_color
        } else if percentage > 60 {
            theme.warning_color
        } else {
            theme.dim_color
        };

        // Fixed width: "999.9k/9999k" = 12 chars max
        let ctx_text = format!("{:>5.1}k/{:.0}k", used_k, max_k);
        left_width += 3 + 12; // " │ " + fixed 12 chars

        left_spans.push(Span::styled(" │ ", Style::default().fg(theme.dim_color)));
        left_spans.push(Span::styled(
            format!("{:>12}", ctx_text), // Pad to fixed width
            Style::default().fg(ctx_color),
        ));
    }

    // Running processes indicator with elapsed time
    if running_processes > 0 {
        let elapsed_str = process_elapsed
            .map(|elapsed| {
                let secs = elapsed.as_secs();
                match secs {
                    s if s >= 3600 => format!(" {}h{}m", s / 3600, (s % 3600) / 60),
                    s if s >= 60 => format!(" {}m{}s", s / 60, s % 60),
                    s => format!(" {}s", s),
                }
            })
            .unwrap_or_default();
        let proc_text = format!("● {}{}", running_processes, elapsed_str);
        left_width += 3 + proc_text.width() as u16; // " │ " + text

        left_spans.push(Span::styled(" │ ", Style::default().fg(theme.dim_color)));
        left_spans.push(Span::styled(
            proc_text,
            Style::default().fg(theme.processing_color),
        ));
    }

    // Split into left (fixed) and right (fill)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_width), Constraint::Fill(1)])
        .split(area);

    // Render left section
    let left_content = Line::from(left_spans);
    f.render_widget(Paragraph::new(left_content), chunks[0]);

    // Render commands based on available width (right-aligned)
    let available_width = chunks[1].width as usize;
    let commands = build_commands_for_width(available_width, theme);
    f.render_widget(
        Paragraph::new(Line::from(commands)).alignment(Alignment::Right),
        chunks[1],
    );
}

/// Build command spans based on available width
/// Priority (highest to lowest): quit, stop, procs, newline, thinking
fn build_commands_for_width<'a>(width: usize, theme: &'a Theme) -> Vec<Span<'a>> {
    // Command definitions with their widths
    // Format: (key_text, desc_text, total_width including spaces)
    let commands: [(&str, &str, usize); 5] = [
        (" ^Q ", "quit ", 10), // highest priority
        (" Esc ", "stop ", 11),
        (" ^P ", "procs ", 11),
        (" Shift+↵ ", "newline ", 18),
        (" Tab ", "thinking ", 15), // lowest priority
    ];

    let mut spans = Vec::new();
    let mut used_width = 0;

    // Add commands from highest to lowest priority
    for (key, desc, cmd_width) in commands {
        if used_width + cmd_width <= width {
            // Insert at beginning (so lower priority items end up on left)
            let insert_pos = spans.len();
            spans.insert(
                insert_pos,
                Span::styled(
                    key,
                    Style::default().bg(theme.border_color).fg(theme.text_color),
                ),
            );
            spans.insert(
                insert_pos + 1,
                Span::styled(desc, Style::default().fg(theme.dim_color)),
            );
            used_width += cmd_width;
        }
    }

    spans
}

/// Shorten model name for display
fn shorten_model_name(model: &str) -> String {
    [
        ("opus", "opus 4.5"),
        ("sonnet", "sonnet 4.5"),
        ("haiku", "haiku 4.5"),
    ]
    .iter()
    .find(|(key, _)| model.contains(key))
    .map_or_else(
        || model.chars().take(15).collect(),
        |(_, name)| name.to_string(),
    )
}

/// Shorten path for display
fn shorten_path(path: &Path, max_len: usize) -> String {
    let path_str = path.to_string_lossy();

    // Try to use ~ for home directory
    let display = if let Ok(home) = std::env::var("HOME") {
        if path_str.starts_with(&home) {
            format!("~{}", &path_str[home.len()..])
        } else {
            path_str.to_string()
        }
    } else {
        path_str.to_string()
    };

    // Truncate if too long (width-based for proper display)
    if display.width() > max_len {
        // Find a byte position where remaining width fits in max_len - 3
        let target_width = max_len.saturating_sub(3);
        let mut current_width = 0;
        let mut start_byte = 0;
        for (i, c) in display.char_indices() {
            let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if display.width() - current_width <= target_width {
                start_byte = i;
                break;
            }
            current_width += char_width;
        }
        format!("...{}", &display[start_byte..])
    } else {
        display
    }
}
