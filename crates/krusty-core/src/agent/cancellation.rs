//! Cancellation support for agent tasks
//!
//! Allows interrupting running API calls and tool executions.

use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

/// Shared wrapper around CancellationToken for agent task cancellation.
///
/// Clones are kept in long-lived tool registry entries, so reset must be visible
/// through every clone after a user interrupt; otherwise later subagents inherit
/// an already-cancelled token and exit immediately.
#[derive(Clone)]
pub struct AgentCancellation {
    token: Arc<Mutex<CancellationToken>>,
}

impl AgentCancellation {
    pub fn new() -> Self {
        Self {
            token: Arc::new(Mutex::new(CancellationToken::new())),
        }
    }

    /// Cancel all tasks using the current token.
    pub fn cancel(&self) {
        self.token.lock().expect("cancellation token mutex").cancel();
    }

    /// Get a child token for a subtask.
    pub fn child_token(&self) -> CancellationToken {
        self.token
            .lock()
            .expect("cancellation token mutex")
            .child_token()
    }

    /// Create a fresh token (for starting a new request).
    pub fn reset(&self) {
        let mut token = self.token.lock().expect("cancellation token mutex");
        *token = CancellationToken::new();
    }
}

impl Default for AgentCancellation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::AgentCancellation;

    #[test]
    fn reset_is_visible_to_existing_clones() {
        let cancellation = AgentCancellation::new();
        let clone = cancellation.clone();

        cancellation.cancel();
        assert!(clone.child_token().is_cancelled());

        cancellation.reset();
        assert!(!clone.child_token().is_cancelled());
    }
}
