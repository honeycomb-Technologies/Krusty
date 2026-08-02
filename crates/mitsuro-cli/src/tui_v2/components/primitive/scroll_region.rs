//! Shared scroll state and scrollbar renderer.

use std::ops::Range;

use ratatui::{layout::Rect, style::Style, widgets::Paragraph, Frame};

use crate::tui_v2::{
    model::capability::{CapabilityProfile, GlyphMode},
    presentation::theme::SemanticTheme,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScrollRegionState {
    pub offset: u32,
    pub content_height: u32,
    pub viewport_height: u16,
    pub follow_live: bool,
    pub unseen: usize,
}

impl Default for ScrollRegionState {
    fn default() -> Self {
        Self {
            offset: 0,
            content_height: 0,
            viewport_height: 0,
            follow_live: true,
            unseen: 0,
        }
    }
}

impl ScrollRegionState {
    pub fn reconcile(&mut self, content_height: u32, viewport_height: u16) {
        self.content_height = content_height;
        self.viewport_height = viewport_height;
        let maximum = self.maximum_offset();
        self.offset = if self.follow_live {
            maximum
        } else {
            self.offset.min(maximum)
        };
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let next = i64::from(self.offset).saturating_add(i64::from(delta));
        self.offset = next.clamp(0, i64::from(self.maximum_offset())) as u32;
        self.follow_live = self.offset == self.maximum_offset();
        if self.follow_live {
            self.unseen = 0;
        }
    }

    pub fn maximum_offset(&self) -> u32 {
        self.content_height
            .saturating_sub(u32::from(self.viewport_height))
    }

    pub fn thumb(&self) -> Option<Range<u16>> {
        if self.content_height <= u32::from(self.viewport_height) || self.viewport_height == 0 {
            return None;
        }
        let track = u32::from(self.viewport_height);
        let thumb_height = (track.saturating_mul(track) / self.content_height).max(1);
        let travel = track.saturating_sub(thumb_height);
        let start = if self.maximum_offset() == 0 {
            0
        } else {
            self.offset.saturating_mul(travel) / self.maximum_offset()
        };
        Some(
            u16::try_from(start).unwrap_or(u16::MAX)
                ..u16::try_from(start.saturating_add(thumb_height)).unwrap_or(u16::MAX),
        )
    }
}

pub struct ScrollRegion;

impl ScrollRegion {
    pub fn render_scrollbar(
        frame: &mut Frame,
        area: Rect,
        state: &ScrollRegionState,
        capability: CapabilityProfile,
        theme: SemanticTheme,
    ) {
        let Some(thumb) = state.thumb() else {
            return;
        };
        let x = area.right().saturating_sub(1);
        for row in 0..area.height {
            let symbol = match (capability.glyph_mode, thumb.contains(&row)) {
                (GlyphMode::Unicode, true) => "│",
                (GlyphMode::Unicode, false) => "·",
                (GlyphMode::Ascii, true) => "|",
                (GlyphMode::Ascii, false) => ".",
            };
            let cell = Rect::new(x, area.y.saturating_add(row), 1, 1);
            frame.render_widget(
                Paragraph::new(symbol).style(Style::default().bg(theme.surface).fg(
                    if thumb.contains(&row) {
                        theme.border_focused
                    } else {
                        theme.border
                    },
                )),
                cell,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_reconciliation_preserves_manual_offset_and_follow_live_truth() {
        let mut state = ScrollRegionState {
            offset: 40,
            content_height: 100,
            viewport_height: 20,
            follow_live: false,
            unseen: 3,
        };
        state.reconcile(60, 30);
        assert_eq!(state.offset, 30);
        assert!(!state.follow_live);
        assert_eq!(state.unseen, 3);

        state.follow_live = true;
        state.reconcile(120, 20);
        assert_eq!(state.offset, 100);
        assert_eq!(state.thumb(), Some(17..20));
    }
}
