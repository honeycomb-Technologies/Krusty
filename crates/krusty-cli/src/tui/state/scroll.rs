//! Scroll State - Centralized scroll and viewport management
//!
//! This module owns all scroll-related state and provides a unified interface for:
//! - Scroll position tracking
//! - Auto-scroll behavior
//! - Viewport calculations
//! - Scroll bounds checking

/// Cache for layout calculations to avoid expensive recalculations during animation
#[derive(Debug, Clone, Default)]
pub struct LayoutCache {
    /// Cached message line count
    pub message_lines: usize,
    /// Width used for cached calculation
    pub cached_width: u16,
}

/// Manages scroll state for the messages area
pub struct ScrollState {
    /// Current scroll offset (0 = top, max = bottom)
    pub offset: usize,
    /// Maximum scroll offset for bounds checking
    pub max_scroll: usize,
    /// Whether to auto-scroll to bottom on new content
    pub auto_scroll: bool,
    /// Flag to jump to bottom on next render
    pub scroll_to_bottom: bool,
    /// Lock scroll to messages area (prevents block capture during scroll momentum)
    pub locked_to_messages: bool,
    /// Lock scroll during text selection (prevents block scroll capture)
    pub locked_for_selection: bool,
}

impl ScrollState {
    /// Create a new scroll state with auto-scroll enabled
    pub fn new() -> Self {
        Self {
            offset: 0,
            max_scroll: 0,
            auto_scroll: true,
            scroll_to_bottom: false,
            locked_to_messages: false,
            locked_for_selection: false,
        }
    }

    // =========================================================================
    // Core Scroll Operations
    // =========================================================================

    /// Scroll up by the given amount
    pub fn scroll_up(&mut self, amount: usize) {
        self.offset = self.offset.saturating_sub(amount);
        // Disable auto-scroll when scrolling away from bottom
        if self.offset < self.max_scroll {
            self.auto_scroll = false;
        }
    }

    /// Scroll down by the given amount
    pub fn scroll_down(&mut self, amount: usize) {
        self.offset = self.offset.saturating_add(amount).min(self.max_scroll);
        // Re-enable auto-scroll if at bottom
        if self.offset >= self.max_scroll {
            self.auto_scroll = true;
        }
    }

    /// Scroll to a specific line
    pub fn scroll_to_line(&mut self, line: usize) {
        self.offset = line.min(self.max_scroll);
        self.auto_scroll = self.offset >= self.max_scroll;
    }

    /// Jump to the bottom
    pub fn scroll_to_end(&mut self) {
        self.offset = self.max_scroll;
        self.auto_scroll = true;
    }

    /// Request scroll to bottom on next render
    pub fn request_scroll_to_bottom(&mut self) {
        self.scroll_to_bottom = true;
    }

    /// Apply pending scroll-to-bottom request
    pub fn apply_scroll_to_bottom(&mut self) {
        if self.scroll_to_bottom {
            self.scroll_to_end();
            self.scroll_to_bottom = false;
        }
    }

    // =========================================================================
    // Max Scroll Updates
    // =========================================================================

    /// Update the maximum scroll value based on total lines and viewport height
    pub fn update_max_scroll(&mut self, total_lines: usize, viewport_height: u16) {
        let viewport = viewport_height as usize;
        self.max_scroll = total_lines.saturating_sub(viewport);

        // Clamp current offset to valid range
        if self.offset > self.max_scroll {
            self.offset = self.max_scroll;
        }

        // Auto-scroll to bottom if enabled
        if self.auto_scroll {
            self.offset = self.max_scroll;
        }
    }

    /// Check if can scroll up (not at top)
    pub fn can_scroll_up(&self) -> bool {
        self.offset > 0
    }

    /// Check if can scroll down (not at bottom)
    pub fn can_scroll_down(&self) -> bool {
        self.offset < self.max_scroll
    }

    /// Check if scrollbar is needed (content exceeds viewport)
    /// Note: Most blocks implement their own needs_scrollbar for block-specific logic
    #[allow(dead_code)]
    pub fn needs_scrollbar(&self) -> bool {
        self.max_scroll > 0
    }

    /// Lock scrolling to messages area (during momentum scroll)
    pub fn lock_to_messages(&mut self) {
        self.locked_to_messages = true;
    }

    /// Unlock scrolling from messages area
    pub fn unlock_from_messages(&mut self) {
        self.locked_to_messages = false;
    }

    /// Check if scroll is locked to messages
    pub fn is_locked_to_messages(&self) -> bool {
        self.locked_to_messages
    }

    /// Lock scrolling for selection (blocks route to block scrollbars)
    pub fn lock_for_selection(&mut self) {
        self.locked_for_selection = true;
    }

    /// Unlock scrolling from selection
    pub fn unlock_from_selection(&mut self) {
        self.locked_for_selection = false;
    }

    /// Check if scroll is locked for selection
    pub fn is_locked_for_selection(&self) -> bool {
        self.locked_for_selection
    }
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}
