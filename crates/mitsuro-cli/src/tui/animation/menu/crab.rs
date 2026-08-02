//! Crab animator - the walking crab mascot

use rand::Rng;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum CrabState {
    Walking { distance_remaining: f32 },
    LookingAround { phase: usize, last_update: Instant },
    Pinching { phase: usize, last_update: Instant },
}

#[derive(Debug, Clone)]
pub struct CrabAnimator {
    pub x: f32,
    pub y: f32,
    state: CrabState,
    direction: f32, // -1.0 for left, 1.0 for right
    speed: f32,
    walk_frame: usize,
    last_walk_update: Instant,
}

impl CrabAnimator {
    pub fn new(x: f32, y: f32) -> Self {
        let mut rng = rand::thread_rng();
        let initial_distance = rng.gen_range(30.0..80.0);

        Self {
            x,
            y,
            state: CrabState::Walking {
                distance_remaining: initial_distance,
            },
            direction: if rng.gen_bool(0.5) { 1.0 } else { -1.0 },
            speed: 50.0, // pixels per second for smooth movement
            walk_frame: 0,
            last_walk_update: Instant::now(),
        }
    }

    pub fn update(&mut self, dt: Duration, area_width: u16) {
        let dt_secs = dt.as_secs_f32();

        match &mut self.state {
            CrabState::Walking { distance_remaining } => {
                // Update position
                let delta = self.speed * dt_secs * self.direction;
                let abs_delta = delta.abs();

                // Move the crab
                self.x += delta;
                *distance_remaining -= abs_delta;

                // Bounce off edges - crab is 25 chars wide (widest line)
                const CRAB_WIDTH: f32 = 25.0;
                if self.x <= 0.0 {
                    self.x = 0.0;
                    self.direction = 1.0;
                    *distance_remaining = 0.0; // Force state change
                } else if self.x >= area_width as f32 - CRAB_WIDTH {
                    self.x = area_width as f32 - CRAB_WIDTH;
                    self.direction = -1.0;
                    *distance_remaining = 0.0; // Force state change
                }

                // Update walk animation smoothly - slower frame rate for visible leg movement
                if self.last_walk_update.elapsed() >= Duration::from_millis(200) {
                    self.walk_frame = (self.walk_frame + 1) % 4;
                    self.last_walk_update = Instant::now();
                }

                // Check if we should change state
                if *distance_remaining <= 0.0 {
                    self.decide_next_action();
                }
            }
            CrabState::LookingAround { phase, last_update } => {
                // Looking sequence: left, right, left, blink, open, blink, open
                let phase_duration = match phase {
                    0..=2 => Duration::from_millis(300), // Eye movements (slower)
                    3 => Duration::from_millis(200),     // First blink (closed)
                    4 => Duration::from_millis(150),     // Open eyes
                    5 => Duration::from_millis(200),     // Second blink (closed)
                    6 => Duration::from_millis(150),     // Open eyes
                    _ => Duration::from_millis(300),
                };

                if last_update.elapsed() >= phase_duration {
                    *phase += 1;
                    *last_update = Instant::now();

                    if *phase >= 7 {
                        // Done looking, start walking again
                        self.start_walking();
                    }
                }
            }
            CrabState::Pinching { phase, last_update } => {
                // Pinching sequence: close, open, close, open
                if last_update.elapsed() >= Duration::from_millis(150) {
                    *phase += 1;
                    *last_update = Instant::now();

                    if *phase >= 4 {
                        // Done pinching, decide what to do next
                        let mut rng = rand::thread_rng();
                        if rng.gen_bool(0.3) {
                            // Sometimes look around after pinching
                            self.state = CrabState::LookingAround {
                                phase: 0,
                                last_update: Instant::now(),
                            };
                        } else {
                            // Otherwise start walking
                            self.start_walking();
                        }
                    }
                }
            }
        }
    }

    fn decide_next_action(&mut self) {
        let mut rng = rand::thread_rng();
        let action_roll = rng.gen_range(0..100);

        if action_roll < 40 {
            // 40% chance to just keep walking
            self.start_walking();
        } else if action_roll < 70 {
            // 30% chance to look around
            self.state = CrabState::LookingAround {
                phase: 0,
                last_update: Instant::now(),
            };
        } else {
            // 30% chance to pinch
            self.state = CrabState::Pinching {
                phase: 0,
                last_update: Instant::now(),
            };
        }
    }

    fn start_walking(&mut self) {
        let mut rng = rand::thread_rng();

        // Random distance to walk
        let distance = rng.gen_range(20.0..100.0);

        // Random chance to change direction
        if rng.gen_bool(0.4) {
            self.direction *= -1.0;
        }

        self.state = CrabState::Walking {
            distance_remaining: distance,
        };
    }

    pub fn render(&self) -> Vec<String> {
        match &self.state {
            CrabState::Walking { .. } => self.render_walking(),
            CrabState::LookingAround { phase, .. } => self.render_looking(*phase),
            CrabState::Pinching { phase, .. } => self.render_pinching(*phase),
        }
    }

    fn render_walking(&self) -> Vec<String> {
        // Smooth walking animation combining /\ and || patterns
        let legs = match self.walk_frame {
            0 => r"/\ /\ /\ /\",
            1 => r"|| || || ||",
            2 => r"/\ /\ /\ /\",
            3 => r"|| || || ||",
            _ => r"/\ /\ /\ /\",
        };

        vec![
            "         ^     ^".to_string(),
            r" (\/)    o     o    (\/)".to_string(),
            r"   \______\___/______/".to_string(),
            "      (___________)".to_string(),
            format!("       {}", legs),
        ]
    }

    fn render_looking(&self, phase: usize) -> Vec<String> {
        // Build the complete eye line to match the standing/walking format exactly
        // Both eyes move together as a pair, maintaining 5 spaces between them
        let (eyebrow_line, eye_line) = match phase {
            0 => (
                "        ^     ^".to_string(),
                r" (\/)   o     o     (\/)".to_string(),
            ), // Look left
            1 => (
                "          ^     ^".to_string(),
                r" (\/)     o     o   (\/)".to_string(),
            ), // Look right
            2 => (
                "        ^     ^".to_string(),
                r" (\/)   o     o     (\/)".to_string(),
            ), // Look left again
            3 => (
                "         ^     ^".to_string(),
                r" (\/)    -     -    (\/)".to_string(),
            ), // First blink
            4 => (
                "         ^     ^".to_string(),
                r" (\/)    o     o    (\/)".to_string(),
            ), // Open
            5 => (
                "         ^     ^".to_string(),
                r" (\/)    -     -    (\/)".to_string(),
            ), // Second blink
            6 => (
                "         ^     ^".to_string(),
                r" (\/)    o     o    (\/)".to_string(),
            ), // Open
            _ => (
                "         ^     ^".to_string(),
                r" (\/)    o     o    (\/)".to_string(),
            ), // Normal
        };

        vec![
            eyebrow_line,
            eye_line,
            r"   \______\___/______/".to_string(),
            "      (___________)".to_string(),
            r"       /\ /\ /\ /\".to_string(),
        ]
    }

    fn render_pinching(&self, phase: usize) -> Vec<String> {
        let (left_pincer, right_pincer) = match phase {
            0 | 2 => ("(||)", "(||)"),   // Closed
            1 | 3 => (r"(\/)", r"(\/)"), // Open
            _ => (r"(\/)", r"(\/)"),
        };

        vec![
            "         ^     ^".to_string(),
            format!(" {}    o     o    {}", left_pincer, right_pincer),
            r"   \______\___/______/".to_string(),
            "      (___________)".to_string(),
            r"       /\ /\ /\ /\".to_string(),
        ]
    }

    pub fn get_x(&self) -> f32 {
        self.x
    }
}
