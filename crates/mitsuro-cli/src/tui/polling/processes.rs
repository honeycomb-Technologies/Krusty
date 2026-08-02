//! Background process polling
//!
//! Polls ProcessRegistry for background process status updates.

use std::sync::Arc;

use crate::process::{ProcessRegistry, ProcessStatus};
use crate::tui::blocks::BashBlock;

use super::PollResult;

/// Poll ProcessRegistry for background process status updates
/// Updates BashBlocks that are tracking background processes
pub fn poll_background_processes(
    process_registry: &Arc<ProcessRegistry>,
    bash_blocks: &mut [BashBlock],
) -> PollResult {
    let mut result = PollResult::new();

    // Get list of processes without blocking
    let Some(processes) = process_registry.try_list() else {
        return result;
    };

    // Check each background BashBlock
    for block in bash_blocks.iter_mut() {
        // Clone process_id to avoid borrow conflict with block.complete()
        let Some(process_id) = block.background_process_id().map(|s| s.to_string()) else {
            continue;
        };

        // Find matching process in registry
        if let Some(info) = processes.iter().find(|p| p.id == process_id) {
            // Check if process has completed and block is still streaming
            if !info.is_running() && block.is_streaming() {
                result.needs_redraw = true;

                // Process finished - update block status
                let exit_code = match &info.status {
                    ProcessStatus::Completed { exit_code, .. } => *exit_code,
                    ProcessStatus::Failed { .. } => 1,
                    ProcessStatus::Killed { .. } => 137, // SIGKILL
                    ProcessStatus::Running | ProcessStatus::Suspended => continue, // Still alive
                };
                block.complete(exit_code);
                tracing::info!(
                    process_id = %process_id,
                    exit_code = exit_code,
                    "Background BashBlock completed from ProcessRegistry"
                );
            }
        }
    }

    result
}
