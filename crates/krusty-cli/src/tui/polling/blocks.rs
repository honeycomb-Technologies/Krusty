//! Block progress channel polling
//!
//! Handles progress updates for explore and build agents.

use std::path::Path;

use crate::plan::{PlanFile, PlanManager};
use crate::tui::blocks::{BuildBlock, ExploreBlock, StreamBlock};
use crate::tui::handlers::commands::generate_krab_from_exploration;
use crate::tui::utils::AsyncChannels;

use super::{PollAction, PollResult};

/// Poll explore progress channel and update ExploreBlock with agent progress
pub fn poll_explore_progress(
    channels: &mut AsyncChannels,
    explore_blocks: &mut [ExploreBlock],
) -> PollResult {
    let mut result = PollResult::new();

    let Some(mut rx) = channels.explore_progress.take() else {
        return result;
    };

    loop {
        match rx.try_recv() {
            Ok(progress) => {
                result.needs_redraw = true;
                // Find matching ExploreBlock by tool_use_id (derived from task_id prefix)
                // Task IDs are like "dir-0", "file-1", "main" - we find the parent explore block
                // by looking for blocks that are still streaming
                for block in explore_blocks.iter_mut() {
                    if block.is_streaming() {
                        block.update_progress(progress.clone());
                        break;
                    }
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                channels.explore_progress = Some(rx);
                break;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                tracing::debug!("Explore progress channel disconnected");
                break;
            }
        }
    }

    result
}

/// Poll build progress channel and update BuildBlock with builder progress
pub fn poll_build_progress(
    channels: &mut AsyncChannels,
    build_blocks: &mut [BuildBlock],
    active_plan: &mut Option<PlanFile>,
    plan_manager: &PlanManager,
) -> PollResult {
    let mut result = PollResult::new();

    let Some(mut rx) = channels.build_progress.take() else {
        return result;
    };

    loop {
        match rx.try_recv() {
            Ok(progress) => {
                result.needs_redraw = true;
                // Find matching BuildBlock that is still streaming
                for block in build_blocks.iter_mut() {
                    if block.is_streaming() {
                        block.update_progress(progress.clone());
                        break;
                    }
                }

                // Auto-complete plan task if specified
                if let Some(ref task_id) = progress.completed_plan_task {
                    if let Some(ref mut plan) = active_plan {
                        if plan.check_task(task_id) {
                            tracing::debug!(task_id = %task_id, "Kraken auto-completed plan task");
                            if let Err(e) = plan_manager.save_plan(plan) {
                                tracing::warn!("Failed to save plan after task completion: {}", e);
                            }
                        }
                    }
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                channels.build_progress = Some(rx);
                break;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                tracing::debug!("Build progress channel disconnected");
                break;
            }
        }
    }

    result
}

/// Poll /init exploration progress and result
///
/// Uses cached languages from /init start. Clears cache on completion.
pub fn poll_init_exploration(
    channels: &mut AsyncChannels,
    explore_blocks: &mut [ExploreBlock],
    init_explore_id: &mut Option<String>,
    cached_languages: &mut Option<Vec<String>>,
    working_dir: &Path,
    languages: &[String],
) -> PollResult {
    let mut result = PollResult::new();

    // Poll progress channel - route to ExploreBlock
    if let Some(mut rx) = channels.init_progress.take() {
        loop {
            match rx.try_recv() {
                Ok(progress) => {
                    result.needs_redraw = true;
                    // Find the init ExploreBlock and update it
                    if let Some(ref explore_id) = init_explore_id {
                        for block in explore_blocks.iter_mut() {
                            if block.tool_use_id() == Some(explore_id.as_str()) {
                                block.update_progress(progress.clone());
                                break;
                            }
                        }
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    channels.init_progress = Some(rx);
                    break;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }
    }

    // Poll result channel for completion
    if let Some(mut rx) = channels.init_exploration.take() {
        match rx.try_recv() {
            Ok(exploration_result) => {
                result.needs_redraw = true;

                // Complete the ExploreBlock
                if let Some(ref explore_id) = init_explore_id {
                    for block in explore_blocks.iter_mut() {
                        if block.tool_use_id() == Some(explore_id.as_str()) {
                            block.complete(String::new());
                            break;
                        }
                    }
                }
                *init_explore_id = None;
                *cached_languages = None; // Clear language cache on completion

                if exploration_result.success {
                    // Generate KRAB.md from exploration results
                    let project_name = working_dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Project".to_string());

                    let krab_path = working_dir.join("KRAB.md");
                    let is_regenerate = krab_path.exists();

                    // Try to preserve user's "Notes for AI" section if regenerating
                    let preserved_notes = if is_regenerate {
                        std::fs::read_to_string(&krab_path)
                            .ok()
                            .and_then(|content| {
                                content.find("## Notes for AI").map(|pos| {
                                    let notes_section = &content[pos..];
                                    notes_section
                                        .lines()
                                        .skip(1)
                                        .skip_while(|l| l.starts_with("<!--") || l.is_empty())
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                })
                            })
                            .filter(|s| !s.trim().is_empty())
                    } else {
                        None
                    };

                    let mut content = generate_krab_from_exploration(
                        &project_name,
                        languages,
                        &exploration_result,
                    );

                    if let Some(notes) = preserved_notes {
                        content.push_str(&notes);
                        content.push('\n');
                    }

                    match std::fs::write(&krab_path, &content) {
                        Ok(_) => {
                            let action = if is_regenerate {
                                "Regenerated"
                            } else {
                                "Created"
                            };
                            result = result.with_message(
                                "assistant",
                                format!(
                                    "{} **KRAB.md** ({} bytes) from codebase analysis.\n\n\
                                    This file is now auto-injected into every AI conversation. \
                                    Edit it to customize how I understand your project.",
                                    action,
                                    content.len()
                                ),
                            );

                            // Store exploration results as insights
                            result = result.with_action(PollAction::StoreInitInsights {
                                architecture: exploration_result.architecture.clone(),
                                conventions: exploration_result.conventions.clone(),
                                key_files: exploration_result.key_files.clone(),
                                build_system: exploration_result.build_system,
                            });
                        }
                        Err(e) => {
                            result = result.with_message(
                                "assistant",
                                format!("Failed to write KRAB.md: {}", e),
                            );
                        }
                    }
                } else {
                    let error = exploration_result
                        .error
                        .unwrap_or_else(|| "Unknown error".to_string());
                    result =
                        result.with_message("assistant", format!("Exploration failed: {}", error));
                }
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                channels.init_exploration = Some(rx);
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                *init_explore_id = None;
                *cached_languages = None; // Clear language cache on cancellation
                result = result.with_message("assistant", "Exploration was cancelled.");
            }
        }
    }

    result
}

/// Poll /init indexing progress and update ExploreBlock
pub fn poll_indexing_progress(
    channels: &mut AsyncChannels,
    explore_blocks: &mut [ExploreBlock],
    init_explore_id: &Option<String>,
) -> PollResult {
    let mut result = PollResult::new();

    let Some(mut rx) = channels.indexing_progress.take() else {
        return result;
    };

    loop {
        match rx.try_recv() {
            Ok(progress) => {
                result.needs_redraw = true;
                // Update the init ExploreBlock with indexing progress
                if let Some(ref explore_id) = init_explore_id {
                    for block in explore_blocks.iter_mut() {
                        if block.tool_use_id() == Some(explore_id.as_str()) {
                            block.update_indexing_progress(progress.clone());
                            break;
                        }
                    }
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                channels.indexing_progress = Some(rx);
                break;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                tracing::debug!("Indexing progress channel disconnected");
                // Clear indexing progress when channel closes (indexing complete)
                if let Some(ref explore_id) = init_explore_id {
                    for block in explore_blocks.iter_mut() {
                        if block.tool_use_id() == Some(explore_id.as_str()) {
                            // Signal completion by clearing the progress
                            block.update_indexing_progress(krusty_core::index::IndexProgress {
                                phase: krusty_core::index::IndexPhase::Complete,
                                current: 0,
                                total: 0,
                                current_file: None,
                            });
                            break;
                        }
                    }
                }
                break;
            }
        }
    }

    result
}
