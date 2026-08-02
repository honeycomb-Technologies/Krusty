//! Selection State - Text selection and scrollbar drag management
//!
//! Handles mouse-based text selection across messages and input areas,
//! as well as scrollbar drag tracking for various block types.

use crate::tui::blocks::BlockType;
use ratatui::layout::Rect;

/// Scrollbar drag state for messages/input - tracks initial position for relative dragging
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarDrag {
    /// Y position where drag started
    pub start_y: u16,
    /// Scroll offset when drag started
    pub start_offset: usize,
    /// Scrollbar area for calculations
    pub area: Rect,
    /// Maximum scroll offset
    pub max_scroll: usize,
}

impl ScrollbarDrag {
    /// Create new drag state
    pub fn new(start_y: u16, start_offset: usize, area: Rect, max_scroll: usize) -> Self {
        Self {
            start_y,
            start_offset,
            area,
            max_scroll,
        }
    }

    /// Calculate new scroll offset based on current y position (relative to start)
    pub fn calculate_offset(&self, current_y: u16) -> usize {
        if self.area.height <= 1 || self.max_scroll == 0 {
            return self.start_offset;
        }

        // Calculate delta in pixels
        let delta_y = current_y as i32 - self.start_y as i32;

        // Convert pixel delta to scroll offset delta
        // ratio: how much of max_scroll per pixel of scrollbar height
        let scroll_per_pixel = self.max_scroll as f32 / (self.area.height.saturating_sub(1)) as f32;
        let offset_delta = (delta_y as f32 * scroll_per_pixel).round() as i32;

        // Apply delta to start offset, clamping to valid range
        let new_offset = self.start_offset as i32 + offset_delta;
        new_offset.clamp(0, self.max_scroll as i32) as usize
    }
}

/// Scrollbar drag state for block scrollbars
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockScrollbarDrag {
    pub block_type: BlockType,
    pub index: usize,
    pub scrollbar_y: u16,
    pub scrollbar_height: u16,
    pub total_lines: u16,
    pub visible_lines: u16,
}

impl BlockScrollbarDrag {
    /// Calculate scroll offset from current mouse y position
    pub fn calculate_offset(&self, y: u16) -> Option<u16> {
        let max_scroll = self.total_lines.saturating_sub(self.visible_lines);
        if self.scrollbar_height == 0 || max_scroll == 0 {
            return None;
        }
        let relative_y = y.saturating_sub(self.scrollbar_y);
        let ratio = (relative_y as f32 / self.scrollbar_height as f32).clamp(0.0, 1.0);
        Some((ratio * max_scroll as f32).round() as u16)
    }
}

/// Which scrollbar is being dragged
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DragTarget {
    /// Input scrollbar with drag state
    Input(ScrollbarDrag),
    /// Messages scrollbar with drag state
    Messages(ScrollbarDrag),
    /// Plan sidebar scrollbar
    PlanSidebar,
    /// Plugin window scrollbar
    PluginWindow,
    /// Plugin/plan divider (for resizing)
    PluginDivider { start_y: u16, start_position: f32 },
    /// Block scrollbar (consolidated for all block types)
    Block(BlockScrollbarDrag),
}

/// Edge scroll direction during selection drag
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeScrollDirection {
    Up,
    Down,
}

/// Edge scroll state for continuous scrolling while holding at edge
#[derive(Debug, Clone, Copy, Default)]
pub struct EdgeScrollState {
    pub direction: Option<EdgeScrollDirection>,
    pub area: SelectionArea,
    pub last_x: u16,
}

/// Which area text selection is happening in
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SelectionArea {
    #[default]
    None,
    Messages,
    Input,
}

/// Text selection state - position as (line, column)
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    pub start: Option<(usize, usize)>,
    pub end: Option<(usize, usize)>,
    pub is_selecting: bool,
    pub area: SelectionArea,
}

impl SelectionState {
    /// Get normalized selection (start always before end)
    pub fn normalized(&self) -> Option<((usize, usize), (usize, usize))> {
        let (start, end) = (self.start?, self.end?);
        Some(if start <= end {
            (start, end)
        } else {
            (end, start)
        })
    }

    /// Check if selection is non-empty
    pub fn has_selection(&self) -> bool {
        matches!((self.start, self.end), (Some(s), Some(e)) if s != e)
    }

    /// Clear selection state
    pub fn clear(&mut self) {
        self.start = None;
        self.end = None;
        self.is_selecting = false;
        self.area = SelectionArea::None;
    }
}
