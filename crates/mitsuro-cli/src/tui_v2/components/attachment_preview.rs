//! Composer attachment preview (bracketed paths / clipboard images).
//!
//! Image payloads render via `ratatui-image` when the terminal supports a
//! graphics protocol; otherwise we fall back to metadata + text body.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use ratatui_image::{protocol::StatefulProtocol, Resize, StatefulImage};

use crate::tui_v2::{app::state::AttachmentPreview, presentation::theme::SemanticTheme};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    preview: &AttachmentPreview,
    theme: SemanticTheme,
    image_protocol: Option<&mut StatefulProtocol>,
) {
    if area.is_empty() {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    let header = vec![
        Line::from(vec![Span::styled(
            preview.title.as_str(),
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::styled(
            format!("{}  ·  {}", preview.kind_label, preview.detail),
            Style::default().fg(theme.foreground_muted),
        ),
    ];
    frame.render_widget(
        Paragraph::new(header).style(Style::default().bg(theme.surface).fg(theme.foreground)),
        chunks[0],
    );

    let content = chunks[1];
    if let Some(protocol) = image_protocol {
        let image_widget = StatefulImage::default().resize(Resize::Fit(None));
        frame.render_stateful_widget(image_widget, content, protocol);
    } else {
        let mut lines = Vec::new();
        for line in preview
            .body
            .lines()
            .take(usize::from(content.height.saturating_sub(1)))
        {
            lines.push(Line::styled(
                line.to_owned(),
                Style::default().fg(theme.foreground),
            ));
        }
        if lines.is_empty() {
            lines.push(Line::styled(
                if preview.image_path.is_some() {
                    "Image attached — graphics protocol unavailable in this terminal."
                } else {
                    "(no preview content)"
                },
                Style::default().fg(theme.foreground_muted),
            ));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Left)
                .style(Style::default().bg(theme.surface).fg(theme.foreground)),
            content,
        );
    }

    frame.render_widget(
        Paragraph::new(Line::styled(
            "Esc close  ·  click chip again to refresh",
            Style::default().fg(theme.foreground_muted),
        )),
        chunks[2],
    );
}
