//! Bash output channel polling
//!
//! Handles streaming output from bash tool executions.

use std::sync::Arc;

use crate::process::ProcessRegistry;
use crate::tui::blocks::BashBlock;
use crate::tui::state::ScrollState;
use crate::tui::utils::AsyncChannels;

use super::PollResult;

/// Poll bash output channel for streaming updates
///
/// Updates BashBlocks with output chunks and completion signals.
/// Returns whether any updates were received.
pub fn poll_bash_output(
    channels: &mut AsyncChannels,
    bash_blocks: &mut [BashBlock],
    scroll: &mut ScrollState,
    process_registry: &Arc<ProcessRegistry>,
) -> PollResult {
    let mut result = PollResult::new();

    // Take the receiver temporarily to poll it
    let Some(mut rx) = channels.bash_output.take() else {
        return result;
    };

    // Poll all available chunks (non-blocking)
    loop {
        match rx.try_recv() {
            Ok(chunk) => {
                result.needs_redraw = true;

                // Find the BashBlock with matching tool_use_id
                // First try to find by ID, then fall back to last
                let block_idx = bash_blocks
                    .iter()
                    .position(|b| b.tool_use_id() == Some(&chunk.tool_use_id))
                    .or_else(|| {
                        if bash_blocks.is_empty() {
                            None
                        } else {
                            Some(bash_blocks.len() - 1)
                        }
                    });
                let block = block_idx.and_then(|i| bash_blocks.get_mut(i));

                if let Some(block) = block {
                    if chunk.is_complete {
                        if !block.is_streaming() {
                            continue;
                        }

                        // Mark block as complete with exit code
                        let exit_code = chunk.exit_code.unwrap_or(0);
                        tracing::info!(
                            tool_use_id = %chunk.tool_use_id,
                            exit_code = exit_code,
                            "Bash block complete signal received"
                        );
                        block.complete(exit_code);

                        // Update ProcessRegistry status (fire and forget)
                        let registry = process_registry.clone();
                        let tool_id = chunk.tool_use_id.clone();
                        tokio::spawn(async move {
                            let status = if exit_code == 0 {
                                crate::process::ProcessStatus::Completed {
                                    exit_code,
                                    duration_ms: 0, // Duration tracked by block
                                }
                            } else {
                                crate::process::ProcessStatus::Failed {
                                    error: format!("Exit code: {}", exit_code),
                                    duration_ms: 0,
                                }
                            };
                            registry.update_status(&tool_id, status).await;
                        });
                    } else if !chunk.chunk.is_empty() {
                        // Append output chunk
                        block.append(&chunk.chunk);
                    }
                }

                if scroll.auto_scroll {
                    scroll.request_scroll_to_bottom();
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                // No more data available, put receiver back
                channels.bash_output = Some(rx);
                break;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // Channel closed - complete any remaining streaming blocks
                // This usually means the tool finished but we missed the completion signal
                tracing::debug!("Bash output channel disconnected");
                for block in bash_blocks.iter_mut() {
                    // Skip background blocks - they're tracked by ProcessRegistry
                    if block.background_process_id().is_some() {
                        continue;
                    }
                    if block.is_streaming() {
                        tracing::warn!(
                            "Completing bash block on channel disconnect without exit status"
                        );
                        block.complete(-1);

                        // Update ProcessRegistry if we have a tool_use_id
                        if let Some(tool_id) = block.tool_use_id() {
                            let registry = process_registry.clone();
                            let tool_id = tool_id.to_string();
                            tokio::spawn(async move {
                                let status = crate::process::ProcessStatus::Failed {
                                    error: "Output channel disconnected before exit status arrived"
                                        .to_string(),
                                    duration_ms: 0,
                                };
                                registry.update_status(&tool_id, status).await;
                            });
                        }
                    }
                }
                break;
            }
        }
    }

    result
}
