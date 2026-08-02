//! Theme management handlers
//!
//! Theme switching, preview, and persistence.

use std::sync::Arc;

use crate::tui::app::App;
use crate::tui::themes::THEME_REGISTRY;

impl App {
    /// Set theme and persist to preferences
    pub fn set_theme(&mut self, name: &str) {
        let theme = THEME_REGISTRY.get_or_default(name);
        self.ui.theme = Arc::new(theme.clone());
        self.ui.theme_name = name.to_string();

        // Update menu animator with theme color
        let accent_rgb = theme.get_bubble_rgb();
        self.ui.menu_animator.set_theme_color(accent_rgb);

        // Save to preferences
        if let Some(ref prefs) = self.services.preferences {
            if let Err(e) = prefs.set_theme(name) {
                tracing::warn!("Failed to save theme preference: {}", e);
            }
        }
    }

    /// Preview theme without saving to preferences (for live preview)
    pub fn preview_theme(&mut self, name: &str) {
        let theme = THEME_REGISTRY.get_or_default(name);
        self.ui.theme = Arc::new(theme.clone());
        self.ui.theme_name = name.to_string();

        // Update menu animator with theme color
        let accent_rgb = theme.get_bubble_rgb();
        self.ui.menu_animator.set_theme_color(accent_rgb);
        // Don't save to preferences - this is just a preview
    }

    /// Restore theme to original (cancel preview)
    pub fn restore_original_theme(&mut self) {
        if let Some(original) = self
            .ui
            .popups
            .theme
            .get_original_theme_name()
            .map(|s| s.to_string())
        {
            self.preview_theme(&original);
        }
    }
}
