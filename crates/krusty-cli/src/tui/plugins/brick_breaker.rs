//! Brick Breaker Game Plugin
//!
//! A native Rust implementation of classic brick breaker game.
//! Features:
//! - Pixel-perfect rendering via Kitty graphics protocol
//! - Smooth 60fps gameplay
//! - Multiple levels with increasing difficulty
//! - Power-ups: expand paddle, multi-ball, slow ball, laser
//! - Particle effects and visual polish

use std::any::Any;
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer, layout::Rect, style::Color, style::Style, text::Line, text::Span,
    widgets::Paragraph, widgets::Widget,
};

use super::{Plugin, PluginContext, PluginEventResult, PluginFrame, PluginRenderMode};

// ============================================================================
// CONSTANTS & CONFIGURATION
// ============================================================================

/// Internal game resolution (scaled to window size)
pub const GAME_WIDTH: u32 = 640;
pub const GAME_HEIGHT: u32 = 480;

/// Paddle configuration
const PADDLE_WIDTH: f32 = 100.0;
const PADDLE_HEIGHT: f32 = 15.0;
const PADDLE_SPEED: f32 = 800.0;
const PADDLE_Y: f32 = 420.0;

/// Ball configuration
const BALL_RADIUS: f32 = 8.0;
const BALL_BASE_SPEED: f32 = 600.0;
const BALL_MAX_SPEED: f32 = 1000.0;

/// Brick configuration
const BRICK_ROWS: usize = 8;
const BRICK_COLS: usize = 10;
const BRICK_WIDTH: f32 = 58.0;
const BRICK_HEIGHT: f32 = 20.0;
const BRICK_PADDING: f32 = 4.0;
const BRICK_TOP_OFFSET: f32 = 40.0;
const BRICK_LEFT_OFFSET: f32 = 27.0;

/// Game configuration
const INITIAL_LIVES: u8 = 3;
const MAX_PARTICLES: usize = 100;

// ============================================================================
// COLOR PALETTE
// ============================================================================

/// Row-based brick colors (from top to bottom)
const BRICK_COLORS: &[u32] = &[
    0xFF0000, // Red
    0xFF7F00, // Orange
    0xFFFF00, // Yellow
    0x00FF00, // Green
    0x0000FF, // Blue
    0x4B0082, // Indigo
    0x9400D3, // Violet
    0xFF1493, // Deep Pink
];

// ============================================================================
// GAME STATE
// ============================================================================

/// Main game states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    Menu,
    Playing,
    Paused,
    GameOver,
    LevelComplete,
}

/// Power-up types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerUpType {
    ExpandPaddle,
    MultiBall,
    SlowBall,
    Laser,
}

/// Brick types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrickType {
    Normal,
    TwoHit,
    ThreeHit,
    Indestructible,
}

/// Individual brick
#[derive(Debug, Clone)]
struct Brick {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    brick_type: BrickType,
    hits_remaining: u8,
    color: u32,
    alive: bool,
}

impl Brick {
    fn new(col: usize, row: usize, brick_type: BrickType) -> Self {
        let x = BRICK_LEFT_OFFSET + col as f32 * (BRICK_WIDTH + BRICK_PADDING);
        let y = BRICK_TOP_OFFSET + row as f32 * (BRICK_HEIGHT + BRICK_PADDING);
        let color = BRICK_COLORS[row % BRICK_COLORS.len()];

        let hits_remaining = match brick_type {
            BrickType::Normal => 1,
            BrickType::TwoHit => 2,
            BrickType::ThreeHit => 3,
            BrickType::Indestructible => 255,
        };

        Self {
            x,
            y,
            width: BRICK_WIDTH,
            height: BRICK_HEIGHT,
            brick_type,
            hits_remaining,
            color,
            alive: true,
        }
    }

    fn get_color_with_damage(&self) -> u32 {
        if !self.alive {
            return 0x000000;
        }
        match self.brick_type {
            BrickType::Indestructible => 0x808080, // Gray
            _ => {
                // Darken color based on hits remaining
                let base = self.color;
                let factor = self.hits_remaining as f32 / 3.0;
                let r = ((base >> 16) & 0xFF) as f32 * factor;
                let g = ((base >> 8) & 0xFF) as f32 * factor;
                let b = (base & 0xFF) as f32 * factor;
                ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
            }
        }
    }
}

/// Paddle state
#[derive(Debug, Clone)]
struct Paddle {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    velocity: f32,
    base_width: f32,
    expand_timer: f32, // Seconds remaining for expanded paddle
}

impl Paddle {
    fn new() -> Self {
        Self {
            x: GAME_WIDTH as f32 / 2.0 - PADDLE_WIDTH / 2.0,
            y: PADDLE_Y,
            width: PADDLE_WIDTH,
            height: PADDLE_HEIGHT,
            velocity: 0.0,
            base_width: PADDLE_WIDTH,
            expand_timer: 0.0,
        }
    }

    fn update(&mut self, dt: f32, keys: &KeyState) {
        // Calculate velocity based on input (both keys cancel out)
        let mut target_velocity = 0.0;
        if keys.left {
            target_velocity -= PADDLE_SPEED;
        }
        if keys.right {
            target_velocity += PADDLE_SPEED;
        }

        // Smooth movement
        self.velocity = target_velocity;
        self.x += self.velocity * dt;

        // Clamp to screen bounds
        self.x = self.x.clamp(0.0, GAME_WIDTH as f32 - self.width);

        // Update expanded paddle timer
        if self.expand_timer > 0.0 {
            self.expand_timer -= dt;
            if self.expand_timer <= 0.0 {
                self.width = self.base_width;
            }
        }
    }

    fn expand(&mut self, duration: f32) {
        self.width = self.base_width * 1.5;
        self.expand_timer = duration;
    }

    fn get_rect(&self) -> (f32, f32, f32, f32) {
        (self.x, self.y, self.width, self.height)
    }
}

/// Ball state
#[derive(Debug, Clone)]
struct Ball {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    radius: f32,
    alive: bool,
    trail: Vec<(f32, f32)>, // Trail positions for visual effect
}

impl Ball {
    fn new() -> Self {
        Self {
            x: GAME_WIDTH as f32 / 2.0,
            y: PADDLE_Y - BALL_RADIUS - 5.0,
            vx: BALL_BASE_SPEED,
            vy: -BALL_BASE_SPEED,
            radius: BALL_RADIUS,
            alive: true,
            trail: Vec::with_capacity(10),
        }
    }

    fn update(&mut self, dt: f32) {
        if !self.alive {
            return;
        }

        // Store trail
        self.trail.push((self.x, self.y));
        if self.trail.len() > 10 {
            self.trail.remove(0);
        }

        // Move
        self.x += self.vx * dt;
        self.y += self.vy * dt;

        // Clamp speed
        let speed = (self.vx * self.vx + self.vy * self.vy).sqrt();
        if speed > BALL_MAX_SPEED {
            let scale = BALL_MAX_SPEED / speed;
            self.vx *= scale;
            self.vy *= scale;
        }
    }

    fn slow_down(&mut self) {
        let speed = (self.vx * self.vx + self.vy * self.vy).sqrt();
        if speed > BALL_BASE_SPEED * 0.6 {
            let scale = (BALL_BASE_SPEED * 0.6) / speed;
            self.vx *= scale;
            self.vy *= scale;
        }
    }
}

/// Power-up state
#[derive(Debug, Clone)]
struct PowerUp {
    x: f32,
    y: f32,
    vy: f32,
    power_type: PowerUpType,
    alive: bool,
}

impl PowerUp {
    fn new(x: f32, y: f32, power_type: PowerUpType) -> Self {
        Self {
            x,
            y,
            vy: 200.0, // Fall speed
            power_type,
            alive: true,
        }
    }

    fn update(&mut self, dt: f32) {
        self.y += self.vy * dt;
        if self.y > GAME_HEIGHT as f32 {
            self.alive = false;
        }
    }

    fn get_color(&self) -> u32 {
        match self.power_type {
            PowerUpType::ExpandPaddle => 0x00FF00, // Green
            PowerUpType::MultiBall => 0xFF00FF,    // Magenta
            PowerUpType::SlowBall => 0x00FFFF,     // Cyan
            PowerUpType::Laser => 0xFF4500,        // Orange Red
        }
    }
}

/// Particle for effects
#[derive(Debug, Clone)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    max_life: f32,
    color: u32,
    size: f32,
}

impl Particle {
    fn new(x: f32, y: f32, color: u32) -> Self {
        let angle = rand::random::<f32>() * std::f32::consts::PI * 2.0;
        let speed = rand::random::<f32>() * 200.0 + 100.0;
        Self {
            x,
            y,
            vx: angle.cos() * speed,
            vy: angle.sin() * speed,
            life: rand::random::<f32>() * 0.3 + 0.15,
            max_life: 0.45,
            color,
            size: rand::random::<f32>() * 3.0 + 2.0,
        }
    }

    fn update(&mut self, dt: f32) {
        self.x += self.vx * dt;
        self.y += self.vy * dt;
        self.vy += 400.0 * dt; // Gravity
        self.life -= dt;
    }
}

/// Keyboard state tracking
#[derive(Debug, Clone, Default)]
struct KeyState {
    left: bool,
    right: bool,
}

// ============================================================================
// MAIN PLUGIN STRUCT
// ============================================================================

pub struct BrickBreakerPlugin {
    // Game state
    state: GameState,

    // Game objects
    paddle: Paddle,
    balls: Vec<Ball>,
    bricks: Vec<Brick>,
    powerups: Vec<PowerUp>,
    particles: Vec<Particle>,

    // Score and lives
    score: u32,
    lives: u8,
    level: u8,
    high_score: u32,

    // Input state
    keys: KeyState,

    // Rendering - Arc for zero-copy sharing with graphics system
    frame_buffer: Arc<Vec<u8>>,
    /// Scratch buffer for rendering (avoids allocation per frame)
    scratch_buffer: Vec<u8>,
    frame_ready: bool,

    // Level data
    current_level_layout: Vec<Vec<BrickType>>,
}

impl BrickBreakerPlugin {
    pub fn new() -> Self {
        let size = (GAME_WIDTH * GAME_HEIGHT * 4) as usize;
        Self {
            state: GameState::Menu,
            paddle: Paddle::new(),
            balls: vec![Ball::new()],
            bricks: Vec::new(),
            powerups: Vec::new(),
            particles: Vec::new(),
            score: 0,
            lives: INITIAL_LIVES,
            level: 1,
            high_score: 0,
            keys: KeyState::default(),
            frame_buffer: Arc::new(Vec::new()),
            scratch_buffer: Vec::with_capacity(size),
            frame_ready: false,
            current_level_layout: Self::generate_level_layout(1),
        }
    }

    fn generate_level_layout(level: u8) -> Vec<Vec<BrickType>> {
        let mut layout = Vec::with_capacity(BRICK_ROWS);

        for row in 0..BRICK_ROWS {
            let mut row_layout = Vec::with_capacity(BRICK_COLS);
            for col in 0..BRICK_COLS {
                let brick_type = match level {
                    1..=2 => BrickType::Normal,
                    3..=4 => {
                        if (row + col) % 3 == 0 {
                            BrickType::TwoHit
                        } else {
                            BrickType::Normal
                        }
                    }
                    5..=6 => match (row + col) % 4 {
                        0 => BrickType::ThreeHit,
                        1 | 2 => BrickType::TwoHit,
                        _ => BrickType::Normal,
                    },
                    7..=8 => {
                        if row == 0 && col % 3 == 0 {
                            BrickType::Indestructible
                        } else {
                            match (row + col) % 3 {
                                0 => BrickType::TwoHit,
                                _ => BrickType::Normal,
                            }
                        }
                    }
                    _ => {
                        // Levels 9+: More indestructible and varied bricks
                        if (row == 0 || row == 1) && col % 2 == 0 {
                            BrickType::Indestructible
                        } else {
                            match (row + col) % 4 {
                                0 => BrickType::ThreeHit,
                                1 | 2 => BrickType::TwoHit,
                                _ => BrickType::Normal,
                            }
                        }
                    }
                };
                row_layout.push(brick_type);
            }
            layout.push(row_layout);
        }

        layout
    }

    fn load_level(&mut self) {
        self.bricks.clear();
        self.powerups.clear();
        self.particles.clear();
        self.balls = vec![Ball::new()];
        self.paddle = Paddle::new();
        self.current_level_layout = Self::generate_level_layout(self.level);

        for row in 0..BRICK_ROWS {
            for col in 0..BRICK_COLS {
                let brick_type = self.current_level_layout[row][col];
                self.bricks.push(Brick::new(col, row, brick_type));
            }
        }
    }

    fn reset_game(&mut self) {
        self.score = 0;
        self.lives = INITIAL_LIVES;
        self.level = 1;
        self.load_level();
        self.state = GameState::Playing;
    }

    fn update(&mut self, dt: f32) {
        match self.state {
            GameState::Playing => {
                // Update paddle
                self.paddle.update(dt, &self.keys);

                // Update balls
                for ball in &mut self.balls {
                    ball.update(dt);
                }

                // Update power-ups
                for powerup in &mut self.powerups {
                    powerup.update(dt);
                }
                self.powerups.retain(|p| p.alive);

                // Update particles
                for particle in &mut self.particles {
                    particle.update(dt);
                }
                self.particles.retain(|p| p.life > 0.0);
                if self.particles.len() > MAX_PARTICLES {
                    self.particles.truncate(MAX_PARTICLES);
                }

                // Collision detection
                self.check_ball_wall_collisions();
                self.check_ball_paddle_collisions();
                self.check_ball_brick_collisions();
                self.check_powerup_paddle_collisions();

                // Check ball deaths
                self.balls.retain(|b| b.alive);
                if self.balls.is_empty() {
                    self.lives = self.lives.saturating_sub(1);
                    if self.lives == 0 {
                        self.state = GameState::GameOver;
                        if self.score > self.high_score {
                            self.high_score = self.score;
                        }
                    } else {
                        self.balls.push(Ball::new());
                    }
                }

                // Check level complete
                if self
                    .bricks
                    .iter()
                    .all(|b| !b.alive || b.brick_type == BrickType::Indestructible)
                {
                    self.state = GameState::LevelComplete;
                }

                self.frame_ready = true;
            }
            GameState::Menu
            | GameState::Paused
            | GameState::GameOver
            | GameState::LevelComplete => {
                // No updates in these states
            }
        }
    }

    fn check_ball_wall_collisions(&mut self) {
        for ball in &mut self.balls {
            if !ball.alive {
                continue;
            }

            // Left wall
            if ball.x - ball.radius < 0.0 {
                ball.x = ball.radius;
                ball.vx = ball.vx.abs();
            }
            // Right wall
            if ball.x + ball.radius > GAME_WIDTH as f32 {
                ball.x = GAME_WIDTH as f32 - ball.radius;
                ball.vx = -ball.vx.abs();
            }
            // Top wall
            if ball.y - ball.radius < 0.0 {
                ball.y = ball.radius;
                ball.vy = ball.vy.abs();
            }
            // Bottom (death)
            if ball.y - ball.radius > GAME_HEIGHT as f32 {
                ball.alive = false;
            }
        }
    }

    fn check_ball_paddle_collisions(&mut self) {
        let (px, py, pw, ph) = self.paddle.get_rect();

        for ball in &mut self.balls {
            if !ball.alive {
                continue;
            }

            // Check if ball is within paddle bounds
            if ball.x + ball.radius > px
                && ball.x - ball.radius < px + pw
                && ball.y + ball.radius > py
                && ball.y - ball.radius < py + ph
            {
                // Only bounce if moving downward
                if ball.vy > 0.0 {
                    ball.y = py - ball.radius;

                    // Calculate bounce angle based on hit position
                    let hit_pos = (ball.x - px) / pw; // 0.0 (left) to 1.0 (right)
                    let angle = (hit_pos - 0.5) * std::f32::consts::PI * 0.7; // Max 63 degrees

                    let speed = (ball.vx * ball.vx + ball.vy * ball.vy).sqrt();
                    ball.vx = angle.sin() * speed;
                    ball.vy = -angle.cos().abs() * speed;

                    // Spawn hit particles
                    for _ in 0..5 {
                        self.particles.push(Particle::new(ball.x, ball.y, 0xFFFFFF));
                    }
                }
            }
        }
    }

    fn check_ball_brick_collisions(&mut self) {
        for ball in &mut self.balls {
            if !ball.alive {
                continue;
            }

            for brick in &mut self.bricks {
                if !brick.alive {
                    continue;
                }

                // AABB collision detection
                if ball.x + ball.radius > brick.x
                    && ball.x - ball.radius < brick.x + brick.width
                    && ball.y + ball.radius > brick.y
                    && ball.y - ball.radius < brick.y + brick.height
                {
                    // Determine collision side
                    let overlap_left = (ball.x + ball.radius) - brick.x;
                    let overlap_right = (brick.x + brick.width) - (ball.x - ball.radius);
                    let overlap_top = (ball.y + ball.radius) - brick.y;
                    let overlap_bottom = (brick.y + brick.height) - (ball.y - ball.radius);

                    let min_overlap_x = overlap_left.min(overlap_right);
                    let min_overlap_y = overlap_top.min(overlap_bottom);

                    if min_overlap_x < min_overlap_y {
                        ball.vx = -ball.vx;
                    } else {
                        ball.vy = -ball.vy;
                    }

                    // Damage brick
                    if brick.brick_type != BrickType::Indestructible {
                        brick.hits_remaining -= 1;
                        if brick.hits_remaining == 0 {
                            brick.alive = false;
                            self.score += 10 * self.level as u32;

                            // Spawn particles
                            for _ in 0..10 {
                                self.particles.push(Particle::new(
                                    brick.x + brick.width / 2.0,
                                    brick.y + brick.height / 2.0,
                                    brick.color,
                                ));
                            }

                            // Maybe drop power-up (10% chance)
                            if rand::random::<f32>() < 0.1 {
                                let power_type = match rand::random::<u8>() % 4 {
                                    0 => PowerUpType::ExpandPaddle,
                                    1 => PowerUpType::MultiBall,
                                    2 => PowerUpType::SlowBall,
                                    _ => PowerUpType::Laser,
                                };
                                self.powerups.push(PowerUp::new(
                                    brick.x + brick.width / 2.0,
                                    brick.y + brick.height / 2.0,
                                    power_type,
                                ));
                            }
                        } else {
                            // Hit but not destroyed
                            for _ in 0..5 {
                                self.particles.push(Particle::new(
                                    brick.x + brick.width / 2.0,
                                    brick.y + brick.height / 2.0,
                                    brick.color,
                                ));
                            }
                        }
                    }
                    break; // Only collide with one brick per frame
                }
            }
        }
    }

    fn check_powerup_paddle_collisions(&mut self) {
        let (px, py, pw, ph) = self.paddle.get_rect();

        for powerup in &mut self.powerups {
            if !powerup.alive {
                continue;
            }

            if powerup.x > px && powerup.x < px + pw && powerup.y > py && powerup.y < py + ph {
                powerup.alive = false;

                match powerup.power_type {
                    PowerUpType::ExpandPaddle => {
                        self.paddle.expand(10.0);
                    }
                    PowerUpType::MultiBall => {
                        let new_balls: Vec<Ball> = self
                            .balls
                            .iter()
                            .map(|b| {
                                let mut new_ball = b.clone();
                                new_ball.vx = -b.vx;
                                new_ball.trail.clear();
                                new_ball
                            })
                            .collect();
                        self.balls.extend(new_balls);
                    }
                    PowerUpType::SlowBall => {
                        for ball in &mut self.balls {
                            ball.slow_down();
                        }
                    }
                    PowerUpType::Laser => {
                        // TODO: Implement laser
                        self.score += 50;
                    }
                }

                // Power-up collect particles
                for _ in 0..15 {
                    self.particles
                        .push(Particle::new(powerup.x, powerup.y, powerup.get_color()));
                }
            }
        }
    }

    fn render_game_frame(&mut self) {
        let width = GAME_WIDTH as usize;
        let height = GAME_HEIGHT as usize;
        let size = width * height * 4;

        // Use scratch buffer for rendering to avoid Arc allocation overhead
        self.scratch_buffer.clear();
        self.scratch_buffer.resize(size, 0);

        // Clear with dark background
        for pixel in self.scratch_buffer.chunks_exact_mut(4) {
            pixel[0] = 20; // R
            pixel[1] = 20; // G
            pixel[2] = 30; // B
            pixel[3] = 255; // A
        }

        // Draw bricks
        for brick in &self.bricks {
            if !brick.alive {
                continue;
            }

            let color = brick.get_color_with_damage();
            let r = ((color >> 16) & 0xFF) as u8;
            let g = ((color >> 8) & 0xFF) as u8;
            let b = (color & 0xFF) as u8;

            let x_start = brick.x as usize;
            let x_end = (brick.x + brick.width) as usize;
            let y_start = brick.y as usize;
            let y_end = (brick.y + brick.height) as usize;

            for y in y_start..y_end.min(height) {
                for x in x_start..x_end.min(width) {
                    let offset = (y * width + x) * 4;
                    self.scratch_buffer[offset] = r;
                    self.scratch_buffer[offset + 1] = g;
                    self.scratch_buffer[offset + 2] = b;
                    self.scratch_buffer[offset + 3] = 255;
                }
            }
        }

        // Draw paddle
        let (px, py, pw, ph) = self.paddle.get_rect();
        let paddle_color = 0x4FC3F7; // Light blue
        let pr = ((paddle_color >> 16) & 0xFF) as u8;
        let pg = ((paddle_color >> 8) & 0xFF) as u8;
        let pb = (paddle_color & 0xFF) as u8;

        for y in py as usize..(py + ph) as usize {
            for x in px as usize..(px + pw) as usize {
                if x >= width || y >= height {
                    continue;
                }
                let offset = (y * width + x) * 4;
                self.scratch_buffer[offset] = pr;
                self.scratch_buffer[offset + 1] = pg;
                self.scratch_buffer[offset + 2] = pb;
                self.scratch_buffer[offset + 3] = 255;
            }
        }

        // Draw balls
        for ball in &self.balls {
            if !ball.alive {
                continue;
            }

            // Draw trail
            for (i, &(tx, ty)) in ball.trail.iter().enumerate() {
                let alpha = ((i as f32 / ball.trail.len() as f32) * 128.0) as u8;
                let tx = tx as usize;
                let ty = ty as usize;
                if tx < width && ty < height {
                    let offset = (ty * width + tx) * 4;
                    self.scratch_buffer[offset] = alpha;
                    self.scratch_buffer[offset + 1] = alpha;
                    self.scratch_buffer[offset + 2] = alpha;
                    self.scratch_buffer[offset + 3] = alpha;
                }
            }

            // Draw ball
            let ball_color = 0xFFFFFF;
            let br = ((ball_color >> 16) & 0xFF) as u8;
            let bg = ((ball_color >> 8) & 0xFF) as u8;
            let bb = (ball_color & 0xFF) as u8;

            let bx_start = (ball.x - ball.radius).max(0.0) as usize;
            let bx_end = (ball.x + ball.radius).min(width as f32) as usize;
            let by_start = (ball.y - ball.radius).max(0.0) as usize;
            let by_end = (ball.y + ball.radius).min(height as f32) as usize;

            for y in by_start..by_end {
                for x in bx_start..bx_end {
                    let dx = x as f32 - ball.x;
                    let dy = y as f32 - ball.y;
                    if dx * dx + dy * dy <= ball.radius * ball.radius {
                        let offset = (y * width + x) * 4;
                        self.scratch_buffer[offset] = br;
                        self.scratch_buffer[offset + 1] = bg;
                        self.scratch_buffer[offset + 2] = bb;
                        self.scratch_buffer[offset + 3] = 255;
                    }
                }
            }
        }

        // Draw power-ups
        for powerup in &self.powerups {
            if !powerup.alive {
                continue;
            }

            let color = powerup.get_color();
            let r = ((color >> 16) & 0xFF) as u8;
            let g = ((color >> 8) & 0xFF) as u8;
            let b = (color & 0xFF) as u8;

            let px = powerup.x as usize;
            let py = powerup.y as usize;
            let size = 12_usize;

            for y in py.saturating_sub(size / 2)..(py + size / 2).min(height) {
                for x in px.saturating_sub(size / 2)..(px + size / 2).min(width) {
                    let offset = (y * width + x) * 4;
                    self.scratch_buffer[offset] = r;
                    self.scratch_buffer[offset + 1] = g;
                    self.scratch_buffer[offset + 2] = b;
                    self.scratch_buffer[offset + 3] = 255;
                }
            }
        }

        // Draw particles
        for particle in &self.particles {
            let color = particle.color;
            let r = ((color >> 16) & 0xFF) as u8;
            let g = ((color >> 8) & 0xFF) as u8;
            let b = (color & 0xFF) as u8;
            let alpha = (particle.life / particle.max_life * 255.0) as u8;

            let px = particle.x as usize;
            let py = particle.y as usize;
            let psize = particle.size as usize;

            for y in py.saturating_sub(psize / 2)..(py + psize / 2).min(height) {
                for x in px.saturating_sub(psize / 2)..(px + psize / 2).min(width) {
                    let offset = (y * width + x) * 4;
                    self.scratch_buffer[offset] = r;
                    self.scratch_buffer[offset + 1] = g;
                    self.scratch_buffer[offset + 2] = b;
                    self.scratch_buffer[offset + 3] = alpha;
                }
            }
        }

        // Swap scratch buffer into frame buffer wrapped in Arc (zero-copy handoff)
        let new_buffer = std::mem::take(&mut self.scratch_buffer);
        self.frame_buffer = Arc::new(new_buffer);
        // Pre-allocate scratch buffer for next frame
        self.scratch_buffer = Vec::with_capacity(size);
    }
}

// ============================================================================
// PLUGIN TRAIT IMPLEMENTATION
// ============================================================================

impl Plugin for BrickBreakerPlugin {
    fn id(&self) -> &str {
        "brick_breaker"
    }

    fn name(&self) -> &str {
        "Brick Breaker"
    }

    fn display_name(&self) -> String {
        format!(
            "Brick Breaker (L{} | Score: {} | Lives: {})",
            self.level, self.score, self.lives
        )
    }

    fn render_mode(&self) -> PluginRenderMode {
        match self.state {
            GameState::Menu
            | GameState::Paused
            | GameState::GameOver
            | GameState::LevelComplete => PluginRenderMode::Text,
            GameState::Playing => PluginRenderMode::KittyGraphics,
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer, _ctx: &PluginContext) {
        match self.state {
            GameState::Menu => {
                let title = Span::styled(
                    "🧱 BRICK BREAKER 🧱",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                );

                let instructions = vec![
                    Line::from(""),
                    Line::from("Controls:"),
                    Line::from("  A/D or ←/→ : Move paddle"),
                    Line::from("  P          : Pause game"),
                    Line::from("  R          : Restart"),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Press SPACE to start",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    )),
                ];

                let paragraph = Paragraph::new(vec![Line::from(title)])
                    .alignment(ratatui::layout::Alignment::Center)
                    .block(
                        ratatui::widgets::Block::default()
                            .borders(ratatui::widgets::Borders::ALL)
                            .border_type(ratatui::widgets::BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Cyan)),
                    );

                paragraph.render(area, buf);

                let instructions_area = Rect {
                    x: area.x,
                    y: area.y + 6,
                    width: area.width,
                    height: area.height.saturating_sub(6),
                };

                let instructions_paragraph =
                    Paragraph::new(instructions).alignment(ratatui::layout::Alignment::Center);

                instructions_paragraph.render(instructions_area, buf);
            }
            GameState::Paused => {
                let text = vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        "⏸️ PAUSED",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from("Press P to resume"),
                ];

                let paragraph = Paragraph::new(text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .block(
                        ratatui::widgets::Block::default()
                            .borders(ratatui::widgets::Borders::ALL)
                            .border_type(ratatui::widgets::BorderType::Rounded),
                    );

                paragraph.render(area, buf);
            }
            GameState::GameOver => {
                let text = vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        "💀 GAME OVER 💀",
                        Style::default()
                            .fg(Color::Red)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(format!("Final Score: {}", self.score)),
                    Line::from(format!("Level Reached: {}", self.level)),
                    Line::from(format!("High Score: {}", self.high_score)),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Press R to restart or Q to quit",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    )),
                ];

                let paragraph = Paragraph::new(text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .block(
                        ratatui::widgets::Block::default()
                            .borders(ratatui::widgets::Borders::ALL)
                            .border_type(ratatui::widgets::BorderType::Rounded),
                    );

                paragraph.render(area, buf);
            }
            GameState::LevelComplete => {
                let text = vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        "🎉 LEVEL COMPLETE! 🎉",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(format!("Level {} cleared!", self.level)),
                    Line::from(format!("Current Score: {}", self.score)),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Press SPACE for next level",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    )),
                ];

                let paragraph = Paragraph::new(text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .block(
                        ratatui::widgets::Block::default()
                            .borders(ratatui::widgets::Borders::ALL)
                            .border_type(ratatui::widgets::BorderType::Rounded),
                    );

                paragraph.render(area, buf);
            }
            GameState::Playing => {
                // KittyGraphics mode - render_game_frame() is used instead
            }
        }
    }

    fn render_frame(&mut self, _width: u32, _height: u32) -> Option<PluginFrame> {
        if self.state != GameState::Playing {
            return None;
        }

        if self.frame_ready {
            self.render_game_frame();
            self.frame_ready = false;

            // Zero-copy: use Arc::clone() instead of Vec::clone()
            Some(PluginFrame::from_arc(
                Arc::clone(&self.frame_buffer),
                GAME_WIDTH,
                GAME_HEIGHT,
            ))
        } else {
            None
        }
    }

    fn handle_event(&mut self, event: &Event, _area: Rect) -> PluginEventResult {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            ..
        }) = event
        {
            let no_modifiers = *modifiers == KeyModifiers::NONE;
            let is_press = *kind == KeyEventKind::Press || *kind == KeyEventKind::Repeat;
            let is_release = *kind == KeyEventKind::Release;

            // Global quit - let parent handle
            if matches!(code, KeyCode::Char('q') | KeyCode::Char('Q')) {
                return PluginEventResult::Ignored;
            }

            // Movement keys - handle press AND release
            if self.state == GameState::Playing {
                match code {
                    KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('A') => {
                        if is_press {
                            self.keys.left = true;
                        } else if is_release {
                            self.keys.left = false;
                        }
                        return PluginEventResult::Consumed;
                    }
                    KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('D') => {
                        if is_press {
                            self.keys.right = true;
                        } else if is_release {
                            self.keys.right = false;
                        }
                        return PluginEventResult::Consumed;
                    }
                    _ => {}
                }
            }

            // Only handle press events for other actions
            if !is_press {
                return PluginEventResult::Ignored;
            }

            match (self.state, code, no_modifiers) {
                // Menu state
                (GameState::Menu, KeyCode::Char(' '), true) => {
                    self.reset_game();
                    return PluginEventResult::Consumed;
                }

                // Playing state - other controls
                (GameState::Playing, KeyCode::Char('p') | KeyCode::Char('P'), true) => {
                    self.state = GameState::Paused;
                    return PluginEventResult::Consumed;
                }
                (GameState::Playing, KeyCode::Char('r') | KeyCode::Char('R'), true) => {
                    self.reset_game();
                    return PluginEventResult::Consumed;
                }

                // Paused state
                (GameState::Paused, KeyCode::Char('p') | KeyCode::Char('P'), true) => {
                    self.state = GameState::Playing;
                    return PluginEventResult::Consumed;
                }

                // Game Over state
                (GameState::GameOver, KeyCode::Char('r') | KeyCode::Char('R'), true) => {
                    self.reset_game();
                    return PluginEventResult::Consumed;
                }

                // Level Complete state
                (GameState::LevelComplete, KeyCode::Char(' '), true) => {
                    self.level += 1;
                    self.load_level();
                    self.state = GameState::Playing;
                    return PluginEventResult::Consumed;
                }

                _ => {}
            }
        }

        PluginEventResult::Ignored
    }

    fn tick(&mut self) -> bool {
        // Fixed time step of ~60fps (16.67ms)
        const DT: f32 = 1.0 / 60.0;
        self.update(DT);
        self.frame_ready
    }

    fn on_activate(&mut self) {
        // Reset input state
        self.keys = KeyState::default();
    }

    fn on_deactivate(&mut self) {
        // Pause game when switching away
        if self.state == GameState::Playing {
            self.state = GameState::Paused;
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Default for BrickBreakerPlugin {
    fn default() -> Self {
        Self::new()
    }
}
