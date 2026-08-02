//! MCP status channel polling
//!
//! Handles status updates from background MCP connection tasks.

use crate::tui::popups::mcp_browser::McpBrowserPopup;
use crate::tui::utils::AsyncChannels;

use super::{PollAction, PollResult};

/// Poll MCP status updates from background connection tasks
///
/// Returns actions for App to execute (RefreshMcpPopup, RefreshAiTools)
/// to avoid borrow conflicts with self methods.
pub fn poll_mcp_status(
    channels: &mut AsyncChannels,
    mcp_popup: &mut McpBrowserPopup,
) -> PollResult {
    let mut result = PollResult::new();

    let Some(mut rx) = channels.mcp_status.take() else {
        return result;
    };

    loop {
        match rx.try_recv() {
            Ok(update) => {
                result.needs_redraw = true;

                // Update popup status message
                let status_msg = if update.success {
                    format!("✓ {}", update.message)
                } else {
                    format!("✗ {}", update.message)
                };
                mcp_popup.set_status(status_msg);

                // Queue actions for App to execute after borrow ends
                result = result.with_action(PollAction::RefreshMcpPopup);

                if update.success {
                    result = result.with_action(PollAction::RefreshAiTools);
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                channels.mcp_status = Some(rx);
                break;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                break;
            }
        }
    }

    result
}
