//! Shared stream activity language for TUI blocks.
//!
//! Center-outward echo frames match the mobile DotEcho indicator so thinking
//! and tool activity share one motion language.

use std::time::Duration;

/// Center-outward echo frames shared with the mobile activity language.
pub const ACTIVITY_ECHO_FRAMES: &[&str] = &["··•··", "·•●•·", "•●•●•", "●•·•●", "•···•", "·····"];

/// Interval between activity frame advances.
pub const ACTIVITY_ECHO_INTERVAL: Duration = Duration::from_millis(130);

/// Resolve the current activity echo frame.
pub fn activity_echo_frame(index: usize) -> &'static str {
    ACTIVITY_ECHO_FRAMES[index % ACTIVITY_ECHO_FRAMES.len()]
}
