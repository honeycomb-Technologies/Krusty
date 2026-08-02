//! Menu animations - crab and bubbles for the start screen

pub mod bubble;
pub mod bubble_field;
pub mod bubble_types;
pub mod crab;

pub use bubble::BubbleAnimator;
pub use crab::CrabAnimator;

use ratatui::style::Color;
use std::time::{Duration, Instant};

/// Coordinates crab and bubble animations for the start menu
pub struct MenuAnimator {
    bubble_animator: BubbleAnimator,
    crab_animator: CrabAnimator,
    last_update: Instant,
}

impl MenuAnimator {
    pub fn new() -> Self {
        Self {
            bubble_animator: BubbleAnimator::new(),
            crab_animator: CrabAnimator::new(5.0, 10.0), // Start at left side
            last_update: Instant::now(),
        }
    }

    pub fn update(&mut self, area_width: u16, area_height: u16, _dt: Duration) {
        // Calculate actual elapsed time since last update
        let now = Instant::now();
        let actual_dt = now.duration_since(self.last_update);
        self.last_update = now;

        // Update crab first with actual elapsed time
        self.crab_animator.update(actual_dt, area_width);

        // Update bubbles with crab position
        self.bubble_animator
            .update(area_width, area_height, Some(self.crab_animator.get_x()));
    }

    pub fn render_crab(&self) -> (Vec<String>, f32, f32) {
        let frames = self.crab_animator.render();
        (frames, self.crab_animator.x, self.crab_animator.y)
    }

    pub fn render_bubbles(
        &self,
        area_width: u16,
        area_height: u16,
    ) -> Vec<(u16, u16, char, Color)> {
        self.bubble_animator.render_bubbles(area_width, area_height)
    }

    pub fn set_theme_color(&mut self, accent_rgb: (u8, u8, u8)) {
        self.bubble_animator.set_theme_color(accent_rgb);
    }
}

impl Default for MenuAnimator {
    fn default() -> Self {
        Self::new()
    }
}
