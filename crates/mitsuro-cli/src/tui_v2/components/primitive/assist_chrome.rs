//! Composer-assist panel chrome (slash / `@` pickers).
//!
//! Clean title-less popup: focused purple border only, scrollable list body.
//! No footer bar / hint shelf — keeps `/` and `@` menus uncluttered.

use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear},
    Frame,
};

use crate::tui_v2::{
    layout::snapshot::intersect,
    model::capability::CapabilityProfile,
    presentation::{symbols::ASCII_BORDER, theme::SemanticTheme},
};

/// Outer border overhead (top + bottom only).
pub const ASSIST_CHROME_ROWS: u16 = 2;
/// Horizontal inset inside the border so rows do not kiss the frame.
pub const CONTENT_INSET: u16 = 1;

#[derive(Clone, Copy, Debug, Default)]
pub struct AssistChrome;

/// Geometry after chrome paint.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AssistChromeLayout {
    pub outer: Rect,
    /// Scrollable list body (inset).
    pub body: Rect,
}

impl AssistChrome {
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
        // Title-less, footer-less block — border only.
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(border_set)
            .border_style(Style::default().fg(theme.border_focused))
            .style(Style::default().bg(theme.surface).fg(theme.foreground));

        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);

        AssistChromeLayout {
            outer: area,
            body: inset(inner, CONTENT_INSET),
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
