//! Event loop polling and tick handlers
//!
//! Poll operations and animation ticks extracted from app.rs.

use crate::tui::app::{App, View};
use crate::tui::polling::{
    poll_bash_output, poll_build_progress, poll_dual_mind, poll_explore_progress, PollAction,
    PollResult,
};

/// Split exploration text into individual insight paragraphs.
/// Keeps bullet groups together as single insights — only splits on paragraph boundaries.
fn split_into_insights(text: &str) -> Vec<String> {
    let cleaned: String = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("## ") && !trimmed.starts_with("### ")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut insights = Vec::new();

    for section in cleaned.split("\n\n") {
        let trimmed = section.trim();
        if trimmed.len() >= 30 {
            insights.push(trimmed.to_string());
        }
    }

    insights.truncate(10);
    insights
}

impl App {
    /// Poll bash output channel and update BashBlock with streaming output
    pub(crate) fn poll_bash_output(&mut self) -> PollResult {
        poll_bash_output(
            &mut self.runtime.channels,
            &mut self.runtime.blocks.bash,
            &mut self.ui.scroll_system.scroll,
            &self.runtime.process_registry,
        )
    }

    /// Poll explore progress channel and update ExploreBlock with agent progress
    pub(crate) fn poll_explore_progress(&mut self) -> PollResult {
        poll_explore_progress(&mut self.runtime.channels, &mut self.runtime.blocks.explore)
    }

    /// Poll build progress channel and update BuildBlock with builder progress
    pub(crate) fn poll_build_progress(&mut self) -> PollResult {
        poll_build_progress(
            &mut self.runtime.channels,
            &mut self.runtime.blocks.build,
            &mut self.runtime.active_plan,
            &self.services.plan_manager,
        )
    }

    /// Poll dual-mind dialogue channel for Big Claw / Little Claw updates
    pub(crate) fn poll_dual_mind(&mut self) -> PollResult {
        let (result, extracted_insights) = poll_dual_mind(&mut self.runtime.channels);

        // Save extracted insights if we have database access and a codebase
        if let Some(insights) = extracted_insights {
            if let (Some(sm), Some(session_id)) = (
                &self.services.session_manager,
                &self.runtime.current_session_id,
            ) {
                let conn = sm.db().conn();
                // Get codebase_id for current working directory
                let working_dir_str = self.runtime.working_dir.to_string_lossy().to_string();
                if let Ok(Some(codebase)) =
                    krusty_core::index::CodebaseStore::new(conn).get_by_path(&working_dir_str)
                {
                    let insight_store = krusty_core::index::InsightStore::new(conn);

                    for content in &insights.insights {
                        // Check for duplicates before saving
                        match insight_store.has_similar(&codebase.id, content) {
                            Ok(false) => {
                                let insight = krusty_core::index::insights::create_insight(
                                    &codebase.id,
                                    content,
                                    Some(session_id),
                                    0.6,
                                    None,
                                );
                                if let Err(e) = insight_store.create(&insight) {
                                    tracing::warn!(error = %e, "Failed to save insight");
                                } else {
                                    tracing::info!(
                                        insight_type = ?insight.insight_type,
                                        "Saved new codebase insight from review"
                                    );
                                }
                            }
                            Ok(true) => {
                                tracing::debug!(content = %content, "Skipping duplicate insight");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Failed to check for duplicate insight");
                            }
                        }
                    }
                }
            }
        }

        result
    }

    /// Poll terminal panes for PTY output and update cursor animations
    pub(crate) fn poll_terminal_panes(&mut self) {
        self.runtime.blocks.poll_terminals();
    }

    /// Process actions returned from polling operations
    pub(crate) fn process_poll_actions(&mut self, result: PollResult) {
        // Add messages
        for (role, content) in result.messages {
            self.runtime.chat.messages.push((role, content));
        }

        // Execute actions
        for action in result.actions {
            match action {
                PollAction::RefreshMcpPopup => {
                    self.refresh_mcp_popup();
                }
                PollAction::RefreshAiTools => {
                    self.services.cached_ai_tools =
                        futures::executor::block_on(self.services.tool_registry.get_ai_tools());
                    tracing::info!(
                        "Refreshed AI tools after MCP update, total: {}",
                        self.services.cached_ai_tools.len()
                    );
                }
                PollAction::SwitchProvider(provider) => {
                    self.switch_provider(provider);
                }
                PollAction::StoreInitInsights {
                    architecture,
                    conventions,
                    key_files,
                    build_system,
                } => {
                    self.store_init_insights(
                        &architecture,
                        &conventions,
                        &key_files,
                        &build_system,
                    );
                }
            }
        }
    }

    /// Store /init exploration results as codebase insights
    fn store_init_insights(
        &self,
        architecture: &str,
        conventions: &str,
        key_files: &str,
        build_system: &str,
    ) {
        let Some(sm) = &self.services.session_manager else {
            return;
        };

        let conn = sm.db().conn();
        let working_dir_str = self.runtime.working_dir.to_string_lossy().to_string();

        let codebase =
            match krusty_core::index::CodebaseStore::new(conn).get_by_path(&working_dir_str) {
                Ok(Some(c)) => c,
                Ok(None) => {
                    tracing::debug!("No codebase entry found for /init insights");
                    return;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to look up codebase for /init insights");
                    return;
                }
            };

        let insight_store = krusty_core::index::InsightStore::new(conn);

        // Clear previous /init insights (keeps dual-mind insights at confidence 0.6)
        match insight_store.delete_by_confidence_above(&codebase.id, 0.75) {
            Ok(deleted) if deleted > 0 => {
                tracing::info!(
                    count = deleted,
                    "Purged stale /init insights before re-indexing"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to purge stale /init insights");
            }
            _ => {}
        }

        let session_id = self.runtime.current_session_id.as_deref();
        let mut stored = 0;

        for (content, label) in [
            (architecture, "architecture"),
            (conventions, "conventions"),
            (key_files, "key_files"),
            (build_system, "build_system"),
        ] {
            if content.is_empty() {
                continue;
            }

            let (insight_type, confidence) = match label {
                "architecture" => (
                    krusty_core::index::insights::InsightType::Architecture,
                    0.90,
                ),
                "conventions" => (krusty_core::index::insights::InsightType::Convention, 0.85),
                "key_files" => (
                    krusty_core::index::insights::InsightType::Architecture,
                    0.80,
                ),
                "build_system" => (krusty_core::index::insights::InsightType::Dependency, 0.85),
                _ => unreachable!(),
            };

            for paragraph in split_into_insights(content) {
                match insight_store.has_similar(&codebase.id, &paragraph) {
                    Ok(false) => {
                        let insight = krusty_core::index::insights::create_insight(
                            &codebase.id,
                            &paragraph,
                            session_id,
                            confidence,
                            Some(insight_type),
                        );
                        if let Err(e) = insight_store.create(&insight) {
                            tracing::warn!(error = %e, "Failed to save /init insight");
                        } else {
                            stored += 1;
                        }
                    }
                    Ok(true) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to check for duplicate /init insight");
                    }
                }
            }
        }

        if stored > 0 {
            tracing::info!(count = stored, "Stored /init exploration insights");
        }
    }

    /// Tick all animations. Returns true if any animation is still running.
    pub(crate) fn tick_blocks(&mut self) -> bool {
        let blocks = self.runtime.blocks.tick_all();
        self.ui.popups.pinch.tick();
        let sidebar = self.ui.plan_sidebar.tick();
        let plugin_window = self.ui.plugin_window.tick();

        if self.ui.plan_sidebar.should_clear_plan() {
            self.clear_plan();
            tracing::info!("Plan cleared after sidebar collapse");
        }

        use crate::tui::popups::pinch::PinchStage;
        let pinch_active = matches!(
            self.ui.popups.pinch.stage,
            PinchStage::Summarizing { .. } | PinchStage::Creating
        );

        blocks || sidebar || plugin_window || pinch_active || self.ui.view == View::StartMenu
    }
}
