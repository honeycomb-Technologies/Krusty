//! Stream blocks - modular widgets for chat stream rendering
//!
//! Each block type implements the StreamBlock trait and handles its own
//! rendering, interaction, and state management.
//!
//! Blocks support partial visibility via ClipContext - when scrolled partially
//! off-screen, they receive clip info to render borders correctly.

pub mod bash;
pub mod build;
pub mod edit;
pub mod explore;
pub mod pinch;
pub mod read;
pub mod terminal_pane;
pub mod thinking;
pub mod tool_result;
pub mod web_search;
pub mod write;

use crossterm::event::Event;
use ratatui::{buffer::Buffer, layout::Rect};

use crate::tui::themes::Theme;

/// Clipping context for partially visible blocks
///
/// When a block is scrolled partially off-screen, this tells it which
/// portions are clipped so it can skip drawing borders appropriately.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClipContext {
    /// Lines clipped from block's top (0 = fully visible from top)
    pub clip_top: u16,
    /// Lines clipped from block's bottom (0 = fully visible at bottom)
    pub clip_bottom: u16,
}

/// Types of blocks that can be hit-tested
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    Thinking,
    Bash,
    ToolResult,
    Read,
    Edit,
    Write,
    TerminalPane,
    WebSearch,
    Explore,
    Build,
    Pinch,
}

/// Result of a block hit test
#[derive(Debug, Clone)]
pub struct BlockHitResult {
    /// Type of block that was hit
    pub block_type: BlockType,
    /// Index into the block collection
    pub index: usize,
    /// Screen area of the block
    pub area: Rect,
    /// Clipping context if block is partially visible
    pub clip: Option<ClipContext>,
}

/// Result of handling an event
#[derive(Debug, Clone)]
pub enum EventResult {
    /// Block consumed the event
    Consumed,
    /// Block ignored the event, pass to parent
    Ignored,
    /// Block triggered an action
    Action(BlockEvent),
}

/// Events that blocks can emit
#[derive(Debug, Clone)]
pub enum BlockEvent {
    /// Request focus on this block
    RequestFocus,
    /// Block was expanded
    Expanded,
    /// Block was collapsed
    Collapsed,
    /// Block requests to be closed/removed
    Close,
    /// Block pinned state changed
    Pinned(bool),
    /// Toggle global diff display mode (unified <-> side-by-side)
    ToggleDiffMode,
}

/// Simple scrolling for blocks with fixed-line content (no width dependency)
///
/// Used by blocks where total content lines are known without render width:
/// ToolResultBlock (result list), WebSearchBlock (result pairs)
pub trait SimpleScrollable {
    /// Get the total number of content lines
    fn total_lines(&self) -> u16;
    /// Get the current scroll offset
    fn scroll_offset(&self) -> u16;
    /// Set the scroll offset (implementation should clamp to max)
    fn set_scroll_offset(&mut self, offset: u16);
    /// Get the max visible lines constant for this block type
    fn max_visible_lines(&self) -> u16;

    /// Scroll up by one line
    fn scroll_up(&mut self) {
        let current = self.scroll_offset();
        self.set_scroll_offset(current.saturating_sub(1));
    }

    /// Scroll down by one line
    fn scroll_down(&mut self) {
        let current = self.scroll_offset();
        let max = self.max_scroll();
        if current < max {
            self.set_scroll_offset(current + 1);
        }
    }

    /// Calculate max scroll offset
    fn max_scroll(&self) -> u16 {
        self.total_lines().saturating_sub(self.max_visible_lines())
    }

    /// Check if scrollbar is needed
    fn needs_scrollbar(&self) -> bool {
        self.total_lines() > self.max_visible_lines()
    }

    /// Get scroll info: (total_lines, visible_lines, scrollbar_height)
    fn simple_scroll_info(&self) -> (u16, u16, u16) {
        let total = self.total_lines();
        let visible = total.min(self.max_visible_lines());
        (total, visible, visible)
    }
}

/// Width-dependent scrolling for blocks that wrap content dynamically
///
/// Used by blocks where content wrapping depends on render width:
/// ReadBlock, WriteBlock, ThinkingBlock
pub trait WidthScrollable {
    /// Get wrapped lines for a given width
    fn get_lines(&mut self, width: u16) -> &[String];
    /// Get the current scroll offset
    fn scroll_offset(&self) -> u16;
    /// Set the scroll offset
    fn set_scroll_offset(&mut self, offset: u16);
    /// Get the max visible lines constant for this block type
    fn max_visible_lines(&self) -> u16;

    /// Scroll up by one line
    fn scroll_up(&mut self) {
        let current = self.scroll_offset();
        self.set_scroll_offset(current.saturating_sub(1));
    }

    /// Scroll down by one line (requires width for max calculation)
    fn scroll_down(&mut self, width: u16) {
        let current = self.scroll_offset();
        let max = self.max_scroll(width);
        if current < max {
            self.set_scroll_offset(current + 1);
        }
    }

    /// Calculate max scroll offset (requires width)
    fn max_scroll(&mut self, width: u16) -> u16 {
        let total = self.get_lines(width).len() as u16;
        total.saturating_sub(self.max_visible_lines())
    }

    /// Check if scrollbar is needed (requires width)
    fn needs_scrollbar(&mut self, width: u16) -> bool {
        self.get_lines(width).len() as u16 > self.max_visible_lines()
    }

    /// Get scroll info: (total_lines, visible_lines, scrollbar_height)
    fn get_width_scroll_info(&mut self, width: u16) -> (u16, u16, u16) {
        let total = self.get_lines(width).len() as u16;
        let visible = total.min(self.max_visible_lines());
        (total, visible, visible)
    }
}

/// Core trait for all stream blocks
pub trait StreamBlock: Send + Sync {
    /// Calculate height needed given a width
    fn height(&self, width: u16, theme: &Theme) -> u16;

    /// Render into the given buffer area
    ///
    /// When `clip` is Some, the block is partially visible and should:
    /// - Skip top border if clip.clip_top > 0
    /// - Skip bottom border if clip.clip_bottom > 0
    /// - Adjust content rendering for the visible portion
    fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        focused: bool,
        clip: Option<ClipContext>,
    );

    /// Handle input events
    ///
    /// When `clip` is Some, translate screen coordinates to block-internal:
    /// internal_y = (screen_y - area.y) + clip.clip_top
    fn handle_event(
        &mut self,
        event: &Event,
        area: Rect,
        clip: Option<ClipContext>,
    ) -> EventResult {
        let _ = (event, area, clip);
        EventResult::Ignored
    }

    /// Get copyable text content
    fn get_text_content(&self) -> Option<String> {
        None
    }

    /// Update animation state, returns true if needs redraw
    fn tick(&mut self) -> bool {
        false
    }

    /// Is this block currently streaming/loading?
    fn is_streaming(&self) -> bool {
        false
    }
}

// Re-exports
pub use bash::BashBlock;
pub use build::BuildBlock;
pub use edit::{DiffMode, EditBlock};
pub use explore::ExploreBlock;
pub use pinch::PinchBlock;
pub use read::ReadBlock;
pub use terminal_pane::TerminalPane;
pub use thinking::ThinkingBlock;
pub use tool_result::ToolResultBlock;
pub use web_search::WebSearchBlock;
pub use write::WriteBlock;
