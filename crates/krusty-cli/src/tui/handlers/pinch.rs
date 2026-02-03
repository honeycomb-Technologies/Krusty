//! Pinch orchestration
//!
//! Handles the multi-stage pinch flow:
//! 1. User provides preservation hints
//! 2. AI summarizes conversation (async, using Sonnet 4.5 + extended thinking)
//! 3. User provides direction for next phase
//! 4. New linked session is created

use std::path::PathBuf;

use crate::agent::{generate_summary, PinchContext, SummarizationResult};
use crate::ai::client::AiClient;
use crate::storage::{FileActivityTracker, RankedFile};
use crate::tui::app::App;
use crate::tui::utils::{SummarizationUpdate, TitleUpdate};

impl App {
    /// Start the summarization phase of pinch
    ///
    /// Spawns an async task that:
    /// 1. Reads key file contents based on activity ranking
    /// 2. Reads project instruction files for context
    /// 3. Sends full conversation + context for extended thinking
    /// 4. Returns structured summary for user review
    pub fn start_pinch_summarization(&mut self) {
        // Move popup to summarizing state
        self.popups.pinch.start_summarizing();

        // Get preservation hints from first stage
        let preservation_hints = self
            .popups
            .pinch
            .get_preservation_input()
            .map(|s| s.to_string());

        // Get ranked files for context
        let ranked_files = self.get_ranked_files_for_summarization();

        // Read key file contents (top 10 by importance)
        let file_contents = self.read_key_file_contents(&ranked_files);

        // Read project context from instruction files
        let project_context = self.read_project_context();

        // Clone conversation for the async task
        let conversation = self.chat.conversation.clone();

        // Capture current model for summarization
        let current_model = self.current_model.clone();

        // Create AI client for summarization
        let client = match self.create_summarization_client() {
            Some(c) => c,
            None => {
                self.popups
                    .pinch
                    .set_error("No AI client available for summarization".to_string());
                return;
            }
        };

        // Set up channel for results
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.channels.summarization = Some(rx);

        // Log before moving into async block
        let msg_count = conversation.len();
        let file_count = file_contents.len();

        // Spawn async summarization task
        tokio::spawn(async move {
            let result = generate_summary(
                &client,
                &conversation,
                preservation_hints.as_deref(),
                &ranked_files,
                &file_contents,
                project_context.as_deref(),
                Some(&current_model),
            )
            .await;

            let update = SummarizationUpdate {
                result: result.map_err(|e| e.to_string()),
            };
            let _ = tx.send(update);
        });

        tracing::info!(
            "Started async summarization with {} messages, {} file contents",
            msg_count,
            file_count
        );
    }

    /// Read contents of top-ranked files for summarization context
    fn read_key_file_contents(&self, ranked_files: &[RankedFile]) -> Vec<(String, String)> {
        ranked_files
            .iter()
            .take(10)
            .filter_map(|file| {
                let path = if std::path::Path::new(&file.path).is_absolute() {
                    PathBuf::from(&file.path)
                } else {
                    self.working_dir.join(&file.path)
                };
                std::fs::read_to_string(&path)
                    .ok()
                    .map(|content| (file.path.clone(), content))
            })
            .collect()
    }

    /// Read project context from instruction files
    fn read_project_context(&self) -> Option<String> {
        // Support common AI coding assistant instruction file formats
        const PROJECT_FILES: &[&str] = &[
            "KRAB.md",
            "krab.md",
            "AGENTS.md",
            "agents.md",
            "CLAUDE.md",
            "claude.md",
            ".cursorrules",
            ".windsurfrules",
            ".clinerules",
            ".github/copilot-instructions.md",
            "JULES.md",
            "gemini.md",
        ];
        for filename in PROJECT_FILES {
            let path = self.working_dir.join(filename);
            if let Ok(content) = std::fs::read_to_string(&path) {
                tracing::debug!("Loaded project context from {}", filename);
                return Some(content);
            }
        }
        None
    }

    /// Create AI client for summarization
    fn create_summarization_client(&self) -> Option<AiClient> {
        self.create_ai_client()
    }

    /// Poll for summarization results
    pub fn poll_summarization(&mut self) {
        let rx = match self.channels.summarization.as_mut() {
            Some(rx) => rx,
            None => return,
        };

        match rx.try_recv() {
            Ok(update) => {
                self.channels.summarization = None;

                match update.result {
                    Ok(summary) => {
                        tracing::info!(
                            "Summarization complete: {}",
                            &summary.work_summary[..100.min(summary.work_summary.len())]
                        );

                        // Show summary in popup and move to direction input
                        self.popups.pinch.show_summary(
                            summary.work_summary.clone(),
                            summary.important_files.clone(),
                        );

                        // Store the full result for use when completing pinch
                        self.popups.pinch.set_summarization_result(summary);
                    }
                    Err(e) => {
                        tracing::error!("Summarization failed: {}", e);
                        self.popups
                            .pinch
                            .set_error(format!("Summarization failed: {}", e));
                    }
                }
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                // Still summarizing
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                // Task failed/cancelled
                self.channels.summarization = None;
                self.popups
                    .pinch
                    .set_error("Summarization task cancelled".to_string());
            }
        }
    }

    /// Get ranked files for summarization context
    fn get_ranked_files_for_summarization(&self) -> Vec<RankedFile> {
        if let (Some(sm), Some(session_id)) =
            (&self.services.session_manager, &self.current_session_id)
        {
            let tracker = FileActivityTracker::new(sm.db(), session_id.clone());
            return tracker.get_ranked_files(20).unwrap_or_default();
        }
        Vec::new()
    }

    /// Start auto-pinch (bypasses popup, used when AI is working autonomously)
    ///
    /// Directly starts summarization without popup interaction.
    pub fn start_auto_pinch(&mut self) {
        self.auto_pinch_in_progress = true;

        let ranked_files = self.get_ranked_files_for_summarization();
        let file_contents = self.read_key_file_contents(&ranked_files);
        let project_context = self.read_project_context();
        let conversation = self.chat.conversation.clone();
        let current_model = self.current_model.clone();

        let client = match self.create_summarization_client() {
            Some(c) => c,
            None => {
                tracing::error!("Auto-pinch: no AI client for summarization");
                self.auto_pinch_in_progress = false;
                return;
            }
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.channels.summarization = Some(rx);

        let msg_count = conversation.len();
        let file_count = file_contents.len();

        tokio::spawn(async move {
            let result = generate_summary(
                &client,
                &conversation,
                None, // no preservation hints in auto mode
                &ranked_files,
                &file_contents,
                project_context.as_deref(),
                Some(&current_model),
            )
            .await;

            let update = SummarizationUpdate {
                result: result.map_err(|e| e.to_string()),
            };
            let _ = tx.send(update);
        });

        tracing::info!(
            "Auto-pinch: started summarization with {} messages, {} file contents",
            msg_count,
            file_count
        );
    }

    /// Poll auto-pinch summarization and complete when ready
    pub fn poll_auto_pinch(&mut self) {
        if !self.auto_pinch_in_progress {
            return;
        }

        let rx = match self.channels.summarization.as_mut() {
            Some(rx) => rx,
            None => return,
        };

        match rx.try_recv() {
            Ok(update) => {
                self.channels.summarization = None;
                match update.result {
                    Ok(summary) => {
                        tracing::info!(
                            "Auto-pinch: summarization complete: {}",
                            &summary.work_summary[..100.min(summary.work_summary.len())]
                        );
                        self.complete_auto_pinch(summary);
                    }
                    Err(e) => {
                        tracing::error!("Auto-pinch: summarization failed: {}", e);
                        self.auto_pinch_in_progress = false;
                        self.chat
                            .messages
                            .push(("system".to_string(), format!("Auto-pinch failed: {}", e)));
                    }
                }
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                // Still summarizing
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.channels.summarization = None;
                self.auto_pinch_in_progress = false;
                tracing::error!("Auto-pinch: summarization task cancelled");
            }
        }
    }

    /// Complete auto-pinch: create linked session and resume AI
    fn complete_auto_pinch(&mut self, summary_result: SummarizationResult) {
        let ranked_files = self.get_ranked_files_for_summarization();
        let project_context = self.read_project_context();
        let key_file_contents = self
            .read_key_file_contents(&ranked_files)
            .into_iter()
            .take(5)
            .collect();
        let active_plan = self.active_plan.as_ref().map(|p| p.to_markdown());

        // Extract summary text before consuming summary_result
        let summary_text = summary_result.work_summary.clone();

        let pinch_ctx = PinchContext::new(
            self.current_session_id.clone().unwrap_or_default(),
            self.session_title
                .clone()
                .unwrap_or_else(|| "Untitled".to_string()),
            summary_result,
            ranked_files,
            None, // no preservation hints
            None, // no direction — auto-continue
            project_context,
            key_file_contents,
            active_plan,
        );

        let Some(sm) = &self.services.session_manager else {
            tracing::error!("Auto-pinch: no session manager");
            self.auto_pinch_in_progress = false;
            return;
        };

        let Some(parent_id) = &self.current_session_id else {
            tracing::error!("Auto-pinch: no current session");
            self.auto_pinch_in_progress = false;
            return;
        };

        let parent_title = self
            .session_title
            .clone()
            .unwrap_or_else(|| "Session".to_string());
        let fallback_title = format!(
            "{} (cont.)",
            &parent_title.chars().take(45).collect::<String>()
        );

        match sm.create_linked_session(
            &fallback_title,
            parent_id,
            &pinch_ctx,
            Some(&self.current_model),
            Some(&self.working_dir.to_string_lossy()),
        ) {
            Ok(new_id) => {
                // Save pinch context as first message
                let system_msg = pinch_ctx.to_system_message();
                if let Err(e) = sm.save_message(&new_id, "system", &system_msg) {
                    tracing::warn!("Auto-pinch: failed to save pinch message: {}", e);
                }

                // Carry over active plan
                if let Some(ref plan) = self.active_plan {
                    if let Err(e) = self
                        .services
                        .plan_manager
                        .save_plan_for_session(&new_id, plan)
                    {
                        tracing::warn!("Auto-pinch: failed to carry over plan: {}", e);
                    }
                }

                // Save "Continue working on the current task." as user message
                let content_json = serde_json::to_string(&vec![crate::ai::types::Content::Text {
                    text: "Continue working on the current task.".to_string(),
                }])
                .unwrap_or_else(|_| "[\"Continue working on the current task.\"]".to_string());

                if let Err(e) = sm.save_message(&new_id, "user", &content_json) {
                    tracing::warn!("Auto-pinch: failed to save continue message: {}", e);
                }

                // Spawn title generation
                self.spawn_pinch_title_generation(new_id.clone(), parent_title, summary_text, None);

                // Load the new session and resume
                self.save_block_ui_states();
                if let Err(e) = self.load_session(&new_id) {
                    tracing::error!("Auto-pinch: failed to load new session: {}", e);
                    self.auto_pinch_in_progress = false;
                    return;
                }

                tracing::info!(
                    "Auto-pinch: complete, resuming AI in new session {}",
                    new_id
                );
                self.auto_pinch_in_progress = false;
                self.send_to_ai();
            }
            Err(e) => {
                tracing::error!("Auto-pinch: failed to create session: {}", e);
                self.auto_pinch_in_progress = false;
                self.chat
                    .messages
                    .push(("system".to_string(), format!("Auto-pinch failed: {}", e)));
            }
        }
    }

    /// Complete the pinch by creating a linked session
    pub fn complete_pinch(&mut self) {
        use crate::tui::popups::pinch::PinchStage;

        // Get summary from popup stage
        let summary = match &self.popups.pinch.stage {
            PinchStage::DirectionInput { summary, .. } => summary.clone(),
            _ => return,
        };

        let direction = self
            .popups
            .pinch
            .get_direction_input()
            .map(|s| s.to_string());
        let preservation_hints = self
            .popups
            .pinch
            .get_preservation_input()
            .map(|s| s.to_string());

        // Move to creating state
        self.popups.pinch.start_creating();

        // Get the full AI summarization result (includes key_decisions, pending_tasks, etc.)
        let summary_result = self
            .popups
            .pinch
            .get_summarization_result()
            .cloned()
            .unwrap_or_else(|| SummarizationResult {
                work_summary: summary.clone(),
                key_decisions: Vec::new(),
                pending_tasks: Vec::new(),
                important_files: Vec::new(),
            });

        // Build pinch context with FULL context for continuation
        let ranked_files = self.get_ranked_files_for_summarization();

        // Read project context - CRITICAL for continuation!
        let project_context = self.read_project_context();

        // Read top 5 key file contents for context
        let key_file_contents = self
            .read_key_file_contents(&ranked_files)
            .into_iter()
            .take(5)
            .collect();

        // Get active plan markdown if one exists
        let active_plan = self.active_plan.as_ref().map(|p| p.to_markdown());

        let pinch_ctx = PinchContext::new(
            self.current_session_id.clone().unwrap_or_default(),
            self.session_title
                .clone()
                .unwrap_or_else(|| "Untitled".to_string()),
            summary_result,
            ranked_files,
            preservation_hints,
            direction.clone(),
            project_context,
            key_file_contents,
            active_plan,
        );

        // Create linked session
        let Some(sm) = &self.services.session_manager else {
            self.popups
                .pinch
                .set_error("No session manager".to_string());
            return;
        };

        let Some(parent_id) = &self.current_session_id else {
            self.popups
                .pinch
                .set_error("No current session".to_string());
            return;
        };

        // Use fallback title initially, spawn AI generation
        let parent_title = self
            .session_title
            .clone()
            .unwrap_or_else(|| "Session".to_string());
        let fallback_title = format!(
            "{} (cont.)",
            &parent_title.chars().take(45).collect::<String>()
        );

        match sm.create_linked_session(
            &fallback_title,
            parent_id,
            &pinch_ctx,
            Some(&self.current_model),
            Some(&self.working_dir.to_string_lossy()),
        ) {
            Ok(new_id) => {
                // Save pinch context as first message
                let system_msg = pinch_ctx.to_system_message();
                if let Err(e) = sm.save_message(&new_id, "system", &system_msg) {
                    tracing::warn!("Failed to save pinch message: {}", e);
                }

                // Carry over active plan to new session
                if let Some(ref plan) = self.active_plan {
                    if let Err(e) = self
                        .services
                        .plan_manager
                        .save_plan_for_session(&new_id, plan)
                    {
                        tracing::warn!("Failed to carry over plan to new session: {}", e);
                    } else {
                        tracing::info!("Carried over plan '{}' to pinched session", plan.title);
                    }
                }

                // If direction provided, save it as user message
                // Auto-continue always (pinch implies continuation)
                let has_direction = direction
                    .as_ref()
                    .map(|d| !d.trim().is_empty())
                    .unwrap_or(false);

                if has_direction {
                    if let Some(dir) = &direction {
                        // Save as JSON content array to match normal message format
                        let content_json =
                            serde_json::to_string(&vec![crate::ai::types::Content::Text {
                                text: dir.clone(),
                            }])
                            .unwrap_or_else(|_| format!("[\"{}\"]", dir));

                        if let Err(e) = sm.save_message(&new_id, "user", &content_json) {
                            tracing::warn!("Failed to save direction as user message: {}", e);
                        }
                    }
                } else {
                    // No direction - save a default "Continue" prompt
                    let content_json =
                        serde_json::to_string(&vec![crate::ai::types::Content::Text {
                            text: "Continue.".to_string(),
                        }])
                        .unwrap_or_else(|_| "[\"Continue.\"]".to_string());

                    if let Err(e) = sm.save_message(&new_id, "user", &content_json) {
                        tracing::warn!("Failed to save default continue message: {}", e);
                    }
                }

                // Always auto-continue after pinch (user explicitly chose to continue)
                let auto_continue = true;

                // Spawn async AI title generation
                self.spawn_pinch_title_generation(new_id.clone(), parent_title, summary, direction);

                // Show completion - auto_continue triggers AI response after switch
                self.popups
                    .pinch
                    .complete(new_id, fallback_title, auto_continue);
            }
            Err(e) => {
                self.popups
                    .pinch
                    .set_error(format!("Failed to create session: {}", e));
            }
        }
    }

    /// Spawn background task to generate AI title for pinch session
    fn spawn_pinch_title_generation(
        &mut self,
        session_id: String,
        parent_title: String,
        summary: String,
        direction: Option<String>,
    ) {
        let client = match self.create_pinch_title_client() {
            Some(c) => c,
            None => {
                tracing::debug!("No AI client available for pinch title generation");
                return;
            }
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.channels.title_update = Some(rx);

        tokio::spawn(async move {
            let title = crate::ai::generate_pinch_title(
                &client,
                &parent_title,
                &summary,
                direction.as_deref(),
            )
            .await;
            let _ = tx.send(TitleUpdate { session_id, title });
        });
    }

    /// Create AI client for pinch title generation
    fn create_pinch_title_client(&self) -> Option<AiClient> {
        self.create_ai_client()
    }
}
