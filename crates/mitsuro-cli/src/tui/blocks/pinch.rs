//! In-place compaction status block with crab pincer animation.

use std::time::{Duration, Instant};

use crossterm::event::Event;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use super::{ClipContext, EventResult, StreamBlock};
use crate::tui::themes::Theme;

const FRAME_MS: Duration = Duration::from_millis(140);
const PAUSE_MS: Duration = Duration::from_millis(360);
const PINCER_OPEN: &str = "(\\/)";
const PINCER_CLOSED: &str = "(||)";

/// Animated compaction indicator: `Pinching` with alternating crab pincers.
pub struct PinchBlock {
    streaming: bool,
    success: Option<bool>,
    open_pincer: bool,
    toggles_remaining: u8,
    paused: bool,
    next_burst_toggles: u8,
    last_tick: Instant,
}

impl PinchBlock {
    pub fn new() -> Self {
        Self {
            streaming: true,
            success: None,
            open_pincer: true,
            toggles_remaining: 2,
            paused: false,
            next_burst_toggles: 3,
            last_tick: Instant::now(),
        }
    }

    pub fn complete(&mut self, success: bool) {
        self.streaming = false;
        self.success = Some(success);
    }

    fn pincer_glyph(&self) -> &'static str {
        if self.streaming {
            if self.open_pincer {
                PINCER_OPEN
            } else {
                PINCER_CLOSED
            }
        } else if self.success == Some(true) {
            "✓"
        } else {
            "✗"
        }
    }

    fn advance_animation(&mut self) {
        if !self.streaming {
            return;
        }

        let wait = if self.paused { PAUSE_MS } else { FRAME_MS };
        if self.last_tick.elapsed() < wait {
            return;
        }
        self.last_tick = Instant::now();

        if self.paused {
            self.paused = false;
            self.toggles_remaining = self.next_burst_toggles;
            self.next_burst_toggles = if self.next_burst_toggles == 2 { 3 } else { 2 };
            self.open_pincer = true;
            return;
        }

        self.open_pincer = !self.open_pincer;
        if self.toggles_remaining > 0 {
            self.toggles_remaining -= 1;
        }
        if self.toggles_remaining == 0 {
            self.paused = true;
        }
    }
}

impl Default for PinchBlock {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamBlock for PinchBlock {
    fn height(&self, _width: u16, _theme: &Theme) -> u16 {
        1
    }

    fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        _focused: bool,
        _clip: Option<ClipContext>,
    ) {
        if area.height == 0 || area.width < 8 {
            return;
        }

        let label = if self.streaming {
            "Pinching"
        } else if self.success == Some(true) {
            "Pinched"
        } else {
            "Pinch failed"
        };

        let glyph = self.pincer_glyph();
        let text = format!("{glyph} {label}");

        let style = if self.success == Some(true) {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD)
        } else if self.success == Some(false) {
            Style::default()
                .fg(theme.error_color)
                .add_modifier(Modifier::ITALIC)
        } else {
            Style::default()
                .fg(theme.text_color)
                .add_modifier(Modifier::ITALIC)
        };

        buf.set_string(area.x, area.y, &text, style);

        if self.streaming {
            let pincer_style = Style::default()
                .fg(theme.accent_color)
                .add_modifier(Modifier::BOLD);
            buf.set_string(area.x, area.y, glyph, pincer_style);
        }
    }

    fn handle_event(
        &mut self,
        _event: &Event,
        _area: Rect,
        _clip: Option<ClipContext>,
    ) -> EventResult {
        EventResult::Ignored
    }

    fn tick(&mut self) -> bool {
        if !self.streaming {
            return false;
        }
        self.advance_animation();
        true
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }
}
