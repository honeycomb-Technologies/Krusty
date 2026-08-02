//! Deterministic shared motion clock.

use std::time::Duration;

/// Default spinner cadence (~12 fps). Used when only status glyphs animate.
pub const MOTION_FRAME_INTERVAL: Duration = Duration::from_millis(84);

/// Home splash / high-FPS ambient scenes. 16 ms ≈ 60 fps cap; Skip missed ticks
/// under load so the event loop never piles up redraw debt.
pub const SPLASH_FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MotionClock {
    elapsed_ms: u64,
}

impl MotionClock {
    pub const fn elapsed_ms(self) -> u64 {
        self.elapsed_ms
    }

    /// Advances monotonically from a runtime-provided timestamp.
    ///
    /// No wall-clock access occurs here, keeping reducer replay deterministic.
    pub fn advance_to(&mut self, elapsed_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.max(elapsed_ms);
    }

    pub const fn frame(self, frame_count: usize, interval_ms: u64) -> usize {
        if frame_count == 0 || interval_ms == 0 {
            return 0;
        }

        ((self.elapsed_ms / interval_ms) % frame_count as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic_and_replayable() {
        let mut clock = MotionClock::default();
        clock.advance_to(280);
        assert_eq!(clock.frame(4, 140), 2);

        clock.advance_to(100);
        assert_eq!(clock.elapsed_ms(), 280);
    }
}
