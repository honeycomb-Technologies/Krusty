//! Bubble field manager - spawns and manages multiple bubbles

use super::bubble_types::{Bubble, ColorScheme};
use rand::Rng;
use std::time::Duration;

pub struct BubbleField {
    bubbles: Vec<Bubble>,
    width: u16,
    height: u16,
    spawn_timer: f32,
    color_scheme: ColorScheme,
}

impl BubbleField {
    pub fn new(width: u16, height: u16) -> Self {
        let mut field = Self {
            bubbles: Vec::new(),
            width,
            height,
            spawn_timer: 0.0,
            color_scheme: ColorScheme::Ocean,
        };

        // Start with some bubbles
        for _ in 0..8 {
            field.spawn_bubble(None);
        }

        field
    }

    pub fn set_theme_color(&mut self, accent_rgb: (u8, u8, u8)) {
        // Only update if the color actually changed
        match &self.color_scheme {
            ColorScheme::Theme {
                accent_rgb: current,
            } if *current == accent_rgb => return,
            _ => {}
        }

        self.color_scheme = ColorScheme::Theme { accent_rgb };
        // Update existing bubbles to gradually use new color instead of resetting
        // New bubbles will use the new color scheme
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    pub fn update(&mut self, dt: Duration, crab_x: Option<f32>) {
        let dt_secs = dt.as_secs_f32();

        // Update existing bubbles
        self.bubbles.retain_mut(|bubble| {
            bubble.update(dt_secs);

            // Pop bubbles near crab
            if let Some(cx) = crab_x {
                let dx = bubble.x - cx as f64;
                let dy = bubble.y - (self.height as f64 - 5.0); // Crab is near bottom
                let dist = (dx * dx + dy * dy).sqrt();

                if dist < 8.0 && !bubble.popped {
                    bubble.pop();
                }
            }

            // Remove bubbles that are done or out of bounds
            !bubble.is_done() && bubble.y > -5.0
        });

        // Spawn new bubbles
        self.spawn_timer += dt_secs;
        if self.spawn_timer > 0.5 {
            self.spawn_timer = 0.0;

            // Spawn near crab sometimes
            if let Some(cx) = crab_x {
                let mut rng = rand::thread_rng();
                if rng.gen_bool(0.3) {
                    self.spawn_bubble(Some(cx));
                } else {
                    self.spawn_bubble(None);
                }
            } else {
                self.spawn_bubble(None);
            }
        }

        // Keep bubble count reasonable
        while self.bubbles.len() > 30 {
            self.bubbles.remove(0);
        }
    }

    fn spawn_bubble(&mut self, near_x: Option<f32>) {
        let mut rng = rand::thread_rng();

        let x = if let Some(cx) = near_x {
            // Spawn near the crab
            cx as f64 + rng.gen_range(-10.0..10.0)
        } else {
            rng.gen_range(5.0..(self.width as f64 - 5.0))
        };

        let y = self.height as f64 + rng.gen_range(1.0..5.0);

        self.bubbles.push(Bubble::new(x, y, &self.color_scheme));
    }

    pub fn get_bubbles(&self) -> &[Bubble] {
        &self.bubbles
    }
}
