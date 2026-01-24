//! Scrollable list utilities for popups
//!
//! Provides reusable scroll management that many popups share.

/// Manages scroll state for a list of items
#[derive(Debug, Clone)]
pub struct ScrollState {
    /// Currently selected index
    pub selected: usize,
    /// Scroll offset (first visible item)
    pub offset: usize,
    /// Number of items in the list
    pub total: usize,
    /// Visible height (items that fit on screen)
    pub visible_height: usize,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new(0)
    }
}

impl ScrollState {
    /// Create new scroll state for a list
    pub fn new(total: usize) -> Self {
        Self {
            selected: 0,
            offset: 0,
            total,
            visible_height: 10,
        }
    }

    /// Update the total item count
    pub fn set_total(&mut self, total: usize) {
        self.total = total;
        if self.selected >= total && total > 0 {
            self.selected = total - 1;
        }
        self.ensure_visible();
    }

    /// Set the visible height (call after layout)
    pub fn set_visible_height(&mut self, height: usize) {
        self.visible_height = height.max(1);
        self.ensure_visible();
    }

    /// Move selection down
    pub fn next(&mut self) {
        if self.total == 0 {
            return;
        }
        if self.selected < self.total - 1 {
            self.selected += 1;
            self.ensure_visible();
        }
    }

    /// Move selection up
    pub fn prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    /// Ensure selected item is visible
    pub fn ensure_visible(&mut self) {
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + self.visible_height {
            self.offset = self.selected - self.visible_height + 1;
        }
    }

    /// Get the range of visible indices
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        let end = (self.offset + self.visible_height).min(self.total);
        self.offset..end
    }

    /// Check if item at index is selected
    pub fn is_selected(&self, index: usize) -> bool {
        index == self.selected
    }

    /// Get items above the visible area
    pub fn items_above(&self) -> usize {
        self.offset
    }

    /// Get items below the visible area
    pub fn items_below(&self) -> usize {
        self.total.saturating_sub(self.offset + self.visible_height)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_state_navigation() {
        let mut state = ScrollState::new(20);
        state.set_visible_height(5);

        assert_eq!(state.selected, 0);
        assert_eq!(state.offset, 0);

        // Move down
        state.next();
        assert_eq!(state.selected, 1);

        // Move past visible area
        for _ in 0..5 {
            state.next();
        }
        assert_eq!(state.selected, 6);
        assert!(state.offset > 0); // Should have scrolled

        // Move up
        state.prev();
        assert_eq!(state.selected, 5);
    }

    #[test]
    fn test_scroll_state_bounds() {
        let mut state = ScrollState::new(5);
        state.set_visible_height(10);

        // Move to end
        for _ in 0..10 {
            state.next();
        }
        assert_eq!(state.selected, 4); // Can't go past end

        // Move to start
        for _ in 0..10 {
            state.prev();
        }
        assert_eq!(state.selected, 0); // Can't go before start
    }

    #[test]
    fn test_visible_range() {
        let mut state = ScrollState::new(20);
        state.set_visible_height(5);

        assert_eq!(state.visible_range(), 0..5);
        assert_eq!(state.items_above(), 0);
        assert_eq!(state.items_below(), 15);

        // Scroll down
        for _ in 0..10 {
            state.next();
        }
        assert!(state.visible_range().contains(&state.selected));
    }
}
