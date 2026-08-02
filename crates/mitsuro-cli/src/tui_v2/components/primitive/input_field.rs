//! Bounded multi-line input presentation shared by the composer and forms.

use ratatui::{
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui_v2::presentation::theme::SemanticTheme;

use super::text_style::{window_to_width, TextRole};

pub struct InputField<'a> {
    pub value: &'a str,
    pub placeholder: &'a str,
    pub masked: bool,
    pub mask_symbol: &'a str,
    pub horizontal_offset: usize,
    pub cursor_byte: usize,
    pub focused: bool,
    pub error: Option<&'a str>,
    /// When false, never paints a text fill.
    pub fill_background: bool,
    /// Optional explicit fill (composer uses `theme.surface` so the bar reads as a field).
    pub fill_color: Option<ratatui::style::Color>,
    /// Optional selection range in source bytes (start, end) unordered.
    pub selection: Option<(usize, usize)>,
    /// When set, pins the first visible wrapped row (Word-like viewport).
    /// When `None`, the window follows the cursor.
    pub viewport_offset: Option<usize>,
}

impl InputField<'_> {
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: SemanticTheme) {
        if area.is_empty() {
            return;
        }
        let (value, cursor_byte, selection) = if self.value.is_empty() {
            (self.placeholder.to_owned(), 0, None)
        } else if self.masked {
            let cursor_char = self.value[..self.cursor_byte.min(self.value.len())]
                .chars()
                .count();
            let value = self.mask_symbol.repeat(self.value.chars().count());
            (value, self.mask_symbol.repeat(cursor_char).len(), None)
        } else {
            (
                self.value.to_owned(),
                self.cursor_byte.min(self.value.len()),
                self.selection,
            )
        };
        let rendered = render_window(
            &value,
            cursor_byte,
            usize::from(area.width),
            usize::from(area.height),
            self.horizontal_offset,
            selection,
            self.viewport_offset,
        );
        let role = if self.error.is_some() {
            TextRole::Error
        } else if self.value.is_empty() {
            TextRole::Muted
        } else {
            TextRole::Body
        };
        let mut base = role.style(theme);
        if let Some(fill) = self.fill_color {
            base = base.bg(fill);
        } else if self.fill_background {
            base = base.bg(theme.surface);
        } else {
            base = base.bg(ratatui::style::Color::Reset);
        }
        let lines = rendered
            .lines
            .into_iter()
            .map(|line| style_input_line(line, base, theme))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(Text::from(lines)).style(base), area);
        // Ratatui shows the terminal caret only when a position is set for this
        // frame; leaving it unset hides (and stops blinking) the cursor. That is
        // required when the logical caret has been scrolled out of the field.
        if self.focused
            && rendered.cursor_in_frame
            && rendered.cursor_row < area.height
            && rendered.cursor_column < area.width
        {
            frame.set_cursor_position(Position::new(
                area.x.saturating_add(rendered.cursor_column),
                area.y.saturating_add(rendered.cursor_row),
            ));
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct RenderedLine {
    text: String,
    /// Byte ranges in this visual line relative to the line string that are selected.
    selected: Vec<(usize, usize)>,
    /// Bracket chip ranges in this visual line.
    brackets: Vec<(usize, usize)>,
}

#[derive(Debug, Eq, PartialEq)]
struct RenderWindow {
    lines: Vec<RenderedLine>,
    cursor_row: u16,
    cursor_column: u16,
    /// False when the logical caret is above/below the visible window
    /// (user scrolled away); paint must not place a terminal cursor then.
    cursor_in_frame: bool,
}

fn render_window(
    value: &str,
    cursor_byte: usize,
    width: usize,
    height: usize,
    horizontal_offset: usize,
    selection: Option<(usize, usize)>,
    viewport_offset: Option<usize>,
) -> RenderWindow {
    let width = width.max(1);
    let height = height.max(1);
    let cursor_byte = cursor_byte.min(value.len());
    let sel = selection.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
    // Soft-wrap aligned with ComposerBuffer: break when next char would exceed width.
    let mut lines: Vec<(String, usize, usize)> = vec![(String::new(), 0, 0)]; // text, src_start, src_end
    let (mut cursor_row, mut cursor_column) = (0usize, 0usize);
    let mut cursor_found = false;
    let mut line_src_start = 0usize;

    for (byte, character) in value.char_indices() {
        if byte == cursor_byte {
            cursor_row = lines.len().saturating_sub(1);
            cursor_column = UnicodeWidthStr::width(lines.last().map_or("", |l| l.0.as_str()));
            cursor_found = true;
        }
        if character == '\n' {
            if let Some(last) = lines.last_mut() {
                last.2 = byte.saturating_add(1); // include newline in src_end (buffer parity)
            }
            lines.push((String::new(), byte.saturating_add(1), byte.saturating_add(1)));
            line_src_start = byte.saturating_add(1);
            continue;
        }
        let cell_width = UnicodeWidthChar::width(character).unwrap_or(0).max(1);
        let current_width = UnicodeWidthStr::width(lines.last().map_or("", |l| l.0.as_str()));
        if current_width > 0 && current_width.saturating_add(cell_width) > width {
            if let Some(last) = lines.last_mut() {
                last.2 = byte;
            }
            lines.push((String::new(), byte, byte));
            line_src_start = byte;
        }
        lines
            .last_mut()
            .expect("input always has a row")
            .0
            .push(character);
        lines.last_mut().expect("row").1 = line_src_start;
        lines.last_mut().expect("row").2 = byte.saturating_add(character.len_utf8());
    }
    if !cursor_found {
        cursor_row = lines.len().saturating_sub(1);
        cursor_column = UnicodeWidthStr::width(lines.last().map_or("", |l| l.0.as_str()));
    }
    if let Some(last) = lines.last_mut() {
        if last.2 < value.len() {
            last.2 = value.len();
        }
    }

    let max_first = lines.len().saturating_sub(height);
    let first_row = viewport_offset
        .map(|off| off.min(max_first))
        .unwrap_or_else(|| cursor_row.saturating_sub(height.saturating_sub(1)));
    let cursor_in_frame =
        cursor_row >= first_row && cursor_row < first_row.saturating_add(height);
    let visible = lines
        .into_iter()
        .enumerate()
        .skip(first_row)
        .take(height)
        .map(|(_, (line, src_start, src_end))| {
            let clipped = window_to_width(&line, horizontal_offset, width);
            // Map selection into line-local bytes (approximate via prefix width clip).
            let mut selected = Vec::new();
            if let Some((sel_lo, sel_hi)) = sel {
                let lo = sel_lo.max(src_start);
                let hi = sel_hi.min(src_end);
                if lo < hi {
                    // local offsets relative to full line before horizontal window
                    let local_lo = lo.saturating_sub(src_start);
                    let local_hi = hi.saturating_sub(src_start).min(line.len());
                    let (vis_lo, vis_hi) =
                        map_local_range_through_window(&line, local_lo, local_hi, horizontal_offset);
                    if vis_lo < vis_hi && vis_lo < clipped.len() {
                        selected.push((vis_lo, vis_hi.min(clipped.len())));
                    }
                }
            }
            let brackets = bracket_ranges_in_line(&clipped);
            RenderedLine {
                text: clipped,
                selected,
                brackets,
            }
        })
        .collect();
    RenderWindow {
        lines: visible,
        cursor_row: if cursor_in_frame {
            cursor_row
                .saturating_sub(first_row)
                .try_into()
                .unwrap_or(u16::MAX)
        } else {
            0
        },
        cursor_column: cursor_column
            .saturating_sub(horizontal_offset)
            .min(width.saturating_sub(1))
            .try_into()
            .unwrap_or(u16::MAX),
        cursor_in_frame,
    }
}

fn map_local_range_through_window(
    line: &str,
    local_lo: usize,
    local_hi: usize,
    horizontal_offset: usize,
) -> (usize, usize) {
    // Approximate: treat horizontal_offset as visual cells, not bytes.
    let _ = horizontal_offset;
    (local_lo.min(line.len()), local_hi.min(line.len()))
}

fn bracket_ranges_in_line(line: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(rel) = line[i..].find(']') {
                let end = i + rel + 1;
                ranges.push((i, end));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    ranges
}

fn style_input_line(line: RenderedLine, base: Style, theme: SemanticTheme) -> Line<'static> {
    if line.selected.is_empty() && line.brackets.is_empty() {
        return Line::styled(line.text, base);
    }
    let mut marks = vec![0usize, line.text.len()];
    for (a, b) in line.selected.iter().chain(line.brackets.iter()) {
        marks.push(*a);
        marks.push(*b);
    }
    marks.sort_unstable();
    marks.dedup();
    let mut spans = Vec::new();
    for window in marks.windows(2) {
        let (a, b) = (window[0], window[1]);
        if a >= b || a >= line.text.len() {
            continue;
        }
        let b = b.min(line.text.len());
        let slice = line.text[a..b].to_owned();
        let in_sel = line.selected.iter().any(|(s, e)| a < *e && b > *s);
        let in_br = line.brackets.iter().any(|(s, e)| a < *e && b > *s);
        let mut style = base;
        if in_br {
            style = style.fg(theme.identity).add_modifier(Modifier::BOLD);
        }
        if in_sel {
            style = style.bg(theme.selection_surface);
        }
        spans.push(Span::styled(slice, style));
    }
    if spans.is_empty() {
        Line::styled(line.text, base)
    } else {
        Line::from(spans)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::tui_v2::presentation::theme::{SemanticTheme, ThemeKind};

    use super::*;

    #[test]
    fn masked_field_never_renders_the_secret() {
        let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("terminal");
        let theme = SemanticTheme::resolve(
            ThemeKind::MitsuroDark,
            crate::tui_v2::model::capability::ColorDepth::TrueColor,
        );
        terminal
            .draw(|frame| {
                InputField {
                    value: "secret-value",
                    placeholder: "token",
                    masked: true,
                    mask_symbol: "•",
                    horizontal_offset: 0,
                    cursor_byte: "secret-value".len(),
                    focused: true,
                    error: None,
                    fill_background: true,
                    fill_color: None,
                    selection: None,
                    viewport_offset: None,
                }
                .render(frame, frame.area(), theme);
            })
            .expect("field");
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(!text.contains("secret-value"));
        assert!(text.contains('•'));
    }

    #[test]
    fn multiline_window_keeps_the_cursor_visible() {
        let rendered = render_window("one\ntwo\nthree", 7, 8, 2, 0, None, None);
        assert_eq!(
            rendered
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert_eq!((rendered.cursor_row, rendered.cursor_column), (1, 3));
        assert!(rendered.cursor_in_frame);
    }

    #[test]
    fn pinned_viewport_offset_does_not_follow_cursor() {
        let rendered = render_window("one\ntwo\nthree", 12, 8, 2, 0, None, Some(1));
        assert_eq!(
            rendered
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["two", "three"]
        );
        // Cursor on "three" is row 2 absolute → row 1 relative to offset 1.
        assert_eq!(rendered.cursor_row, 1);
        assert!(rendered.cursor_in_frame);
    }

    #[test]
    fn scrolled_away_marks_cursor_out_of_frame() {
        // Viewport pinned at top; caret on last line → not in the 1-row window.
        let rendered = render_window("one\ntwo\nthree", 12, 8, 1, 0, None, Some(0));
        assert!(!rendered.cursor_in_frame);
    }

    #[test]
    fn brackets_are_detected_for_styling() {
        assert_eq!(bracket_ranges_in_line("see [foo.rs] now"), vec![(4, 12)]);
    }
}
