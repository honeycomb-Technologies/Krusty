//! Agent state tracking
//!
//! Tracks turn count and timing for safety limits.

use std::time::{Duration, Instant};

use crate::constants;

/// Runtime state of the agent
#[derive(Debug, Default)]
pub struct AgentState {
    /// Current turn number (increments each time we send to AI)
    pub current_turn: usize,
    /// When the current turn started
    pub turn_start: Option<Instant>,
    /// Whether the agent was interrupted
    pub is_interrupted: bool,
}

impl AgentState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new turn
    pub fn start_turn(&mut self) {
        self.current_turn += 1;
        self.turn_start = Some(Instant::now());
        self.is_interrupted = false;
    }

    /// Get duration of current turn
    pub fn turn_duration(&self) -> Option<Duration> {
        self.turn_start.map(|start| start.elapsed())
    }

    /// Mark as interrupted
    pub fn interrupt(&mut self) {
        self.is_interrupted = true;
        self.turn_start = None;
    }

    /// Reset all per-session state.
    pub fn reset(&mut self) {
        self.current_turn = 0;
        self.turn_start = None;
        self.is_interrupted = false;
    }
}

/// Configuration for agent behavior
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Legacy alias for the primary-session turn budget.
    /// `None` means unlimited.
    pub max_turns: Option<usize>,
    /// Maximum turns for primary interactive sessions (None = unlimited).
    pub primary_max_turns: Option<usize>,
    /// Maximum turns for sub-agents (None = unlimited).
    pub subagent_max_turns: Option<usize>,
    /// Maximum turns for ACP sessions (None = unlimited).
    pub acp_max_turns: Option<usize>,
    /// Idle stream timeout in seconds.
    pub stream_idle_timeout_secs: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: None,
            primary_max_turns: None,
            subagent_max_turns: Some(200),
            acp_max_turns: None,
            stream_idle_timeout_secs: constants::http::STREAM_TIMEOUT.as_secs(),
        }
    }
}

impl AgentConfig {
    /// Resolve the primary-session turn budget, honoring the legacy alias.
    pub fn primary_max_turns(&self) -> Option<usize> {
        self.primary_max_turns.or(self.max_turns)
    }

    /// Resolve the ACP turn budget, honoring the legacy alias when no explicit ACP budget exists.
    pub fn acp_max_turns(&self) -> Option<usize> {
        self.acp_max_turns.or(self.max_turns)
    }

    /// Resolve the configured idle stream timeout.
    pub fn stream_idle_timeout(&self) -> Duration {
        Duration::from_secs(self.stream_idle_timeout_secs.max(1))
    }

    /// Check if we've exceeded the primary-session turn budget.
    pub fn exceeded_max_turns(&self, current_turn: usize) -> bool {
        self.primary_max_turns()
            .is_some_and(|max| current_turn >= max)
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentConfig, AgentState};

    #[test]
    fn state_reset_clears_turn_tracking() {
        let mut state = AgentState::new();
        state.start_turn();
        state.interrupt();
        state.reset();

        assert_eq!(state.current_turn, 0);
        assert!(state.turn_start.is_none());
        assert!(!state.is_interrupted);
    }

    #[test]
    fn default_config_is_unlimited_for_primary_and_acp() {
        let config = AgentConfig::default();

        assert_eq!(config.primary_max_turns(), None);
        assert_eq!(config.acp_max_turns(), None);
        assert_eq!(config.subagent_max_turns, Some(200));
        assert!(config.stream_idle_timeout().as_secs() >= 600);
    }
}
