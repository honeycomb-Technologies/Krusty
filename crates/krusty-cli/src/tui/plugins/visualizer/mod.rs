//! XP-Style Audio Visualizer Plugin
//!
//! A Windows XP-inspired audio visualizer with spectrum analyzer display.
//! Features:
//! - Animated spectrum bars with XP color palette (teals, blues)
//! - Track information overlay
//! - Playback controls via mouse scroll and click
//! - Keyboard shortcuts for playback control

use std::any::Any;
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
    widgets::{Block, BorderType, Borders, Paragraph},
};

use super::{Plugin, PluginContext, PluginEventResult, PluginFrame, PluginRenderMode};

mod audio;
pub use audio::{AudioPlayer, AudioState, PlaybackState};

// ============================================================================
// CONSTANTS & CONFIGURATION
// ============================================================================

/// Internal render resolution
const VIS_WIDTH: u32 = 640;
const VIS_HEIGHT: u32 = 320;

/// Number of frequency bands
const BANDS: usize = 32;

/// Waveform sample count
const WAVEFORM_SAMPLES: usize = 128;

/// Bar configuration
const BAR_WIDTH: u32 = 12;
const BAR_SPACING: u32 = 4;
const MIN_BAR_HEIGHT: u32 = 4;

/// Visualizer area dimensions
const VISUALIZER_Y: u32 = 0;
const VISUALIZER_H: u32 = 200;

/// Track info area
const TRACK_INFO_Y: u32 = 210;

/// Progress bar area
const PROGRESS_Y: u32 = 300;

// ============================================================================
// XP COLOR PALETTE
// ============================================================================

/// XP-style gradient colors (from dark to light teal/blue)
const XP_GRADIENT: &[u32] = &[
    0x0D2B45, // Dark navy
    0x154277, // Deep blue
    0x1C5A8E, // Medium blue
    0x2473A6, // XP blue
    0x358FC1, // Light blue
    0x4BA3D3, // Teal blue
    0x6DBDD6, // Light teal
    0x8ED2E2, // Pale teal
];

// ============================================================================
// VISUALIZER STATE
// ============================================================================

// ============================================================================
// VISUALIZER MODES
// ============================================================================

/// Visualizer display mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerMode {
    /// Standard spectrum analyzer (XP-style bars)
    Spectrum,
    /// Waveform display
    Waveform,
    /// Equalizer-style frequency display
    Equalizer,
}

impl Default for VisualizerMode {
    fn default() -> Self {
        VisualizerMode::Spectrum
    }
}

/// Transition state between modes
#[derive(Debug, Clone, Copy)]
struct ModeTransition {
    /// Is transition active
    active: bool,
    /// Transition progress (0.0 to 1.0)
    progress: f32,
    /// Source mode
    from_mode: VisualizerMode,
    /// Target mode
    to_mode: VisualizerMode,
}

impl Default for ModeTransition {
    fn default() -> Self {
        Self {
            active: false,
            progress: 0.0,
            from_mode: VisualizerMode::Spectrum,
            to_mode: VisualizerMode::Spectrum,
        }
    }
}

/// Visualizer animation state
#[derive(Debug, Clone)]
struct VisualizerState {
    /// Current band heights (0.0 to 1.0)
    band_heights: [f32; BANDS],
    /// Target band heights for smooth interpolation
    target_heights: [f32; BANDS],
    /// Peak heights for each band
    peak_heights: [f32; BANDS],
    /// Animation phase for smooth transitions
    phase: f32,
    /// Waveform sample history
    waveform_samples: [f32; WAVEFORM_SAMPLES],
    /// Current waveform position
    waveform_pos: usize,
}

impl Default for VisualizerState {
    fn default() -> Self {
        Self {
            band_heights: [0.5; BANDS],
            target_heights: [0.5; BANDS],
            peak_heights: [0.0; BANDS],
            phase: 0.0,
            waveform_samples: [0.0; WAVEFORM_SAMPLES],
            waveform_pos: 0,
        }
    }
}

/// Visualizer plugin main struct
pub struct VisualizerPlugin {
    /// Audio player instance
    player: Arc<AudioPlayer>,
    /// Current audio state
    audio_state: AudioState,
    /// Visualizer animation state
    vis_state: VisualizerState,
    /// Current visualizer mode
    mode: VisualizerMode,
    /// Mode transition state
    mode_transition: ModeTransition,
    /// Frame buffer for rendering
    frame_buffer: Vec<u8>,
    /// Whether frame is ready to render
    frame_ready: bool,
    /// Volume feedback timer (show volume when changed)
    volume_feedback_timer: f32,
    /// Current volume display value
    display_volume: u8,
    /// Message to display
    message: String,
    /// Message timer
    message_timer: f32,
}

impl VisualizerPlugin {
    /// Create new visualizer plugin
    pub fn new() -> Self {
        let size = (VIS_WIDTH * VIS_HEIGHT * 4) as usize;
        Self {
            player: Arc::new(AudioPlayer::new()),
            audio_state: AudioState::default(),
            vis_state: VisualizerState::default(),
            mode: VisualizerMode::Spectrum,
            mode_transition: ModeTransition::default(),
            frame_buffer: Vec::with_capacity(size),
            frame_ready: false,
            volume_feedback_timer: 0.0,
            display_volume: 80,
            message: "Drop audio files here or use /play".to_string(),
            message_timer: 3.0,
        }
    }

    /// Switch to a different visualizer mode with smooth transition
    fn switch_mode(&mut self, new_mode: VisualizerMode) {
        if new_mode != self.mode {
            self.mode_transition = ModeTransition {
                active: true,
                progress: 0.0,
                from_mode: self.mode,
                to_mode: new_mode,
            };
            self.mode = new_mode;
            self.message = format!("Mode: {:?}", new_mode);
            self.message_timer = 1.5;
            self.frame_ready = true;
        }
    }

    /// Cycle to next visualizer mode
    fn next_mode(&mut self) {
        let next_mode = match self.mode {
            VisualizerMode::Spectrum => VisualizerMode::Waveform,
            VisualizerMode::Waveform => VisualizerMode::Equalizer,
            VisualizerMode::Equalizer => VisualizerMode::Spectrum,
        };
        self.switch_mode(next_mode);
    }

    /// Update visualizer animation state
    fn update_visuals(&mut self, dt: f32) {
        self.vis_state.phase += dt * 2.0;

        // Generate new target heights based on audio state
        for i in 0..BANDS {
            // Create wave-like pattern based on band index
            let base = (i as f32 / BANDS as f32) * std::f32::consts::PI * 2.0;
            let wave = (self.vis_state.phase + base).sin() * 0.5 + 0.5;

            // Add randomness based on playback state
            let random_factor = match self.audio_state.playback_state {
                PlaybackState::Playing => {
                    // More active when playing
                    (i as f32 * 0.3 + self.vis_state.phase * 3.0).sin() * 0.3
                }
                PlaybackState::Paused => {
                    // Gentler when paused
                    (i as f32 * 0.1).sin() * 0.1
                }
                PlaybackState::Stopped => 0.0,
            };

            // Target height varies by band
            let target = wave * 0.7 + random_factor;
            self.vis_state.target_heights[i] = target.clamp(0.05, 1.0);
        }

        // Smoothly interpolate current heights towards targets
        let interpolation = 0.15; // Smoothing factor
        for i in 0..BANDS {
            let diff = self.vis_state.target_heights[i] - self.vis_state.band_heights[i];
            self.vis_state.band_heights[i] += diff * interpolation;

            // Update peak heights
            if self.vis_state.band_heights[i] > self.vis_state.peak_heights[i] {
                self.vis_state.peak_heights[i] = self.vis_state.band_heights[i];
            } else {
                self.vis_state.peak_heights[i] *= 0.98; // Decay peaks
            }
        }

        // Update timers
        if self.volume_feedback_timer > 0.0 {
            self.volume_feedback_timer -= dt;
        }
        if self.message_timer > 0.0 {
            self.message_timer -= dt;
        }

        self.frame_ready = true;
    }

    /// Render a single frame to buffer
    fn render_to_buffer(&mut self) {
        let width = VIS_WIDTH as usize;
        let height = VIS_HEIGHT as usize;
        let size = width * height * 4;

        self.frame_buffer.clear();
        self.frame_buffer.resize(size, 0);

        // Fill background with dark XP-style background
        let bg_r = 16;
        let bg_g = 32;
        let bg_b = 48;

        for chunk in self.frame_buffer.chunks_exact_mut(4) {
            chunk[0] = bg_r;
            chunk[1] = bg_g;
            chunk[2] = bg_b;
            chunk[3] = 255;
        }

        // Draw visualizer bars
        self.render_bars();

        // Draw track info
        self.render_track_info();

        // Draw progress bar
        self.render_progress();

        // Draw volume feedback if active
        if self.volume_feedback_timer > 0.0 {
            self.render_volume_feedback();
        }

        // Draw message if active
        if self.message_timer > 0.0 && !self.message.is_empty() {
            self.render_message();
        }
    }

    /// Render spectrum analyzer bars
    fn render_bars(&mut self) {
        let width = VIS_WIDTH as usize;
        let bar_area_width = BANDS as u32 * (BAR_WIDTH + BAR_SPACING);
        let start_x = (VIS_WIDTH - bar_area_width) / 2;

        for band in 0..BANDS {
            let height_normalized = self.vis_state.band_heights[band];
            let peak_normalized = self.vis_state.peak_heights[band];

            let bar_h = (height_normalized * VISUALIZER_H as f32) as u32;
            let peak_h = (peak_normalized * VISUALIZER_H as f32) as u32;

            // Get color from gradient based on height
            let color_idx = ((height_normalized * (XP_GRADIENT.len() - 1) as f32) as usize)
                .min(XP_GRADIENT.len() - 1);
            let color = XP_GRADIENT[color_idx];

            let r = ((color >> 16) & 0xFF) as u8;
            let g = ((color >> 8) & 0xFF) as u8;
            let b = (color & 0xFF) as u8;

            let x_start = (start_x + band as u32 * (BAR_WIDTH + BAR_SPACING)) as usize;
            let x_end = (x_start + BAR_WIDTH as usize).min(width);

            // Draw bar (from bottom up)
            let bar_bottom = VISUALIZER_Y + VISUALIZER_H;
            let y_start = (bar_bottom - bar_h as u32) as usize;
            let y_end = bar_bottom as usize;

            for y in y_start..y_end {
                for x in x_start..x_end {
                    let offset = (y * width + x) * 4;
                    if offset + 3 < self.frame_buffer.len() {
                        self.frame_buffer[offset] = r;
                        self.frame_buffer[offset + 1] = g;
                        self.frame_buffer[offset + 2] = b;
                        self.frame_buffer[offset + 3] = 255;
                    }
                }
            }

            // Draw peak indicator
            if peak_h > MIN_BAR_HEIGHT {
                let peak_y = (bar_bottom - peak_h as u32) as usize;
                if peak_y > 0 && peak_y < VIS_HEIGHT as usize {
                    let peak_color = 0xFFFFFF; // White peak
                    let pr = (peak_color >> 16) as u8;
                    let pg = (peak_color >> 8) as u8;
                    let pb = peak_color as u8;

                    for x in x_start..x_end {
                        let offset = (peak_y * width + x) * 4;
                        if offset + 3 < self.frame_buffer.len() {
                            self.frame_buffer[offset] = pr;
                            self.frame_buffer[offset + 1] = pg;
                            self.frame_buffer[offset + 2] = pb;
                            self.frame_buffer[offset + 3] = 255;
                        }
                    }
                }
            }
        }
    }

    /// Render track information
    fn render_track_info(&mut self) {
        let width = VIS_WIDTH as usize;

        // Track title/artist
        let track_text = if let Some(ref track) = self.audio_state.current_track {
            track.display_string()
        } else {
            "No track loaded".to_string()
        };

        // Playback state indicator
        let state_icon = match self.audio_state.playback_state {
            PlaybackState::Playing => "▶",
            PlaybackState::Paused => "⏸",
            PlaybackState::Stopped => "⏹",
        };

        // Combine info text
        let info_text = format!("{}  {}", state_icon, track_text);

        // Render text (simple ASCII approximation for pixel buffer)
        let text_x = 10;
        let text_y = TRACK_INFO_Y as usize + 15;

        for (i, ch) in info_text.chars().enumerate() {
            let x = text_x + i * 8;
            if x + 8 < width && text_y + 16 < VIS_HEIGHT as usize {
                // Draw character approximation (simplified)
                self.draw_char_8x16(x, text_y, ch, 0xFFFFFF);
            }
        }
    }

    /// Render progress bar
    fn render_progress(&mut self) {
        let width = VIS_WIDTH as usize;
        let progress_width = VIS_WIDTH - 40;
        let progress_x = 20;
        let progress_y = PROGRESS_Y as usize + 5;

        let progress = if self.audio_state.duration > 0.0 {
            (self.audio_state.position / self.audio_state.duration).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let filled_width = (progress * progress_width as f64) as u32;

        // Draw progress bar background
        for x in progress_x..progress_x + progress_width as usize {
            for y in progress_y..progress_y + 6 {
                let offset = (y * width + x) * 4;
                if offset + 3 < self.frame_buffer.len() {
                    // Dark gray background
                    self.frame_buffer[offset] = 64;
                    self.frame_buffer[offset + 1] = 64;
                    self.frame_buffer[offset + 2] = 64;
                    self.frame_buffer[offset + 3] = 255;
                }
            }
        }

        // Draw progress bar fill (XP blue)
        for x in progress_x..progress_x + filled_width as usize {
            for y in progress_y..progress_y + 6 {
                let offset = (y * width + x) * 4;
                if offset + 3 < self.frame_buffer.len() {
                    self.frame_buffer[offset] = 53;
                    self.frame_buffer[offset + 1] = 142;
                    self.frame_buffer[offset + 2] = 210;
                    self.frame_buffer[offset + 3] = 255;
                }
            }
        }
    }

    /// Render volume feedback
    fn render_volume_feedback(&mut self) {
        let _width = VIS_WIDTH as usize;
        let text = format!("Volume: {}%", self.display_volume);
        let text_x = (VIS_WIDTH as usize / 2) - (text.len() * 8 / 2);
        let text_y = 20;

        for (i, ch) in text.chars().enumerate() {
            let x = text_x + i * 8;
            self.draw_char_8x16(x, text_y, ch, 0xFFFFFF);
        }
    }

    /// Render message overlay
    fn render_message(&mut self) {
        let _width = VIS_WIDTH as usize;
        let text_x = (VIS_WIDTH as usize / 2) - (self.message.len() * 8 / 2);
        let text_y = VIS_HEIGHT as usize / 2;

        // Clone message to avoid borrowing self.message during iteration
        let message = self.message.clone();
        for (i, ch) in message.chars().enumerate() {
            let x = text_x + i * 8;
            self.draw_char_8x16(x, text_y, ch, 0xAAAAAA);
        }
    }

    /// Draw a simple 8x16 character approximation
    fn draw_char_8x16(&mut self, x: usize, y: usize, ch: char, color: u32) {
        let width = VIS_WIDTH as usize;
        let r = ((color >> 16) & 0xFF) as u8;
        let g = ((color >> 8) & 0xFF) as u8;
        let b = (color & 0xFF) as u8;

        // Simple character bitmap (very rough approximation)
        let bitmap = get_char_bitmap(ch);

        for row in 0..16 {
            for col in 0..8 {
                if bitmap[row] & (1 << (7 - col)) != 0 {
                    let px = x + col;
                    let py = y + row;
                    if px < width && py < VIS_HEIGHT as usize {
                        let offset = (py * width + px) * 4;
                        if offset + 3 < self.frame_buffer.len() {
                            self.frame_buffer[offset] = r;
                            self.frame_buffer[offset + 1] = g;
                            self.frame_buffer[offset + 2] = b;
                            self.frame_buffer[offset + 3] = 255;
                        }
                    }
                }
            }
        }
    }
}

/// Get simple bitmap for character (very rough approximation)
fn get_char_bitmap(ch: char) -> [u8; 16] {
    // Very simplified bitmap - just shows character is present
    match ch {
        ' ' => [0; 16],
        '!' => [
            0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0x18, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
        'A' => [
            0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0, 0,
        ],
        'P' => [
            0x7E, 0x18, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x18, 0x18, 0, 0, 0, 0, 0, 0, 0,
        ],
        'L' => [
            0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x7E, 0, 0, 0, 0, 0,
        ],
        'a' => [
            0, 0, 0, 0, 0x3C, 0x42, 0x42, 0x42, 0x3C, 0x40, 0x40, 0x42, 0x3C, 0, 0, 0,
        ],
        'y' => [
            0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x18, 0x7E, 0, 0, 0,
        ],
        ':' => [0, 0, 0x18, 0x18, 0, 0, 0, 0, 0, 0, 0x18, 0x18, 0, 0, 0, 0],
        '%' => [
            0x24, 0x24, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x42, 0x42, 0x24, 0x24, 0,
            0,
        ],
        '0'..='9' => {
            let n = ch.to_digit(10).unwrap_or(0) as usize;
            DIGITS[n]
        }
        _ => {
            // Generic character outline
            [
                0x7E, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0,
            ]
        }
    }
}

/// Digit bitmaps
const DIGITS: [[u8; 16]; 10] = [
    [
        0x7E, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 0
    [
        0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 1
    [
        0x7E, 0x02, 0x02, 0x02, 0x7E, 0x40, 0x40, 0x40, 0x7E, 0, 0, 0, 0, 0, 0, 0,
    ], // 2
    [
        0x7E, 0x02, 0x02, 0x02, 0x3E, 0x02, 0x02, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 3
    [
        0x42, 0x42, 0x42, 0x42, 0x7E, 0x02, 0x02, 0x02, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 4
    [
        0x7E, 0x40, 0x40, 0x40, 0x7E, 0x02, 0x02, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 5
    [
        0x7E, 0x40, 0x40, 0x40, 0x7E, 0x42, 0x42, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 6
    [
        0x7E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 7
    [
        0x7E, 0x42, 0x42, 0x42, 0x7E, 0x42, 0x42, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 8
    [
        0x7E, 0x42, 0x42, 0x42, 0x7E, 0x02, 0x02, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0,
    ], // 9
];

// ============================================================================
// PLUGIN TRAIT IMPLEMENTATION
// ============================================================================

impl Plugin for VisualizerPlugin {
    fn id(&self) -> &str {
        "visualizer"
    }

    fn name(&self) -> &str {
        "Audio Visualizer"
    }

    fn display_name(&self) -> String {
        let track = &self.audio_state.current_track;
        let track_name = track
            .as_ref()
            .map(|t| t.display_title())
            .unwrap_or_else(|| "No track".to_string());

        let state = match self.audio_state.playback_state {
            PlaybackState::Playing => "▶",
            PlaybackState::Paused => "⏸",
            PlaybackState::Stopped => "⏹",
        };

        format!("Visualizer {} - {}", state, track_name)
    }

    fn render_mode(&self) -> PluginRenderMode {
        PluginRenderMode::KittyGraphics
    }

    fn render(&self, area: Rect, buf: &mut Buffer, _ctx: &PluginContext) {
        // Fallback text rendering when Kitty graphics not available
        let title = Span::styled(
            "🎵 Audio Visualizer",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        let track_info = if let Some(ref track) = self.audio_state.current_track {
            format!("Now playing: {}", track.display_string())
        } else {
            "No track loaded".to_string()
        };

        let controls = vec![
            Line::from(""),
            Line::from("Controls:"),
            Line::from("  Scroll ↑/↓ : Volume"),
            Line::from("  Click      : Play/Pause"),
            Line::from("  Space      : Pause"),
            Line::from("  ←/→        : Seek"),
            Line::from("  N          : Next track"),
            Line::from("  P          : Previous track"),
            Line::from(""),
            Line::from(Span::styled(track_info, Style::default().fg(Color::Yellow))),
        ];

        let paragraph =
            Paragraph::new(vec![Line::from(title)]).alignment(ratatui::layout::Alignment::Center);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan));

        block.render(area, buf);
        paragraph.render(area, buf);

        let info_area = Rect {
            x: area.x,
            y: area.y + 2,
            width: area.width,
            height: area.height.saturating_sub(2),
        };

        let info_paragraph = Paragraph::new(controls).alignment(ratatui::layout::Alignment::Left);
        info_paragraph.render(info_area, buf);
    }

    fn render_frame(&mut self, _width: u32, _height: u32) -> Option<PluginFrame> {
        if !self.frame_ready {
            return None;
        }

        self.render_to_buffer();
        self.frame_ready = false;

        Some(PluginFrame::from_arc(
            Arc::new(std::mem::take(&mut self.frame_buffer)),
            VIS_WIDTH,
            VIS_HEIGHT,
        ))
    }

    fn handle_event(&mut self, event: &Event, _area: Rect) -> PluginEventResult {
        match event {
            Event::Mouse(mouse) => {
                // Handle scroll for volume
                if let crossterm::event::MouseEventKind::ScrollUp = mouse.kind {
                    let vol = self.player.adjust_volume(5);
                    self.display_volume = vol;
                    self.volume_feedback_timer = 1.0;
                    self.frame_ready = true;
                    return PluginEventResult::Consumed;
                }

                if let crossterm::event::MouseEventKind::ScrollDown = mouse.kind {
                    let vol = self.player.adjust_volume(-5);
                    self.display_volume = vol;
                    self.volume_feedback_timer = 1.0;
                    self.frame_ready = true;
                    return PluginEventResult::Consumed;
                }

                // Handle click for play/pause
                if let crossterm::event::MouseEventKind::Down(_) = mouse.kind {
                    self.player.toggle_pause();
                    self.frame_ready = true;
                    return PluginEventResult::Consumed;
                }
            }
            Event::Key(KeyEvent {
                code,
                modifiers,
                kind,
                ..
            }) if *kind == KeyEventKind::Press || *kind == KeyEventKind::Repeat => {
                let no_modifiers = *modifiers == KeyModifiers::NONE;

                match (code, no_modifiers) {
                    // Play/pause
                    (KeyCode::Char(' '), true) => {
                        self.player.toggle_pause();
                        self.frame_ready = true;
                        return PluginEventResult::Consumed;
                    }

                    // Volume up
                    (KeyCode::Up, true) | (KeyCode::Char('+'), true) => {
                        let vol = self.player.adjust_volume(5);
                        self.display_volume = vol;
                        self.volume_feedback_timer = 1.0;
                        self.frame_ready = true;
                        return PluginEventResult::Consumed;
                    }

                    // Volume down
                    (KeyCode::Down, true) | (KeyCode::Char('-'), true) => {
                        let vol = self.player.adjust_volume(-5);
                        self.display_volume = vol;
                        self.volume_feedback_timer = 1.0;
                        self.frame_ready = true;
                        return PluginEventResult::Consumed;
                    }

                    // Seek forward
                    (KeyCode::Right, true) | (KeyCode::Char('f'), true) => {
                        self.player.seek(5.0);
                        self.frame_ready = true;
                        return PluginEventResult::Consumed;
                    }

                    // Seek backward
                    (KeyCode::Left, true) | (KeyCode::Char('b'), true) => {
                        self.player.seek(-5.0);
                        self.frame_ready = true;
                        return PluginEventResult::Consumed;
                    }

                    // Stop
                    (KeyCode::Char('s'), true) | (KeyCode::Char('S'), true) => {
                        self.player.stop();
                        self.audio_state.playback_state = PlaybackState::Stopped;
                        self.frame_ready = true;
                        return PluginEventResult::Consumed;
                    }

                    _ => {}
                }
            }
            _ => {}
        }

        PluginEventResult::Ignored
    }

    fn tick(&mut self) -> bool {
        // Update audio state (synchronous)
        self.audio_state = self.player.state();

        // Update visuals
        const DT: f32 = 1.0 / 60.0;
        self.update_visuals(DT);

        self.frame_ready
    }

    fn on_activate(&mut self) {
        // Check if mpv is available
        if !self.player.is_available() {
            self.message = "mpv not found - install mpv for audio".to_string();
            self.message_timer = 5.0;
        }
    }

    fn on_deactivate(&mut self) {
        // Pause playback when switching away
        self.player.pause();
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Default for VisualizerPlugin {
    fn default() -> Self {
        Self::new()
    }
}
