//! Selection highlighting utilities for message rendering
//!
//! Functions for applying text selection highlighting to message content.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use regex::Regex;
use std::sync::LazyLock;

/// Pattern for file references in user messages: [Image: filename] or [PDF: filename]
pub(super) static FILE_REF_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[(Image|PDF): ([^\]]+)\]").unwrap());

pub(super) struct UserLineFileRefOptions<'a> {
    pub line_idx: usize,
    pub selection: Option<((usize, usize), (usize, usize))>,
    pub base_style: Style,
    pub link_color: Color,
    pub sel_bg: Color,
    pub sel_fg: Color,
    pub msg_idx: usize,
    pub hovered_file_ref: Option<&'a (usize, String)>,
}

/// Apply selection highlighting to a pre-rendered markdown line (preserves existing styles)
/// Uses CHARACTER indexing (not byte indexing) to handle UTF-8 safely
#[inline]
pub(super) fn apply_selection_to_rendered_line(
    line: Line<'static>,
    line_idx: usize,
    selection: Option<((usize, usize), (usize, usize))>,
    sel_bg: Color,
    sel_fg: Color,
) -> Line<'static> {
    // Early return if no selection (most common case)
    let Some(((start_line, start_col), (end_line, end_col))) = selection else {
        return line;
    };

    // Early return if line outside selection range
    if line_idx < start_line || line_idx > end_line {
        return line;
    }

    // Calculate total CHARACTER count (not byte count!)
    let total_chars: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
    if total_chars == 0 {
        return line;
    }

    // Determine selection bounds for this line (in characters)
    let (sel_start, sel_end) = if line_idx == start_line && line_idx == end_line {
        (start_col.min(total_chars), end_col.min(total_chars))
    } else if line_idx == start_line {
        (start_col.min(total_chars), total_chars)
    } else if line_idx == end_line {
        (0, end_col.min(total_chars))
    } else {
        (0, total_chars)
    };

    if sel_start >= sel_end {
        return line;
    }

    // For full-line selection, just restyle all spans (faster path)
    if sel_start == 0 && sel_end >= total_chars {
        let mut spans = Vec::with_capacity(line.spans.len());
        for span in line.spans {
            spans.push(Span::styled(span.content, span.style.bg(sel_bg).fg(sel_fg)));
        }
        return Line::from(spans);
    }

    // Partial selection - split spans at CHARACTER boundaries (UTF-8 safe)
    let mut new_spans = Vec::with_capacity(line.spans.len() + 2);
    let mut char_pos = 0;

    for span in line.spans {
        let span_char_len = span.content.chars().count();
        let span_char_end = char_pos + span_char_len;

        if span_char_end <= sel_start || char_pos >= sel_end {
            // Span is entirely outside selection
            new_spans.push(span);
        } else if char_pos >= sel_start && span_char_end <= sel_end {
            // Span is entirely inside selection
            new_spans.push(Span::styled(span.content, span.style.bg(sel_bg).fg(sel_fg)));
        } else {
            // Span partially overlaps - split at character boundaries
            let local_start = sel_start.saturating_sub(char_pos);
            let local_end = (sel_end - char_pos).min(span_char_len);
            let chars: Vec<char> = span.content.chars().collect();

            if local_start > 0 {
                let before: String = chars[..local_start].iter().collect();
                new_spans.push(Span::styled(before, span.style));
            }
            let selected: String = chars[local_start..local_end].iter().collect();
            new_spans.push(Span::styled(selected, span.style.bg(sel_bg).fg(sel_fg)));
            if local_end < span_char_len {
                let after: String = chars[local_end..].iter().collect();
                new_spans.push(Span::styled(after, span.style));
            }
        }

        char_pos = span_char_end;
    }

    Line::from(new_spans)
}

/// Apply selection highlighting to a line (takes ownership of text)
pub(super) fn apply_selection_to_line(
    text: String,
    line_idx: usize,
    selection: Option<((usize, usize), (usize, usize))>,
    base_style: Style,
    sel_bg: Color,
    sel_fg: Color,
) -> Line<'static> {
    let Some(((start_line, start_col), (end_line, end_col))) = selection else {
        return Line::from(Span::styled(text, base_style));
    };

    // Check if this line is within selection
    if line_idx < start_line || line_idx > end_line {
        return Line::from(Span::styled(text, base_style));
    }

    let text_len = text.chars().count();
    let sel_style = Style::default()
        .bg(sel_bg)
        .fg(sel_fg)
        .add_modifier(Modifier::BOLD);

    // Determine selection bounds for this line
    let (sel_start, sel_end) = if line_idx == start_line && line_idx == end_line {
        // Selection starts and ends on this line
        (start_col.min(text_len), end_col.min(text_len))
    } else if line_idx == start_line {
        // Selection starts on this line, continues past
        (start_col.min(text_len), text_len)
    } else if line_idx == end_line {
        // Selection ends on this line
        (0, end_col.min(text_len))
    } else {
        // Entire line is selected
        (0, text_len)
    };

    if sel_start >= sel_end {
        return Line::from(Span::styled(text, base_style));
    }

    // Split text into three parts: before, selected, after
    let chars: Vec<char> = text.chars().collect();
    let before: String = chars[..sel_start].iter().collect();
    let selected: String = chars[sel_start..sel_end].iter().collect();
    let after: String = chars[sel_end..].iter().collect();

    let mut spans = Vec::with_capacity(3);
    if !before.is_empty() {
        spans.push(Span::styled(before, base_style));
    }
    if !selected.is_empty() {
        spans.push(Span::styled(selected, sel_style));
    }
    if !after.is_empty() {
        spans.push(Span::styled(after, base_style));
    }

    Line::from(spans)
}

/// Style a user message line with file references highlighted
/// File references like [filename.png] get link styling with hover effects
pub(super) fn style_user_line_with_file_refs(
    text: &str,
    options: UserLineFileRefOptions<'_>,
) -> Line<'static> {
    let UserLineFileRefOptions {
        line_idx,
        selection,
        base_style,
        link_color,
        sel_bg,
        sel_fg,
        msg_idx,
        hovered_file_ref,
    } = options;

    let link_style = Style::default()
        .fg(link_color)
        .add_modifier(Modifier::UNDERLINED);
    // Hover style: inverted colors for clear visibility
    let hover_style = Style::default()
        .fg(Color::Black)
        .bg(link_color)
        .add_modifier(Modifier::BOLD);
    let sel_style = Style::default()
        .bg(sel_bg)
        .fg(sel_fg)
        .add_modifier(Modifier::BOLD);

    // Find file reference matches with capture groups
    let captures: Vec<_> = FILE_REF_PATTERN.captures_iter(text).collect();

    if captures.is_empty() {
        // No file refs - use standard rendering
        return apply_selection_to_line(
            text.to_string(),
            line_idx,
            selection,
            base_style,
            sel_bg,
            sel_fg,
        );
    }

    // Build spans with file refs styled
    let mut spans = Vec::with_capacity(captures.len() * 2 + 1);
    let mut last_end = 0;

    for caps in captures {
        let Some(full_match) = caps.get(0) else {
            continue;
        };

        // Add text before this match
        if full_match.start() > last_end {
            let before = &text[last_end..full_match.start()];
            spans.push(Span::styled(before.to_string(), base_style));
        }

        // Get the full file ref text (e.g., "[Image: screenshot.png]")
        let file_ref = full_match.as_str();
        // Extract display name from capture group 2 (the filename part)
        let display_name = caps.get(2).map(|m| m.as_str()).unwrap_or("");

        // Check if this file ref is hovered
        let is_hovered = hovered_file_ref
            .map(|(idx, name)| *idx == msg_idx && name == display_name)
            .unwrap_or(false);

        let style = if is_hovered { hover_style } else { link_style };
        spans.push(Span::styled(file_ref.to_string(), style));

        last_end = full_match.end();
    }

    // Add remaining text after last match
    if last_end < text.len() {
        let after = &text[last_end..];
        spans.push(Span::styled(after.to_string(), base_style));
    }

    // Apply selection if active
    if let Some(((start_line, start_col), (end_line, end_col))) = selection {
        if line_idx >= start_line && line_idx <= end_line {
            // Selection overlaps this line - need to apply selection styling
            let text_len = text.chars().count();
            let (sel_start, sel_end) = if line_idx == start_line && line_idx == end_line {
                (start_col.min(text_len), end_col.min(text_len))
            } else if line_idx == start_line {
                (start_col.min(text_len), text_len)
            } else if line_idx == end_line {
                (0, end_col.min(text_len))
            } else {
                (0, text_len)
            };

            if sel_start < sel_end {
                // Apply selection styling to spans
                return apply_selection_to_spans(spans, sel_start, sel_end, sel_style);
            }
        }
    }

    Line::from(spans)
}

/// Apply selection highlighting to a vector of spans
fn apply_selection_to_spans(
    spans: Vec<Span<'static>>,
    sel_start: usize,
    sel_end: usize,
    sel_style: Style,
) -> Line<'static> {
    let mut new_spans = Vec::with_capacity(spans.len() + 2);
    let mut char_pos = 0;

    for span in spans {
        let span_char_len = span.content.chars().count();
        let span_char_end = char_pos + span_char_len;

        if span_char_end <= sel_start || char_pos >= sel_end {
            // Span is entirely outside selection
            new_spans.push(span);
        } else if char_pos >= sel_start && span_char_end <= sel_end {
            // Span is entirely inside selection
            new_spans.push(Span::styled(span.content, sel_style));
        } else {
            // Span partially overlaps - split at character boundaries
            let local_start = sel_start.saturating_sub(char_pos);
            let local_end = (sel_end - char_pos).min(span_char_len);
            let chars: Vec<char> = span.content.chars().collect();

            if local_start > 0 {
                let before: String = chars[..local_start].iter().collect();
                new_spans.push(Span::styled(before, span.style));
            }
            let selected: String = chars[local_start..local_end].iter().collect();
            new_spans.push(Span::styled(selected, sel_style));
            if local_end < span_char_len {
                let after: String = chars[local_end..].iter().collect();
                new_spans.push(Span::styled(after, span.style));
            }
        }

        char_pos = span_char_end;
    }

    Line::from(new_spans)
}
