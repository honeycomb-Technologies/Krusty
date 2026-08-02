//! Bubble types and color schemes for the bubble animation

use palette::{Hsv, IntoColor, Srgb};
use rand::Rng;

#[derive(Debug, Clone)]
pub struct Bubble {
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub radius: f64,
    pub max_radius: f64,
    pub growth_rate: f64,
    pub lifetime: f64,
    pub max_lifetime: f64,
    pub wobble_phase: f64,
    pub wobble_speed: f64,
    pub wobble_amplitude: f64,
    pub color: Srgb<u8>,
    pub shimmer_phase: f64,
    pub popped: bool,
    pub pop_animation_progress: f64,
}

impl Bubble {
    pub fn new(x: f64, y: f64, color_scheme: &ColorScheme) -> Self {
        let mut rng = rand::thread_rng();

        let base_color = color_scheme.get_bubble_color();
        let color = Srgb::new(base_color.0, base_color.1, base_color.2);

        Self {
            x,
            y,
            vx: rng.gen_range(-2.0..2.0),
            vy: rng.gen_range(-15.0..-8.0), // Upward velocity
            radius: rng.gen_range(0.3..0.8),
            max_radius: rng.gen_range(1.5..2.5),
            growth_rate: rng.gen_range(0.3..0.8),
            lifetime: 0.0,
            max_lifetime: rng.gen_range(4.0..8.0),
            wobble_phase: rng.gen_range(0.0..std::f64::consts::TAU),
            wobble_speed: rng.gen_range(2.0..4.0),
            wobble_amplitude: rng.gen_range(0.5..1.5),
            color,
            shimmer_phase: rng.gen_range(0.0..std::f64::consts::TAU),
            popped: false,
            pop_animation_progress: 0.0,
        }
    }

    pub fn update(&mut self, dt: f32) {
        if self.popped {
            self.pop_animation_progress += (dt * 3.0) as f64;
            return;
        }

        self.lifetime += dt as f64;

        // Grow bubble
        if self.radius < self.max_radius {
            self.radius += self.growth_rate * dt as f64;
        }

        // Physics
        self.x += self.vx * dt as f64;
        self.y += self.vy * dt as f64;

        // Wobble
        self.wobble_phase += self.wobble_speed * dt as f64;
        let wobble = self.wobble_amplitude * self.wobble_phase.sin();
        self.x += wobble * dt as f64;

        // Shimmer
        self.shimmer_phase += dt as f64 * 3.0;

        // Slow down horizontal movement
        self.vx *= 0.98;

        // Pop if too old
        if self.lifetime > self.max_lifetime {
            self.pop();
        }
    }

    pub fn pop(&mut self) {
        self.popped = true;
        self.pop_animation_progress = 0.0;
    }

    pub fn is_done(&self) -> bool {
        self.popped && self.pop_animation_progress > 1.0
    }

    pub fn get_visual_radius(&self) -> f64 {
        if self.popped {
            self.radius * (1.0 + self.pop_animation_progress * 0.5)
        } else {
            self.radius
        }
    }
}

#[derive(Debug, Clone)]
pub enum ColorScheme {
    Ocean,
    Theme { accent_rgb: (u8, u8, u8) },
}

impl ColorScheme {
    pub fn get_bubble_color(&self) -> (u8, u8, u8) {
        let mut rng = rand::thread_rng();

        match self {
            ColorScheme::Ocean => {
                // Ocean blues and teals
                let hue = rng.gen_range(180.0..220.0);
                let saturation = rng.gen_range(0.4..0.8);
                let value = rng.gen_range(0.6..0.9);

                let hsv = Hsv::new(hue, saturation, value);
                let rgb: Srgb = hsv.into_color();

                (
                    (rgb.red * 255.0) as u8,
                    (rgb.green * 255.0) as u8,
                    (rgb.blue * 255.0) as u8,
                )
            }
            ColorScheme::Theme { accent_rgb } => {
                // Use theme accent color with some variation
                let base_rgb = Srgb::new(
                    accent_rgb.0 as f32 / 255.0,
                    accent_rgb.1 as f32 / 255.0,
                    accent_rgb.2 as f32 / 255.0,
                );

                // Convert to HSV for manipulation
                let base_hsv: Hsv = base_rgb.into_color();

                // Add some variation to the theme color
                let hue_variation = rng.gen_range(-15.0..15.0);
                let sat_variation = rng.gen_range(-0.1..0.1);
                let val_variation = rng.gen_range(-0.1..0.2);

                let hue = (base_hsv.hue.into_positive_degrees() + hue_variation) % 360.0;
                let saturation = (base_hsv.saturation + sat_variation).clamp(0.3, 1.0);
                let value = (base_hsv.value + val_variation).clamp(0.4, 1.0);

                let hsv = Hsv::new(hue, saturation, value);
                let rgb: Srgb = hsv.into_color();

                (
                    (rgb.red * 255.0) as u8,
                    (rgb.green * 255.0) as u8,
                    (rgb.blue * 255.0) as u8,
                )
            }
        }
    }
}
