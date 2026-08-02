//! One shared, deterministic motion state.

pub mod clock;
pub mod preference;

use clock::MotionClock;
use preference::MotionPreference;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MotionState {
    pub preference: MotionPreference,
    pub clock: MotionClock,
    pub terminal_focused: bool,
    active_regions: u8,
}

impl MotionState {
    pub fn new(preference: MotionPreference) -> Self {
        Self {
            preference,
            clock: MotionClock::default(),
            terminal_focused: true,
            active_regions: 0,
        }
    }

    pub const fn wants_tick(self) -> bool {
        matches!(self.preference, MotionPreference::Full)
            && self.terminal_focused
            && self.active_regions > 0
    }

    pub fn set_active_regions(&mut self, count: u8) {
        self.active_regions = count.min(2);
    }

    pub const fn active_regions(self) -> u8 {
        self.active_regions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_state_never_requests_animation_and_regions_are_bounded() {
        let mut state = MotionState::new(MotionPreference::Full);
        assert!(!state.wants_tick());

        state.set_active_regions(9);
        assert_eq!(state.active_regions(), 2);
        assert!(state.wants_tick());

        state.preference = MotionPreference::Reduced;
        assert!(!state.wants_tick());
        state.preference = MotionPreference::Off;
        assert!(!state.wants_tick());
    }
}
