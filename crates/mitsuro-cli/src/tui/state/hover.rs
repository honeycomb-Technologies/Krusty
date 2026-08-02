//! Hover State
//!
//! Tracks mouse hover position for interactive elements like file references and hyperlinks.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Throttle interval for expensive hover detection (50ms = 20 updates/sec max)
const HOVER_THROTTLE: Duration = Duration::from_millis(50);

/// Hover state for tracking mouse position over interactive elements
#[derive(Debug, Default)]
pub struct HoverState {
    /// Current mouse position (screen coords)
    pub mouse_pos: Option<(u16, u16)>,
    /// Hovered file reference in messages (message_idx, display_name)
    pub message_file_ref: Option<(usize, String)>,
    /// Hovered file reference in input (byte_start, byte_end, path)
    pub input_file_ref: Option<(usize, usize, PathBuf)>,
    /// Hovered hyperlink in messages (message_idx, line, start_col, end_col, url)
    pub message_link: Option<HoveredLink>,
    /// Last time expensive hover detection was performed
    last_detection: Option<Instant>,
}

impl HoverState {
    /// Check if enough time has passed to run expensive detection.
    /// Returns true if detection should run, and updates the timestamp.
    pub fn should_detect(&mut self) -> bool {
        let now = Instant::now();
        match self.last_detection {
            Some(last) if now.duration_since(last) < HOVER_THROTTLE => false,
            _ => {
                self.last_detection = Some(now);
                true
            }
        }
    }
}

/// Information about a hovered hyperlink
#[derive(Debug, Clone)]
pub struct HoveredLink {
    pub msg_idx: usize,
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub url: String,
}
