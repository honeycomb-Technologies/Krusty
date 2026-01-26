//! Event loop polling and tick handlers
//!
//! Poll operations and animation ticks extracted from app.rs.

use crate::tui::app::{App, View};
use crate::tui::polling::{
    poll_bash_output, poll_build_progress, poll_dual_mind, poll_explore_progress, PollAction,
    PollResult,
};

impl App {
    /// Poll bash output channel and update BashBlock with streaming output
    pub(crate) fn poll_bash_output(&mut self) -> PollResult {
        poll_bash_output(
            &mut self.channels,
            &mut self.blocks.bash,
            &mut self.scroll_system.scroll,
            &self.process_registry,
        )
    }

    /// Poll explore progress channel and update ExploreBlock with agent progress
    pub(crate) fn poll_explore_progress(&mut self) -> PollResult {
        poll_explore_progress(&mut self.channels, &mut self.blocks.explore)
    }

    /// Poll build progress channel and update BuildBlock with builder progress
    pub(crate) fn poll_build_progress(&mut self) -> PollResult {
        poll_build_progress(
            &mut self.channels,
            &mut self.blocks.build,
            &mut self.active_plan,
            &self.services.plan_manager,
        )
    }

    /// Poll dual-mind dialogue channel for Big Claw / Little Claw updates
    pub(crate) fn poll_dual_mind(&mut self) -> PollResult {
        poll_dual_mind(&mut self.channels)
    }

    /// Poll terminal panes for PTY output and update cursor animations
    pub(crate) fn poll_terminal_panes(&mut self) {
        self.blocks.poll_terminals();
    }

    /// Process actions returned from polling operations
    pub(crate) fn process_poll_actions(&mut self, result: PollResult) {
        // Add messages
        for (role, content) in result.messages {
            self.chat.messages.push((role, content));
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
            }
        }
    }

    /// Tick all animations. Returns true if any animation is still running.
    pub(crate) fn tick_blocks(&mut self) -> bool {
        let blocks = self.blocks.tick_all();
        self.popups.pinch.tick();
        let sidebar = self.plan_sidebar.tick();
        let plugin_window = self.plugin_window.tick();

        if self.plan_sidebar.should_clear_plan() {
            self.active_plan = None;
            tracing::info!("Plan cleared after sidebar collapse");
        }

        use crate::tui::popups::pinch::PinchStage;
        let pinch_active = matches!(
            self.popups.pinch.stage,
            PinchStage::Summarizing { .. } | PinchStage::Creating
        );

        blocks || sidebar || plugin_window || pinch_active || self.ui.view == View::StartMenu
    }
}
