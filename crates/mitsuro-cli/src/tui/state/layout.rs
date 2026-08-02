//! Layout State - Centralized layout area tracking
//!
//! Owns all cached layout rectangles used for hit testing and rendering.

use ratatui::layout::Rect;

use super::DragTarget;

/// Cached layout areas for hit testing and rendering
///
/// Updated each frame during rendering, used for mouse event handling.
#[derive(Debug, Default)]
pub struct LayoutState {
    /// Input text area bounds
    pub input_area: Option<Rect>,
    /// Messages/chat area bounds
    pub messages_area: Option<Rect>,
    /// Pinned terminal pane area (when visible)
    pub pinned_terminal_area: Option<Rect>,
    /// Input scrollbar track area
    pub input_scrollbar_area: Option<Rect>,
    /// Messages scrollbar track area
    pub messages_scrollbar_area: Option<Rect>,
    /// Plan sidebar area
    pub plan_sidebar_area: Option<Rect>,
    /// Plan sidebar scrollbar track area
    pub plan_sidebar_scrollbar_area: Option<Rect>,
    /// Plugin window area
    pub plugin_window_area: Option<Rect>,
    /// Plugin window scrollbar track area
    pub plugin_window_scrollbar_area: Option<Rect>,
    /// Plugin/plan divider area (for drag resizing)
    pub plugin_divider_area: Option<Rect>,
    /// Whether mouse is hovering over the plugin divider
    pub plugin_divider_hovered: bool,
    /// Decision prompt area (when visible)
    pub prompt_area: Option<Rect>,
    /// Currently dragging scrollbar (if any)
    pub dragging_scrollbar: Option<DragTarget>,
    /// Session title area in toolbar (for click-to-edit)
    pub toolbar_title_area: Option<Rect>,
}

impl LayoutState {
    /// Create a new empty layout state
    pub fn new() -> Self {
        Self::default()
    }
}
