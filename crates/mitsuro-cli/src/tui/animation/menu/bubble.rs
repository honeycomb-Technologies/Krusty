//! Bubble animator - renders floating bubbles

use super::bubble_field::BubbleField;
use ratatui::style::Color;
use std::time::Instant;

pub struct BubbleAnimator {
    bubble_field: BubbleField,
    last_update: Instant,
}

impl BubbleAnimator {
    pub fn new() -> Self {
        Self {
            bubble_field: BubbleField::new(80, 24),
            last_update: Instant::now(),
        }
    }

    pub fn set_theme_color(&mut self, accent_rgb: (u8, u8, u8)) {
        self.bubble_field.set_theme_color(accent_rgb);
    }

    pub fn update(&mut self, area_width: u16, area_height: u16, crab_x: Option<f32>) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_update);
        self.last_update = now;

        self.bubble_field.resize(area_width, area_height);
        self.bubble_field.update(dt, crab_x);
    }

    pub fn render_bubbles(
        &self,
        area_width: u16,
        area_height: u16,
    ) -> Vec<(u16, u16, char, Color)> {
        let mut result = Vec::new();

        for bubble in self.bubble_field.get_bubbles() {
            // Convert float coordinates to screen coordinates
            let x = bubble.x.round() as u16;
            let y = bubble.y.round() as u16;

            // Only render if within bounds
            if x < area_width && y < area_height {
                let ch = if bubble.popped {
                    // Pop animation characters
                    if bubble.pop_animation_progress < 0.3 {
                        '\u{25CC}' // ◌
                    } else if bubble.pop_animation_progress < 0.6 {
                        '\u{25E6}' // ◦
                    } else {
                        '\u{00B7}' // ·
                    }
                } else {
                    self.get_bubble_char(bubble.get_visual_radius() as f32)
                };

                // Convert palette color to ratatui color, fade during pop
                let mut color = Color::Rgb(bubble.color.red, bubble.color.green, bubble.color.blue);
                if bubble.popped {
                    // Fade to lighter color during pop
                    let fade = ((1.0 - bubble.pop_animation_progress) * 0.7 + 0.3) as f32;
                    color = Color::Rgb(
                        (bubble.color.red as f32 * fade) as u8,
                        (bubble.color.green as f32 * fade) as u8,
                        (bubble.color.blue as f32 * fade) as u8,
                    );
                }

                result.push((x, y, ch, color));
            }
        }

        result
    }

    fn get_bubble_char(&self, radius: f32) -> char {
        if radius < 1.0 {
            '\u{25CB}' // ○ Small bubble
        } else if radius < 1.5 {
            '\u{25EF}' // ◯ Medium bubble
        } else if radius < 1.8 {
            '\u{25C9}' // ◉ Large bubble
        } else {
            '\u{2B24}' // ⬤ Extra large bubble
        }
    }
}

impl Default for BubbleAnimator {
    fn default() -> Self {
        Self::new()
    }
}
