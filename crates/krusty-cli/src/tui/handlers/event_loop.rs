//! Event loop polling and tick handlers
//!
//! Poll operations and animation ticks extracted from app.rs.

use crate::tui::app::{App, Popup, View};
use crate::tui::blocks::StreamBlock;
use crate::tui::polling::{
    poll_bash_output, poll_build_progress, poll_delegated_progress, poll_explore_progress,
    PollAction, PollResult,
};
use std::time::{Duration, Instant};

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
            self.services.plan_manager.as_ref(),
        )
    }

    /// Poll delegated agent progress emitted by the orchestrator.
    pub(crate) fn poll_delegated_progress(&mut self) -> PollResult {
        poll_delegated_progress(
            &mut self.runtime.channels,
            &mut self.runtime.blocks.explore,
            &mut self.runtime.blocks.build,
        )
    }

    /// Poll terminal panes for PTY output and update cursor animations
    pub(crate) fn poll_terminal_panes(&mut self) {
        self.runtime.blocks.poll_terminals();
    }

    /// Poll git status on an interval for status bar updates.
    pub(crate) fn poll_git_status(&mut self) {
        const GIT_POLL_INTERVAL: Duration = Duration::from_secs(2);

        let now = Instant::now();
        if now.duration_since(self.runtime.last_git_status_poll) < GIT_POLL_INTERVAL {
            return;
        }
        self.runtime.last_git_status_poll = now;

        match krusty_core::git::status(&self.runtime.working_dir) {
            Ok(new_status) => {
                if self.runtime.git_status.as_ref() != new_status.as_ref() {
                    self.runtime.git_status = new_status;
                    self.ui.needs_redraw = true;
                }
            }
            Err(err) => {
                tracing::debug!("Failed to poll git status: {}", err);
                if self.runtime.git_status.take().is_some() {
                    self.ui.needs_redraw = true;
                }
            }
        }
    }

    /// Poll plugin catalog on an interval for install/update/enable state changes.
    pub(crate) fn poll_plugin_catalog(&mut self) {
        const PLUGIN_POLL_INTERVAL: Duration = Duration::from_secs(2);

        let now = Instant::now();
        if now.duration_since(self.runtime.last_plugin_catalog_poll) < PLUGIN_POLL_INTERVAL {
            return;
        }
        self.runtime.last_plugin_catalog_poll = now;

        if self.refresh_plugin_catalog(true) {
            self.ui.needs_redraw = true;
        }
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
                PollAction::RefreshDynamicModels(provider) => {
                    // Force a refresh after new credentials (ignore cache freshness).
                    self.runtime.dynamic_model_fetches.remove(&provider);
                    self.start_dynamic_model_fetch(provider);
                }
            }
        }
    }

    /// Tick all animations. Returns true if any animation is still running.
    pub(crate) fn tick_blocks(&mut self) -> bool {
        let blocks = self.runtime.blocks.tick_all();
        let sidebar = self.ui.plan_sidebar.tick();
        let plugin_window = self.ui.plugin_window.tick();

        if self.ui.plan_sidebar.should_clear_plan() {
            self.clear_plan();
            tracing::info!("Plan cleared after sidebar collapse");
        }

        let pinch_active = self
            .runtime
            .blocks
            .pinch
            .iter()
            .any(|block| block.is_streaming());
        let auth_browser_waiting =
            self.ui.popup == Popup::Auth && self.ui.popups.auth.is_browser_waiting();

        blocks
            || sidebar
            || plugin_window
            || pinch_active
            || auth_browser_waiting
            || self.ui.view == View::StartMenu
    }
}
