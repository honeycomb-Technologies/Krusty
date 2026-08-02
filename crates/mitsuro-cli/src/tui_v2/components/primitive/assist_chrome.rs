//! Composer-assist panel chrome (slash / `@` pickers).
//!
//! Popup Option-B format: focused purple border + footer shelf with centered
//! hints — but **no title** on the top edge.

use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui_v2::{
    components::primitive::overlay_chrome::{paint_crossbar, FOOTER_SHELF_ROWS},
    layout::snapshot::intersect,
    model::capability::CapabilityProfile,
    presentation::{symbols::ASCII_BORDER, theme::SemanticTheme},
};

/// Outer border + shelf chrome overhead (top, bottom, crossbar, hints).
pub const ASSIST_CHROME_ROWS: u16 = 4;
/// Horizontal inset inside the border so rows do not kiss the frame.
pub const CONTENT_INSET: u16 = 1;

#[derive(Clone, Copy, Debug)]
pub struct AssistChrome<'a> {
    pub hints: &'a str,
}

/// Geometry after chrome paint.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AssistChromeLayout {
    pub outer: Rect,
    /// Scrollable list body (inset).
    pub body: Rect,
}

impl AssistChrome<'_> {
    pub fn render(
        self,
        frame: &mut Frame,
        area: Rect,
        theme: SemanticTheme,
        capability: CapabilityProfile,
    ) -> AssistChromeLayout {
        let area = intersect(area, frame.area());
        if area.is_empty() {
            return AssistChromeLayout {
                outer: area,
                body: area,
            };
        }

        let border_set = if capability.supports_rounded_borders() {
            ratatui::symbols::border::ROUNDED
        } else {
            ASCII_BORDER
        };
        // Title-less block — same border family as floating popups.
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(border_set)
            .border_style(Style::default().fg(theme.border_focused))
            .style(Style::default().bg(theme.surface).fg(theme.foreground));

        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);

        if inner.is_empty() {
            return AssistChromeLayout {
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
            frame.render_widget(
                Paragraph::new(self.hints)
                    .alignment(Alignment::Center)
                    .style(
                        Style::default()
                            .fg(theme.foreground_muted)
                            .bg(theme.surface),
                    ),
                hints_area,
            );
        }

        AssistChromeLayout {
            outer: area,
            body: inset(body, CONTENT_INSET),
        }
    }
}

fn inset(area: Rect, pad: u16) -> Rect {
    if area.width <= pad.saturating_mul(2) || area.height == 0 {
        return area;
    }
    Rect::new(
        area.x.saturating_add(pad),
        area.y,
        area.width.saturating_sub(pad.saturating_mul(2)),
        area.height,
    )
}
