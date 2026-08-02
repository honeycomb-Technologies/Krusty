//! Shared floating-overlay chrome: centered title, body band, ruled footer shelf.
//!
//! Option B layout:
//! ```text
//! ╭─────────────────── Title ────────────────────╮
//! │  body…                                       │
//! ├──────────────────────────────────────────────┤
//! │       ↑/↓ choose  ·  Enter  ·  Esc close     │
//! ╰──────────────────────────────────────────────╯
//! ```
//!
//! One outer border owns containment. Bodies never draw borders or control
//! lines. Hints live only on the reserved bottom band and are centered.

use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::tui_v2::{
    layout::snapshot::intersect,
    model::{capability::CapabilityProfile, overlay::OverlayKind},
    presentation::{
        symbols::{Symbols, ASCII_BORDER},
        theme::SemanticTheme,
    },
};

/// Vertical rows reserved under the body: shelf rule + centered hints.
pub const FOOTER_SHELF_ROWS: u16 = 2;
/// Search field row + matching crossbar under it.
pub const SEARCH_BAND_ROWS: u16 = 2;
/// Extra horizontal inset inside the border so content does not kiss the frame.
pub const CONTENT_INSET: u16 = 1;

#[derive(Clone, Copy, Debug)]
pub struct OverlayChrome<'a> {
    pub title: &'a str,
    pub hints: &'a str,
}

/// Geometry returned after chrome paint.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OverlayChromeLayout {
    /// Full overlay rectangle including borders (for full-width crossbars).
    pub outer: Rect,
    /// Inset content rectangle above the footer shelf.
    pub body: Rect,
}

impl OverlayChromeLayout {
    /// Split body into search row, crossbar y (full outer width), and list.
    pub fn search_list(&self) -> Option<(Rect, u16, Rect)> {
        if self.body.height < SEARCH_BAND_ROWS {
            return None;
        }
        let search = Rect::new(self.body.x, self.body.y, self.body.width, 1);
        let crossbar_y = self.body.y.saturating_add(1);
        let list = Rect::new(
            self.body.x,
            self.body.y.saturating_add(SEARCH_BAND_ROWS),
            self.body.width,
            self.body.height.saturating_sub(SEARCH_BAND_ROWS),
        );
        Some((search, crossbar_y, list))
    }
}

impl OverlayChrome<'_> {
    pub fn for_overlay(kind: &OverlayKind, capability: CapabilityProfile) -> OverlayChrome<'static> {
        OverlayChrome {
            title: kind.label(),
            hints: overlay_hints(kind, capability),
        }
    }

    /// Paint chrome and return outer + body geometry for content and crossbars.
    pub fn render(
        self,
        frame: &mut Frame,
        area: Rect,
        theme: SemanticTheme,
        capability: CapabilityProfile,
    ) -> OverlayChromeLayout {
        let area = intersect(area, frame.area());
        if area.is_empty() {
            return OverlayChromeLayout {
                outer: area,
                body: area,
            };
        }

        let border_set = if capability.supports_rounded_borders() {
            ratatui::symbols::border::ROUNDED
        } else {
            ASCII_BORDER
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(border_set)
            .border_style(Style::default().fg(theme.border_focused))
            .style(Style::default().bg(theme.surface).fg(theme.foreground))
            .title(Line::from(self.title).centered().style(Style::default().fg(theme.identity)));

        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);

        if inner.is_empty() {
            return OverlayChromeLayout {
                outer: area,
                body: inner,
            };
        }

        let shelf_rows = FOOTER_SHELF_ROWS.min(inner.height);
        let body_height = inner.height.saturating_sub(shelf_rows);
        let body = Rect::new(inner.x, inner.y, inner.width, body_height);

        if shelf_rows >= 1 {
            let shelf_y = inner.y.saturating_add(body_height);
            paint_crossbar(frame, area, shelf_y, theme, capability);
        }
        if shelf_rows >= 2 {
            let hints_area = Rect::new(
                inner.x,
                inner.y.saturating_add(body_height.saturating_add(1)),
                inner.width,
                1,
            );
            paint_hints(frame, hints_area, self.hints, theme, capability);
        }

        OverlayChromeLayout {
            outer: area,
            body: inset(body, CONTENT_INSET),
        }
    }
}

/// Full-width T-junction rule matching the focused overlay border color.
pub fn paint_crossbar(
    frame: &mut Frame,
    outer: Rect,
    shelf_y: u16,
    theme: SemanticTheme,
    capability: CapabilityProfile,
) {
    if shelf_y < outer.y || shelf_y >= outer.bottom() || outer.width < 2 {
        return;
    }
    let symbols = Symbols::for_mode(capability.glyph_mode);
    let (left, right) = if capability.supports_rounded_borders() {
        ("├", "┤")
    } else {
        ("+", "+")
    };
    let mid_width = usize::from(outer.width.saturating_sub(2));
    let mut line = String::with_capacity(mid_width.saturating_add(2));
    line.push_str(left);
    for _ in 0..mid_width {
        line.push_str(symbols.divider);
    }
    line.push_str(right);
    frame.render_widget(
        Paragraph::new(line).style(
            Style::default()
                .fg(theme.border_focused)
                .bg(theme.surface),
        ),
        Rect::new(outer.x, shelf_y, outer.width, 1),
    );
}

fn paint_hints(
    frame: &mut Frame,
    area: Rect,
    hints: &str,
    theme: SemanticTheme,
    capability: CapabilityProfile,
) {
    if area.is_empty() {
        return;
    }
    let separator = if capability.glyph_mode
        == crate::tui_v2::model::capability::GlyphMode::Ascii
    {
        " | "
    } else {
        " · "
    };
    let fitted = fit_hint_segments(hints, separator, usize::from(area.width));
    frame.render_widget(
        Paragraph::new(fitted)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.foreground_muted).bg(theme.surface)),
        area,
    );
}

fn inset(area: Rect, pad: u16) -> Rect {
    if area.width <= pad.saturating_mul(2) || area.height == 0 {
        return Rect::new(area.x, area.y, area.width, area.height);
    }
    Rect::new(
        area.x.saturating_add(pad),
        area.y,
        area.width.saturating_sub(pad.saturating_mul(2)),
        area.height,
    )
}

/// Prefer whole hint segments; drop middle ones before truncating glyphs.
pub fn fit_hint_segments(hints: &str, separator: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(hints) <= max_width {
        return hints.to_owned();
    }
    let parts = hints
        .split(separator)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return fit_to_width(hints, max_width);
    }
    if parts.len() == 1 {
        return fit_to_width(parts[0], max_width);
    }

    // Keep first + last as long as possible; drop from the middle.
    for keep_middle in (0..parts.len().saturating_sub(2)).rev() {
        let mut selected = Vec::with_capacity(2 + keep_middle);
        selected.push(parts[0]);
        let start = 1;
        let end = parts.len() - 1;
        // Take a prefix of the middle slice.
        let middle = &parts[start..end];
        selected.extend(middle.iter().take(keep_middle).copied());
        selected.push(parts[end]);
        let candidate = selected.join(separator);
        if UnicodeWidthStr::width(candidate.as_str()) <= max_width {
            return candidate;
        }
    }

    let ends = [parts[0], parts[parts.len() - 1]];
    let candidate = ends.join(separator);
    if UnicodeWidthStr::width(candidate.as_str()) <= max_width {
        return candidate;
    }
    // Last resort: Esc / last segment alone, then hard truncate.
    let last = parts[parts.len() - 1];
    if UnicodeWidthStr::width(last) <= max_width {
        return last.to_owned();
    }
    fit_to_width(last, max_width)
}

fn fit_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_owned();
    }
    if max_width <= 1 {
        return "…".chars().take(max_width).collect();
    }
    let mut output = String::new();
    let mut width = 0usize;
    let budget = max_width.saturating_sub(1);
    for ch in text.chars() {
        let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
        if width.saturating_add(ch_width) > budget {
            break;
        }
        output.push(ch);
        width = width.saturating_add(ch_width);
    }
    output.push('…');
    output
}

pub fn overlay_hints(kind: &OverlayKind, capability: CapabilityProfile) -> &'static str {
    let ascii = matches!(
        capability.glyph_mode,
        crate::tui_v2::model::capability::GlyphMode::Ascii
    );
    match kind {
        OverlayKind::CommandPalette => {
            if ascii {
                "Up/Down choose | Enter run | Esc close"
            } else {
                "↑/↓ choose  ·  Enter run  ·  Esc close"
            }
        }
        OverlayKind::Help => {
            if ascii {
                "Esc close"
            } else {
                "Esc close"
            }
        }
        OverlayKind::SessionPicker => {
            if ascii {
                "Up/Down choose | Enter open | Esc close"
            } else {
                "↑/↓ choose  ·  Enter open  ·  Esc close"
            }
        }
        OverlayKind::ModelPicker => {
            if ascii {
                "Up/Down choose | Enter select | Esc close"
            } else {
                "↑/↓ choose  ·  Enter select  ·  Esc close"
            }
        }
        OverlayKind::Connections => {
            if ascii {
                "Up/Down choose | Enter continue | Esc close"
            } else {
                "↑/↓ choose  ·  Enter continue  ·  Esc close"
            }
        }
        OverlayKind::ThemeAppearance => {
            if ascii {
                "Up/Down choose | Enter apply | Esc close"
            } else {
                "↑/↓ choose  ·  Enter apply  ·  Esc close"
            }
        }
        OverlayKind::PlanGoal => {
            if ascii {
                "Esc close"
            } else {
                "Esc close"
            }
        }
        OverlayKind::Processes => {
            if ascii {
                "Up/Down choose | Enter stop | Esc close"
            } else {
                "↑/↓ choose  ·  Enter stop  ·  Esc close"
            }
        }
        OverlayKind::ExtensionsCenter => {
            if ascii {
                "Up/Down choose | Enter toggle | Esc close"
            } else {
                "↑/↓ choose  ·  Enter toggle  ·  Esc close"
            }
        }
        OverlayKind::FileArtifactInspector { .. } => {
            if ascii {
                "PgUp/PgDn scroll | c copy | Esc close"
            } else {
                "PgUp/PgDn scroll  ·  c copy  ·  Esc close"
            }
        }
        OverlayKind::AttachmentPreview => {
            if ascii {
                "Esc close"
            } else {
                "Esc close"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    use crate::tui_v2::{
        model::{
            capability::{CapabilityProfile, ColorDepth, GlyphMode},
            overlay::OverlayKind,
        },
        presentation::theme::{SemanticTheme, ThemeKind},
    };

    use super::*;

    fn capability() -> CapabilityProfile {
        CapabilityProfile {
            glyph_mode: GlyphMode::Ascii,
            color_depth: ColorDepth::TrueColor,
        }
    }

    #[test]
    fn shelf_splits_body_from_centered_footer_and_keeps_equal_insets() {
        let mut terminal = Terminal::new(TestBackend::new(48, 12)).expect("terminal");
        let capability = capability();
        let theme = SemanticTheme::resolve(ThemeKind::MitsuroDark, capability.color_depth);
        let area = Rect::new(2, 1, 44, 10);

        let mut layout = OverlayChromeLayout::default();
        terminal
            .draw(|frame| {
                layout = OverlayChrome {
                    title: "Sessions",
                    hints: "Up/Down choose | Enter open | Esc close",
                }
                .render(frame, area, theme, capability);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        // Outer box corners.
        assert_eq!(buffer.cell((2, 1)).expect("tl").symbol(), "+");
        assert_eq!(buffer.cell((45, 1)).expect("tr").symbol(), "+");
        assert_eq!(buffer.cell((2, 10)).expect("bl").symbol(), "+");
        assert_eq!(buffer.cell((45, 10)).expect("br").symbol(), "+");

        // Shelf T-junctions on the vertical borders, same color family as the frame.
        let shelf_y = 8u16; // area y=1 h=10 → inner y=2 h=8 → body_h=6 → shelf y=8
        assert_eq!(buffer.cell((2, shelf_y)).expect("shelf L").symbol(), "+");
        assert_eq!(buffer.cell((45, shelf_y)).expect("shelf R").symbol(), "+");
        assert_eq!(buffer.cell((3, shelf_y)).expect("shelf mid").symbol(), "-");
        assert_eq!(
            buffer.cell((3, shelf_y)).expect("shelf mid").style().fg,
            Some(theme.border_focused)
        );
        assert_eq!(
            buffer.cell((2, 1)).expect("tl").style().fg,
            Some(theme.border_focused)
        );

        // Hints centered on the footer band.
        let hints_y = 9u16;
        let row: String = (3..45)
            .map(|x| buffer.cell((x, hints_y)).expect("hint cell").symbol().to_owned())
            .collect::<Vec<_>>()
            .join("");
        let trimmed = row.trim();
        assert!(
            trimmed.contains("Enter open") && trimmed.contains("Esc close"),
            "expected centered hints, got {trimmed:?}"
        );
        let start = row.find(trimmed).expect("trim start");
        let end = start + trimmed.len();
        let left_pad = start;
        let right_pad = row.len().saturating_sub(end);
        assert!(
            left_pad.abs_diff(right_pad) <= 1,
            "hints should be centered, left={left_pad} right={right_pad} row={row:?}"
        );

        // Body inset is symmetric and does not include the shelf.
        assert_eq!(layout.outer, area);
        assert_eq!(layout.body.x, area.x + 1 + CONTENT_INSET);
        assert_eq!(layout.body.right(), area.right() - 1 - CONTENT_INSET);
        assert_eq!(layout.body.y, area.y + 1);
        assert_eq!(layout.body.bottom(), shelf_y);

        let (search, cross_y, list) = layout.search_list().expect("search band");
        assert_eq!(search.y, layout.body.y);
        assert_eq!(cross_y, layout.body.y + 1);
        assert_eq!(list.y, layout.body.y + 2);
    }

    #[test]
    fn hint_elision_drops_middle_segments_before_hard_truncate() {
        let full = "Up/Down choose | Enter open | Esc close";
        let ends = "Up/Down choose | Esc close";
        assert!(UnicodeWidthStr::width(full) > UnicodeWidthStr::width(ends));

        let fitted = fit_hint_segments(full, " | ", UnicodeWidthStr::width(ends));
        assert_eq!(fitted, ends);

        let tight = fit_hint_segments(full, " | ", UnicodeWidthStr::width("Esc close"));
        assert_eq!(tight, "Esc close");
    }

    #[test]
    fn overlay_hints_cover_every_kind() {
        let capability = capability();
        for kind in [
            OverlayKind::CommandPalette,
            OverlayKind::Help,
            OverlayKind::SessionPicker,
            OverlayKind::ModelPicker,
            OverlayKind::Connections,
            OverlayKind::ThemeAppearance,
            OverlayKind::PlanGoal,
            OverlayKind::Processes,
            OverlayKind::ExtensionsCenter,
            OverlayKind::FileArtifactInspector {
                part_id: crate::tui_v2::model::artifact::PartId::from_semantic("p"),
            },
            OverlayKind::AttachmentPreview,
        ] {
            assert!(!overlay_hints(&kind, capability).is_empty());
            assert_eq!(OverlayChrome::for_overlay(&kind, capability).title, kind.label());
        }
    }
}
