//! Block Manager - Centralized management for all stream blocks
//!
//! This module owns all block types and provides a unified interface for:
//! - Block lifecycle (ticking, completion)
//! - Terminal management (closing)

use crate::tui::blocks::{
    build::BuildBlock, BashBlock, DiffMode, EditBlock, ExploreBlock, PinchBlock, ReadBlock,
    StreamBlock, TerminalPane, ThinkingBlock, ToolResultBlock, WebSearchBlock, WriteBlock,
};

/// Manages all block types in the TUI
pub struct BlockManager {
    // Block collections
    pub thinking: Vec<ThinkingBlock>,
    pub pinch: Vec<PinchBlock>,
    pub bash: Vec<BashBlock>,
    pub terminal: Vec<TerminalPane>,
    pub tool_result: Vec<ToolResultBlock>,
    pub read: Vec<ReadBlock>,
    pub edit: Vec<EditBlock>,
    pub write: Vec<WriteBlock>,
    pub web_search: Vec<WebSearchBlock>,
    pub explore: Vec<ExploreBlock>,
    pub build: Vec<BuildBlock>,

    // Terminal state
    pub focused_terminal: Option<usize>,
    pub pinned_terminal: Option<usize>,

    // Global settings
    pub diff_mode: DiffMode,
}

impl BlockManager {
    /// Create a new empty block manager
    pub fn new() -> Self {
        Self {
            thinking: Vec::new(),
            pinch: Vec::new(),
            bash: Vec::new(),
            terminal: Vec::new(),
            tool_result: Vec::new(),
            read: Vec::new(),
            edit: Vec::new(),
            write: Vec::new(),
            web_search: Vec::new(),
            explore: Vec::new(),
            build: Vec::new(),
            focused_terminal: None,
            pinned_terminal: None,
            diff_mode: DiffMode::Unified,
        }
    }

    /// Get total count of all blocks (for capacity estimation)
    pub fn total_count(&self) -> usize {
        self.thinking.len()
            + self.pinch.len()
            + self.bash.len()
            + self.terminal.len()
            + self.tool_result.len()
            + self.read.len()
            + self.edit.len()
            + self.write.len()
            + self.web_search.len()
            + self.explore.len()
            + self.build.len()
    }

    /// Tick all animation blocks. Returns true if any block is still animating.
    pub fn tick_all(&mut self) -> bool {
        let mut animating = false;
        for block in &mut self.thinking {
            animating |= block.tick();
        }
        for block in &mut self.pinch {
            animating |= block.tick();
        }
        for block in &mut self.bash {
            animating |= block.tick();
        }
        for block in &mut self.tool_result {
            animating |= block.tick();
        }
        for block in &mut self.read {
            animating |= block.tick();
        }
        for block in &mut self.edit {
            animating |= block.tick();
        }
        for block in &mut self.write {
            animating |= block.tick();
        }
        for block in &mut self.web_search {
            animating |= block.tick();
        }
        for block in &mut self.explore {
            animating |= block.tick();
        }
        for block in &mut self.build {
            animating |= block.tick();
        }
        animating
    }

    /// Poll terminal panes for PTY output
    pub fn poll_terminals(&mut self) {
        for pane in &mut self.terminal {
            pane.poll();
            pane.tick();
        }
    }

    /// Clear focus from all terminal panes (prevents state divergence)
    ///
    /// This ensures both the focused_terminal index AND the individual
    /// is_focused flags on each pane are cleared together.
    pub fn clear_all_terminal_focus(&mut self) {
        for pane in &mut self.terminal {
            pane.set_focused(false);
        }
        self.focused_terminal = None;
    }

    /// Set focus to a specific terminal by index
    ///
    /// Clears focus from all other terminals first to prevent divergence.
    pub fn focus_terminal(&mut self, idx: usize) {
        self.clear_all_terminal_focus();
        if let Some(pane) = self.terminal.get_mut(idx) {
            pane.set_focused(true);
            self.focused_terminal = Some(idx);
        }
    }

    /// Close a terminal pane by index, returns process_id for deregistration
    pub fn close_terminal(&mut self, idx: usize) -> Option<String> {
        // Clear focus if this terminal was focused
        if self.focused_terminal == Some(idx) {
            self.focused_terminal = None;
        } else if let Some(focused) = self.focused_terminal {
            if focused > idx {
                self.focused_terminal = Some(focused - 1);
            }
        }

        // Clear or adjust pinned terminal index
        if self.pinned_terminal == Some(idx) {
            self.pinned_terminal = None;
        } else if let Some(pinned) = self.pinned_terminal {
            if pinned > idx {
                self.pinned_terminal = Some(pinned - 1);
            }
        }

        // Get process ID before removing
        let process_id = if idx < self.terminal.len() {
            self.terminal[idx].get_process_id().map(|s| s.to_string())
        } else {
            None
        };

        // Remove the terminal pane
        if idx < self.terminal.len() {
            self.terminal.remove(idx);
        }

        process_id
    }
}

impl Default for BlockManager {
    fn default() -> Self {
        Self::new()
    }
}
