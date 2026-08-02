//! Quiet, width-aware conversation identity and metadata.
//!
//! Top row, three bands:
//! - Left: git diff (+N −M) · agent context (used/max)
//! - Center: intentionally empty — global “working” is the bottom purple edge
//! - Right: session title · project  (original placement)
//!
//! Title is clickable: click → edit → Enter saves, Esc cancels.

use ratatui::{
    layout::{Alignment, Position},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::tui_v2::{
    app::state::UiState,
    layout::snapshot::{LayoutRegionId, LayoutSnapshot},
    model::conversation::ConversationMetadata,
    presentation::theme::SemanticTheme,
    services::HomeSnapshot,
};

pub fn render_context_bar(
    frame: &mut Frame,
    layout: &LayoutSnapshot,
    state: &UiState,
    metadata: &ConversationMetadata,
    context: Option<&HomeSnapshot>,
    theme: SemanticTheme,
) {
    let Some(identity) = layout.region(LayoutRegionId::ContextIdentity) else {
        return;
    };
    let Some(meta) = layout.region(LayoutRegionId::ContextMeta) else {
        return;
    };
    // Center band is optional on very narrow widths.
    let status = layout.region(LayoutRegionId::ContextStatus);

    // ── Left: +N −M · context ─────────────────────────────────────────
    let mut left: Vec<Span> = Vec::with_capacity(10);
    left.push(Span::raw(" "));
    let chrome = &state.workspace;
    if chrome.has_git_diff() {
        left.push(Span::styled(
            format!("+{}", chrome.git_additions),
            Style::default().fg(theme.diff_add),
        ));
        left.push(Span::raw(" "));
        left.push(Span::styled(
            format!("−{}", chrome.git_deletions),
            Style::default().fg(theme.diff_remove),
        ));
    }
    if chrome.has_context() {
        if left.len() > 1 {
            push_sep(&mut left, theme);
        }
        let used = compact_k(chrome.context_used);
        let max = compact_k(chrome.context_max);
        let pct = ((chrome.context_used as f64 / chrome.context_max as f64) * 100.0) as u8;
        let ctx_color = if pct > 80 {
            theme.error
        } else if pct > 60 {
            theme.warning
        } else {
            theme.foreground_muted
        };
        left.push(Span::styled(
            format!("{used}/{max}"),
            Style::default().fg(ctx_color),
        ));
    }
    let left_w = usize::from(identity.width.saturating_sub(1)).max(1);
    frame.render_widget(
        Paragraph::new(Line::from(fit_spans(left, left_w)))
            .style(Style::default().fg(theme.foreground_muted)),
        identity,
    );

    // ── Center: empty (working lives on the bottom purple edge rail) ──
    if let Some(status) = status {
        if status.width > 0 {
            // Clear the band so prior “working” paint never sticks.
            frame.render_widget(
                Paragraph::new("").style(Style::default().fg(theme.foreground_muted)),
                status,
            );
        }
    }

    // ── Right: title · project ────────────────────────────────────────
    if state.title_edit.active {
        let buffer = &state.title_edit.buffer;
        let room = usize::from(meta.width.saturating_sub(2)).max(1);
        let shown = truncate_to_width(buffer, room.saturating_sub(1), "…");
        let cursor_col = UnicodeWidthStr::width(shown.as_str()).min(room.saturating_sub(1));
        let pad = room.saturating_sub(UnicodeWidthStr::width(shown.as_str()).saturating_add(1));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(" ".repeat(pad)),
                Span::styled(
                    shown,
                    Style::default()
                        .fg(theme.foreground)
                        .bg(theme.selection_surface)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "│",
                    Style::default()
                        .fg(theme.identity)
                        .bg(theme.selection_surface),
                ),
                Span::raw(" "),
            ])),
            meta,
        );
        frame.set_cursor_position(Position::new(
            meta.x
                .saturating_add(pad as u16)
                .saturating_add(cursor_col as u16),
            meta.y,
        ));
    } else {
        let title = metadata.title.as_deref().unwrap_or("New conversation");
        let project = context
            .map(|ctx| ctx.project.as_str())
            .filter(|p| !p.is_empty());
        let mut right = Vec::new();
        right.push(Span::styled(
            truncate_to_width(title, usize::from(meta.width.saturating_sub(4)).max(8), "…"),
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        ));
        if let Some(project) = project.filter(|_| meta.width >= 28) {
            let used = right
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>();
            let room = usize::from(meta.width).saturating_sub(used.saturating_add(6));
            if room >= 4 {
                right.push(Span::styled(
                    "  ·  ",
                    Style::default().fg(theme.foreground_muted),
                ));
                right.push(Span::styled(
                    truncate_to_width(project, room, "…"),
                    Style::default().fg(theme.foreground_muted),
                ));
            }
        }
        right.push(Span::raw(" "));
        frame.render_widget(
            Paragraph::new(Line::from(right))
                .alignment(Alignment::Right)
                .style(Style::default().fg(theme.foreground)),
            meta,
        );
    }
}

fn push_sep(spans: &mut Vec<Span>, theme: SemanticTheme) {
    spans.push(Span::styled(
        "  ·  ",
        Style::default().fg(theme.foreground_muted),
    ));
}

fn compact_k(tokens: usize) -> String {
    if tokens >= 1_000 {
        format!("{:.0}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn fit_spans(spans: Vec<Span>, max_width: usize) -> Vec<Span> {
    let mut used = 0usize;
    let mut out = Vec::new();
    for span in spans {
        let w = UnicodeWidthStr::width(span.content.as_ref());
        if used.saturating_add(w) > max_width {
            let room = max_width.saturating_sub(used);
            if room > 1 {
                let clipped = truncate_to_width(span.content.as_ref(), room, "…");
                out.push(Span::styled(clipped, span.style));
            }
            break;
        }
        used = used.saturating_add(w);
        out.push(span);
    }
    out
}

fn truncate_to_width(text: &str, max_width: usize, ellipsis: &str) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_owned();
    }
    let ellipsis_w = UnicodeWidthStr::width(ellipsis);
    if ellipsis_w >= max_width {
        return ellipsis.chars().take(1).collect();
    }
    let budget = max_width.saturating_sub(ellipsis_w);
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used.saturating_add(w) > budget {
            break;
        }
        out.push(ch);
        used = used.saturating_add(w);
    }
    out.push_str(ellipsis);
    out
}
