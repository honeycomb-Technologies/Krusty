//! Theme system for Mitsuro TUI
//!
//! Provides 30+ beautiful themes with intelligent color defaults.

use ratatui::style::Color;

pub mod base;
pub mod definitions;
mod registry;

use once_cell::sync::Lazy;
pub use registry::ThemeRegistry;

/// Global theme registry with all built-in themes
pub static THEME_REGISTRY: Lazy<ThemeRegistry> = Lazy::new(ThemeRegistry::new);

/// A complete theme definition
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub display_name: String,

    // Core colors
    pub bg_color: Color,
    pub border_color: Color,
    pub title_color: Color,
    pub accent_color: Color,
    pub text_color: Color,
    pub success_color: Color,
    pub dim_color: Color,

    // Mode colors (kept for backward compatibility, used as accent variants)
    pub mode_view_color: Color,
    pub mode_chat_color: Color,
    pub mode_plan_color: Color,
    pub mode_bash_color: Color,
    pub mode_leader_color: Color,

    // Special colors
    pub warning_color: Color,
    pub error_color: Color,
    pub code_bg_color: Color,

    // UI element colors
    pub cursor_color: Color,
    pub selection_bg_color: Color,
    pub selection_fg_color: Color,

    // Message role colors (text)
    pub user_msg_color: Color,
    pub assistant_msg_color: Color,
    pub system_msg_color: Color,
    pub tool_msg_color: Color,

    // Status colors
    pub info_color: Color,
    pub progress_color: Color,

    // Input & Form Colors
    pub input_bg_color: Color,
    pub input_placeholder_color: Color,
    pub input_border_color: Color,

    // Message Bubble Backgrounds
    pub user_msg_bg_color: Color,
    pub assistant_msg_bg_color: Color,
    pub system_msg_bg_color: Color,
    pub tool_msg_bg_color: Color,

    // Status Bar & UI Components
    pub status_bar_bg_color: Color,
    pub scrollbar_bg_color: Color,
    pub scrollbar_fg_color: Color,
    pub scrollbar_hover_color: Color,

    // Branding & Logo
    pub logo_primary_color: Color,
    pub logo_secondary_color: Color,

    // Animation & Effects
    pub animation_color: Color,
    pub processing_color: Color,
    pub highlight_color: Color,
    pub bubble_color: Color,

    // Token Usage Indicators
    pub token_low_color: Color,
    pub token_medium_color: Color,
    pub token_high_color: Color,
    pub token_critical_color: Color,

    // Syntax Highlighting Colors
    pub syntax_keyword_color: Color,
    pub syntax_function_color: Color,
    pub syntax_string_color: Color,
    pub syntax_number_color: Color,
    pub syntax_comment_color: Color,
    pub syntax_type_color: Color,
    pub syntax_variable_color: Color,
    pub syntax_operator_color: Color,
    pub syntax_punctuation_color: Color,

    // Diff & Code Display Colors
    pub diff_add_color: Color,
    pub diff_add_bg_color: Color,
    pub diff_remove_color: Color,
    pub diff_remove_bg_color: Color,
    pub diff_context_color: Color,
    pub line_number_color: Color,
    pub link_color: Color,
    pub running_color: Color,
}

impl Theme {
    /// Get bubble color as RGB tuple
    pub fn get_bubble_rgb(&self) -> (u8, u8, u8) {
        match self.bubble_color {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (139, 233, 253), // Default to ocean cyan
        }
    }
}
