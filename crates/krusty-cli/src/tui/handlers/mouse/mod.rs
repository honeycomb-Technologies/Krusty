//! Mouse event handling
//!
//! Handles mouse clicks, scrolling, drag operations, hover behavior, and text selection.
//! Submodules separate click routing, block interaction, scroll routing, and hover/file-link handling.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use crate::tui::app::{App, Popup};
use crate::tui::blocks::{BlockType, ClipContext, StreamBlock};
use crate::tui::state::{BlockScrollbarDrag, DragTarget, ScrollbarDrag, SelectionArea};

mod block_click;
mod click;
mod hover;
mod scroll;

/// Extract clip values from optional ClipContext
fn extract_clip(clip: Option<ClipContext>) -> (u16, u16) {
    clip.map(|c| (c.clip_top, c.clip_bottom)).unwrap_or((0, 0))
}

/// Create a BlockScrollbarDrag for a block
fn make_block_scrollbar_drag(
    block_type: BlockType,
    index: usize,
    block_area: ratatui::layout::Rect,
    clip: Option<ClipContext>,
    total_lines: u16,
    visible_lines: u16,
) -> BlockScrollbarDrag {
    let (clip_top, clip_bottom) = extract_clip(clip);
    let header_lines = if clip_top == 0 { 1u16 } else { 0 };
    let footer_lines = if clip_bottom == 0 { 1u16 } else { 0 };
    let scrollbar_height = block_area
        .height
        .saturating_sub(header_lines + footer_lines);
    let scrollbar_y = block_area.y + header_lines;

    BlockScrollbarDrag {
        block_type,
        index,
        scrollbar_y,
        scrollbar_height,
        total_lines,
        visible_lines,
    }
}

/// Scroll direction for routing
#[derive(Clone, Copy)]
enum ScrollDirection {
    Up,
    Down,
}

impl App {
    /// Handle mouse events for scrolling, clicking, and selection
    pub fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.handle_scroll(mouse.column, mouse.row, ScrollDirection::Down);
            }
            MouseEventKind::ScrollUp => {
                self.handle_scroll(mouse.column, mouse.row, ScrollDirection::Up);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_left_click(mouse);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.handle_drag(mouse.column, mouse.row);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.handle_mouse_up();
            }
            MouseEventKind::Moved => {
                self.ui.scroll_system.scroll.unlock_from_messages();
                self.update_hover_state(mouse.column, mouse.row);
            }
            _ => {}
        }
    }

    /// Handle mouse drag (for scrollbar dragging and text selection)
    fn handle_drag(&mut self, x: u16, y: u16) {
        // Handle scrollbar dragging first (from scrollbar.rs)
        if self.handle_scrollbar_drag(y) {
            return;
        }

        // Handle text selection dragging (from selection.rs)
        self.handle_selection_drag(x, y);
    }

    /// Handle mouse button release
    fn handle_mouse_up(&mut self) {
        self.ui.scroll_system.layout.dragging_scrollbar = None;
        self.ui.scroll_system.edge_scroll.direction = None;

        if self.ui.scroll_system.selection.is_selecting
            && self.ui.scroll_system.selection.has_selection()
        {
            // Copy to clipboard then clear selection
            self.copy_selection_to_clipboard();
            self.ui.scroll_system.selection.clear();
        } else {
            self.ui.scroll_system.selection.is_selecting = false;
        }

        self.ui.scroll_system.scroll.unlock_from_selection();
    }
}
