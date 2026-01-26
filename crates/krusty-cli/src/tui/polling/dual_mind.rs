//! Dual-mind dialogue channel polling
//!
//! Handles dialogue updates from the Big Claw / Little Claw system.

use crate::tui::utils::{AsyncChannels, DualMindPhase};

use super::PollResult;

/// Poll dual-mind dialogue channel and return updates for display
pub fn poll_dual_mind(channels: &mut AsyncChannels) -> PollResult {
    let mut result = PollResult::new();

    let Some(mut rx) = channels.dual_mind.take() else {
        return result;
    };

    loop {
        match rx.try_recv() {
            Ok(update) => {
                result.needs_redraw = true;

                // Format dialogue for display
                if !update.dialogue.is_empty() {
                    let phase_label = match update.phase {
                        DualMindPhase::PreReview => "Pre-Review",
                        DualMindPhase::PostReview => "Post-Review",
                    };
                    result.messages.push((
                        "dual_mind".to_string(),
                        format!("[{}]\n{}", phase_label, update.dialogue),
                    ));
                }

                // Show enhancement if present
                if let Some(enhancement) = update.enhancement {
                    result.messages.push((
                        "dual_mind_enhancement".to_string(),
                        format!("[Little Claw Concern]: {}", enhancement),
                    ));
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                channels.dual_mind = Some(rx);
                break;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                tracing::debug!("Dual-mind dialogue channel disconnected");
                break;
            }
        }
    }

    result
}
