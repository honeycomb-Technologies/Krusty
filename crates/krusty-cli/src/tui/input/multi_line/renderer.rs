//! Rendering logic for multi-line input

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

use super::MultiLineInput;

impl MultiLineInput {
    /// Render with optional file reference styling
    pub fn render_styled_with_file_refs(
        &self,
        _area: Rect,
        bg_color: Color,
        border_color: Color,
        accent_color: Color,
        selection: Option<((usize, usize), (usize, usize))>,
        sel_bg: Color,
        sel_fg: Color,
        link_color: Option<Color>,
        hover_range: Option<(usize, usize)>,
    ) -> Paragraph<'_> {
        let lines = self.get_wrapped_lines();
        let file_ref_ranges = if link_color.is_some() {
            self.get_file_ref_ranges()
        } else {
            vec![]
        };

        // Pre-compute byte offsets for each line
        let line_byte_offsets = self.compute_line_byte_offsets(&lines);

        let visible_lines: Vec<Line> = lines
            .iter()
            .skip(self.viewport_offset)
            .take(self.max_visible_lines as usize)
            .enumerate()
            .map(|(idx, line)| {
                let global_line_idx = self.viewport_offset + idx;
                let line_byte_start = line_byte_offsets.get(global_line_idx).copied().unwrap_or(0);
                let mut spans = vec![Span::raw(" ")]; // Left padding

                // Check if this line has selection
                let line_selection = get_line_selection(global_line_idx, line.len(), selection);

                // Check if cursor is on this line
                if global_line_idx == self.cursor_visual.0 {
                    let (_, cursor_col) = self.cursor_visual;
                    render_line_with_cursor_and_file_refs(
                        &mut spans,
                        line,
                        cursor_col,
                        line_selection,
                        accent_color,
                        sel_bg,
                        sel_fg,
                        link_color,
                        &file_ref_ranges,
                        hover_range,
                        line_byte_start,
                    );
                } else if let Some((sel_start, sel_end)) = line_selection {
                    // Line has selection but no cursor
                    render_line_with_selection(
                        &mut spans,
                        line,
                        sel_start,
                        sel_end,
                        accent_color,
                        sel_bg,
                        sel_fg,
                    );
                } else {
                    // No cursor, no selection - check for file refs
                    if !line.is_empty() {
                        if let Some(lc) = link_color {
                            render_line_with_file_refs(
                                &mut spans,
                                line,
                                accent_color,
                                lc,
                                &file_ref_ranges,
                                hover_range,
                                line_byte_start,
                            );
                        } else {
                            spans.push(Span::styled(
                                line.clone(),
                                Style::default().fg(accent_color),
                            ));
                        }
                    }
                }

                if spans.len() == 1 {
                    // Just the padding, add a space for empty lines
                    spans.push(Span::raw(" "));
                }

                Line::from(spans)
            })
            .collect();

        Paragraph::new(visible_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border_color)),
            )
            .style(Style::default().bg(bg_color).fg(accent_color))
    }

    /// Compute the byte offset of each wrapped line's start in the original content
    fn compute_line_byte_offsets(&self, lines: &[String]) -> Vec<usize> {
        let mut offsets = Vec::with_capacity(lines.len());
        let mut byte_pos = 0;

        for (idx, line) in lines.iter().enumerate() {
            offsets.push(byte_pos);
            byte_pos += line.len();

            // Check for hard newline after this line (not soft wrap)
            if idx < lines.len() - 1
                && byte_pos < self.content.len()
                && self.content.as_bytes().get(byte_pos) == Some(&b'\n')
            {
                byte_pos += 1;
            }
        }

        offsets
    }
}

/// Render a line with file reference highlighting
/// `line_byte_start` is the byte offset of this line's start in the original content
fn render_line_with_file_refs(
    spans: &mut Vec<Span<'static>>,
    line: &str,
    accent_color: Color,
    link_color: Color,
    file_ref_ranges: &[(usize, usize)],
    hover_range: Option<(usize, usize)>,
    line_byte_start: usize,
) {
    let base_style = Style::default().fg(accent_color);
    let link_style = Style::default()
        .fg(link_color)
        .add_modifier(Modifier::UNDERLINED);
    // Hover style: inverted colors for clear visibility
    let hover_style = Style::default()
        .fg(Color::Black)
        .bg(link_color)
        .add_modifier(Modifier::BOLD);

    if file_ref_ranges.is_empty() {
        spans.push(Span::styled(line.to_string(), base_style));
        return;
    }

    let line_byte_end = line_byte_start + line.len();

    // Find which global file_ref_ranges overlap with this line
    let mut highlight_ranges: Vec<(usize, usize, bool)> = Vec::new();
    for &(ref_start, ref_end) in file_ref_ranges {
        // Check if this file ref overlaps with current line
        if ref_end > line_byte_start && ref_start < line_byte_end {
            // Calculate the overlap within this line (as byte offsets within line)
            let local_start = ref_start.saturating_sub(line_byte_start);
            let local_end = (ref_end - line_byte_start).min(line.len());

            let is_hovered = hover_range.is_some_and(|(hs, he)| hs == ref_start && he == ref_end);
            highlight_ranges.push((local_start, local_end, is_hovered));
        }
    }

    if highlight_ranges.is_empty() {
        spans.push(Span::styled(line.to_string(), base_style));
        return;
    }

    // Sort by start position
    highlight_ranges.sort_by_key(|(s, _, _)| *s);

    // Render with highlights
    let mut last_end = 0;
    for (start, end, is_hovered) in highlight_ranges {
        // Add text before highlight
        if start > last_end {
            spans.push(Span::styled(line[last_end..start].to_string(), base_style));
        }
        // Add highlighted text
        let style = if is_hovered { hover_style } else { link_style };
        spans.push(Span::styled(line[start..end].to_string(), style));
        last_end = end;
    }
    // Add remaining text
    if last_end < line.len() {
        spans.push(Span::styled(line[last_end..].to_string(), base_style));
    }
}

/// Get selection bounds for a specific line, returns (start_col, end_col) if selected
fn get_line_selection(
    line_idx: usize,
    line_len: usize,
    selection: Option<((usize, usize), (usize, usize))>,
) -> Option<(usize, usize)> {
    let ((start_line, start_col), (end_line, end_col)) = selection?;

    if line_idx < start_line || line_idx > end_line {
        return None;
    }

    let sel_start = if line_idx == start_line {
        start_col.min(line_len)
    } else {
        0
    };
    let sel_end = if line_idx == end_line {
        end_col.min(line_len)
    } else {
        line_len
    };

    if sel_start < sel_end {
        Some((sel_start, sel_end))
    } else {
        None
    }
}

/// Render a line with cursor, optional selection, AND file reference styling
/// `line_byte_start` is the byte offset of this line's start in the original content
#[allow(clippy::too_many_arguments)]
fn render_line_with_cursor_and_file_refs(
    spans: &mut Vec<Span<'static>>,
    line: &str,
    cursor_col: usize,
    selection: Option<(usize, usize)>,
    accent_color: Color,
    sel_bg: Color,
    sel_fg: Color,
    link_color: Option<Color>,
    file_ref_ranges: &[(usize, usize)],
    hover_range: Option<(usize, usize)>,
    line_byte_start: usize,
) {
    let base_style = Style::default().fg(accent_color);
    let sel_style = Style::default()
        .bg(sel_bg)
        .fg(sel_fg)
        .add_modifier(Modifier::BOLD);

    // Build link/hover styles if link_color provided
    let (link_style, hover_style) = if let Some(lc) = link_color {
        (
            Style::default().fg(lc).add_modifier(Modifier::UNDERLINED),
            Style::default()
                .fg(Color::Black)
                .bg(lc)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (base_style, base_style)
    };

    let line_byte_end = line_byte_start + line.len();

    // Find which global file_ref_ranges overlap with this line and convert to local byte ranges
    let local_highlight_ranges: Vec<(usize, usize, bool)> = file_ref_ranges
        .iter()
        .filter_map(|&(ref_start, ref_end)| {
            if ref_end > line_byte_start && ref_start < line_byte_end {
                let local_start = ref_start.saturating_sub(line_byte_start);
                let local_end = (ref_end - line_byte_start).min(line.len());
                let is_hovered =
                    hover_range.is_some_and(|(hs, he)| hs == ref_start && he == ref_end);
                Some((local_start, local_end, is_hovered))
            } else {
                None
            }
        })
        .collect();

    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let cursor_col = cursor_col.min(len);

    // Build a byte-to-char mapping for range lookup
    let mut byte_to_char: Vec<usize> = Vec::with_capacity(line.len() + 1);
    for (char_idx, ch) in chars.iter().enumerate() {
        for _ in 0..ch.len_utf8() {
            byte_to_char.push(char_idx);
        }
    }
    byte_to_char.push(len); // End sentinel

    // Convert byte ranges to char ranges
    let char_highlight_ranges: Vec<(usize, usize, bool)> = local_highlight_ranges
        .iter()
        .filter_map(|(start, end, is_hovered)| {
            let start_char = byte_to_char.get(*start).copied()?;
            let end_char = byte_to_char.get((*end).min(line.len())).copied()?;
            Some((start_char, end_char, *is_hovered))
        })
        .collect();

    // Determine selection bounds
    let (sel_start, sel_end) = selection
        .map(|(s, e)| (s.min(len), e.min(len)))
        .filter(|(s, e)| s < e)
        .unwrap_or((len, len)); // No selection

    // Render character by character
    for (i, ch) in chars.iter().enumerate() {
        // Insert cursor before this char if needed
        if i == cursor_col {
            spans.push(Span::styled("█", base_style));
        }

        // Determine style for this character
        let style = if i >= sel_start && i < sel_end {
            // Selection takes priority
            sel_style
        } else if link_color.is_some() {
            // Check if in a file ref highlight range
            let highlight_info = char_highlight_ranges
                .iter()
                .find(|(s, e, _)| i >= *s && i < *e);
            if let Some((_, _, is_hovered)) = highlight_info {
                if *is_hovered {
                    hover_style
                } else {
                    link_style
                }
            } else {
                base_style
            }
        } else {
            base_style
        };

        spans.push(Span::styled(ch.to_string(), style));
    }

    // Cursor at end of line
    if cursor_col >= len {
        spans.push(Span::styled("█", base_style));
    }
}

/// Render a line with selection (no cursor)
fn render_line_with_selection(
    spans: &mut Vec<Span<'static>>,
    line: &str,
    sel_start: usize,
    sel_end: usize,
    accent_color: Color,
    sel_bg: Color,
    sel_fg: Color,
) {
    let base_style = Style::default().fg(accent_color);
    let sel_style = Style::default()
        .bg(sel_bg)
        .fg(sel_fg)
        .add_modifier(Modifier::BOLD);

    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();

    // Clamp indices to valid range
    let sel_start = sel_start.min(len);
    let sel_end = sel_end.min(len);

    if sel_start >= sel_end {
        // No valid selection, render normally
        if !line.is_empty() {
            spans.push(Span::styled(line.to_string(), base_style));
        }
        return;
    }

    let before: String = chars[..sel_start].iter().collect();
    let selected: String = chars[sel_start..sel_end].iter().collect();
    let after: String = chars[sel_end..].iter().collect();

    if !before.is_empty() {
        spans.push(Span::styled(before, base_style));
    }
    if !selected.is_empty() {
        spans.push(Span::styled(selected, sel_style));
    }
    if !after.is_empty() {
        spans.push(Span::styled(after, base_style));
    }
}
