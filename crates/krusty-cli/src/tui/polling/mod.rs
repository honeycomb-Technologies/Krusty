//! Channel Polling
//!
//! Extracted polling logic from App. These functions poll async channels
//! for background task results and update the appropriate state.
//!
//! This module reduces the App god object by extracting ~500 lines of
//! channel polling logic into focused, testable functions.

mod bash;
mod blocks;
mod dual_mind;
mod mcp;
mod oauth;
mod processes;

pub use bash::poll_bash_output;
pub use blocks::{poll_build_progress, poll_explore_progress, poll_init_exploration};
pub use dual_mind::poll_dual_mind;
pub use mcp::poll_mcp_status;
pub use oauth::poll_oauth_status;
pub use processes::poll_background_processes;

use krusty_core::ai::providers::ProviderId;

/// Result of a polling operation that may trigger UI updates
#[derive(Debug, Default)]
pub struct PollResult {
    /// Whether any data was received that requires a redraw
    pub needs_redraw: bool,
    /// Messages to append to the conversation
    pub messages: Vec<(String, String)>,
    /// Actions for App to take after polling (avoids borrow conflicts)
    pub actions: Vec<PollAction>,
}

/// Actions that App should take after polling completes
/// This pattern avoids borrow conflicts - pollers return what to do,
/// App executes after borrows are released
#[derive(Debug, Clone)]
pub enum PollAction {
    /// Refresh MCP popup server list
    RefreshMcpPopup,
    /// Refresh cached AI tools
    RefreshAiTools,
    /// Switch to a provider (after OAuth success)
    SwitchProvider(ProviderId),
}

impl PollResult {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_message(mut self, role: impl Into<String>, content: impl Into<String>) -> Self {
        self.messages.push((role.into(), content.into()));
        self
    }

    pub fn with_action(mut self, action: PollAction) -> Self {
        self.actions.push(action);
        self
    }
}
