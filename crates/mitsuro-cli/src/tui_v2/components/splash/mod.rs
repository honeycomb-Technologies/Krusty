//! Home load: box-drawing mitsuro with a linear stroke-in.

mod mark;
mod scenes;

use ratatui::{layout::Rect, Frame};

use crate::tui_v2::{
    model::capability::GlyphMode, motion::preference::MotionPreference,
    presentation::theme::SemanticTheme,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SplashState {
    pub settled: bool,
    pub scene_origin_ms: u64,
}

impl SplashState {
    pub fn restart_at(&mut self, now_ms: u64) {
        self.scene_origin_ms = now_ms;
        self.settled = false;
    }

    pub fn local_ms(self, wall_ms: u64) -> u64 {
        wall_ms.saturating_sub(self.scene_origin_ms)
    }

    pub fn mark_settled_if_ready(&mut self, wall_ms: u64, preference: MotionPreference) {
        let local = self.local_ms(wall_ms);
        // Stroke finishes at TRACE_MS (1.1s); mark settled shortly after.
        if !matches!(preference, MotionPreference::Full) || local >= 1_200 {
            self.settled = true;
        }
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    splash: SplashState,
    wall_ms: u64,
    preference: MotionPreference,
    glyph_mode: GlyphMode,
    theme: SemanticTheme,
) {
    if area.is_empty() {
        return;
    }

    let elapsed_ms = splash.local_ms(wall_ms);
    let complete =
        splash.settled || !matches!(preference, MotionPreference::Full) || elapsed_ms >= 1_200;

    scenes::render(frame, area, elapsed_ms, complete, glyph_mode, theme);
}
