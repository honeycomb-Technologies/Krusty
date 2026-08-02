//! Agent event bus
//!
//! Central hub for agent events.

use super::events::AgentEvent;

/// Event bus for agent events
pub struct AgentEventBus {
    // Events are logged for debugging via tracing
}

impl AgentEventBus {
    pub fn new() -> Self {
        Self {}
    }

    /// Emit an event (logged via tracing)
    pub fn emit(&mut self, event: AgentEvent) {
        tracing::debug!("Agent event: {:?}", event);
    }
}

impl Default for AgentEventBus {
    fn default() -> Self {
        Self::new()
    }
}
