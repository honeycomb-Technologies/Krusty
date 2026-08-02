//! Theme registry for discovering and accessing themes

use super::Theme;
use once_cell::sync::Lazy;
use ratatui::style::Color;
use std::collections::HashMap;

/// Hardcoded minimal fallback theme used when both the requested theme
/// and the "mitsuro" default are missing from the registry.
static DEFAULT_THEME: Lazy<Theme> = Lazy::new(|| Theme {
    name: "mitsuro".to_string(),
    display_name: "Mitsuro Original".to_string(),
    bg_color: Color::Rgb(24, 24, 37),
    border_color: Color::Rgb(88, 91, 112),
    title_color: Color::Rgb(139, 233, 253),
    accent_color: Color::Rgb(189, 147, 249),
    text_color: Color::Rgb(203, 213, 225),
    success_color: Color::Rgb(80, 250, 123),
    dim_color: Color::Rgb(148, 163, 184),
    mode_view_color: Color::Rgb(80, 250, 123),
    mode_chat_color: Color::Rgb(255, 121, 198),
    mode_plan_color: Color::Rgb(139, 233, 253),
    mode_bash_color: Color::Rgb(255, 184, 108),
    mode_leader_color: Color::Rgb(189, 147, 249),
    warning_color: Color::Rgb(255, 203, 107),
    error_color: Color::Rgb(255, 85, 85),
    code_bg_color: Color::Rgb(30, 30, 45),
    cursor_color: Color::Rgb(189, 147, 249),
    selection_bg_color: Color::Rgb(51, 65, 85),
    selection_fg_color: Color::Rgb(203, 213, 225),
    user_msg_color: Color::Rgb(80, 250, 123),
    assistant_msg_color: Color::Rgb(189, 147, 249),
    system_msg_color: Color::Rgb(255, 203, 107),
    tool_msg_color: Color::Rgb(139, 233, 253),
    info_color: Color::Rgb(139, 233, 253),
    progress_color: Color::Rgb(189, 147, 249),
    input_bg_color: Color::Rgb(51, 65, 85),
    input_placeholder_color: Color::Rgb(100, 116, 139),
    input_border_color: Color::Rgb(88, 91, 112),
    user_msg_bg_color: Color::Rgb(40, 40, 60),
    assistant_msg_bg_color: Color::Rgb(50, 50, 50),
    system_msg_bg_color: Color::Rgb(60, 40, 60),
    tool_msg_bg_color: Color::Rgb(60, 60, 40),
    status_bar_bg_color: Color::Rgb(44, 44, 57),
    scrollbar_bg_color: Color::Rgb(44, 44, 57),
    scrollbar_fg_color: Color::Rgb(88, 91, 112),
    scrollbar_hover_color: Color::Rgb(139, 233, 253),
    logo_primary_color: Color::Rgb(255, 140, 90),
    logo_secondary_color: Color::Rgb(183, 65, 14),
    animation_color: Color::Rgb(255, 140, 90),
    processing_color: Color::Rgb(255, 203, 107),
    highlight_color: Color::Rgb(255, 184, 108),
    bubble_color: Color::Rgb(139, 233, 253),
    token_low_color: Color::Rgb(80, 250, 123),
    token_medium_color: Color::Rgb(255, 203, 107),
    token_high_color: Color::Rgb(255, 121, 198),
    token_critical_color: Color::Rgb(255, 121, 198),
    syntax_keyword_color: Color::Rgb(255, 121, 198),
    syntax_function_color: Color::Rgb(80, 250, 123),
    syntax_string_color: Color::Rgb(241, 250, 140),
    syntax_number_color: Color::Rgb(189, 147, 249),
    syntax_comment_color: Color::Rgb(98, 114, 164),
    syntax_type_color: Color::Rgb(139, 233, 253),
    syntax_variable_color: Color::Rgb(248, 248, 242),
    syntax_operator_color: Color::Rgb(255, 121, 198),
    syntax_punctuation_color: Color::Rgb(248, 248, 242),
    diff_add_color: Color::Rgb(80, 250, 123),
    diff_add_bg_color: Color::Rgb(20, 45, 30),
    diff_remove_color: Color::Rgb(255, 85, 85),
    diff_remove_bg_color: Color::Rgb(50, 25, 30),
    diff_context_color: Color::Rgb(98, 114, 164),
    line_number_color: Color::Rgb(88, 91, 112),
    link_color: Color::Rgb(139, 233, 253),
    running_color: Color::Rgb(139, 233, 253),
});

/// Registry of all available themes
pub struct ThemeRegistry {
    themes: HashMap<String, Theme>,
    ordered_names: Vec<String>,
}

impl ThemeRegistry {
    /// Create a new registry with all built-in themes
    pub fn new() -> Self {
        let mut registry = Self {
            themes: HashMap::new(),
            ordered_names: Vec::new(),
        };

        // Register all themes from definitions module
        use super::definitions::*;

        // System/Terminal theme - uses native terminal colors
        registry.register(terminal());

        // Original themes
        registry.register(mitsuro());
        registry.register(tokyo_night());
        registry.register(dracula());
        registry.register(catppuccin_mocha());
        registry.register(gruvbox_dark());
        registry.register(nord());
        registry.register(one_dark());
        registry.register(solarized_dark());

        // Popular themes
        registry.register(aura());
        registry.register(synthwave_84());
        registry.register(monokai());
        registry.register(palenight());
        registry.register(rosepine());
        registry.register(vesper());
        registry.register(cobalt2());
        registry.register(everforest());
        registry.register(kanagawa());

        // Fun themes
        registry.register(sith_lord());
        registry.register(matrix());
        registry.register(night_owl());
        registry.register(moonlight());
        registry.register(ayu_dark());
        registry.register(material_ocean());
        registry.register(zenburn());
        registry.register(github_dark());

        // Additional unique themes
        registry.register(cyberpunk());
        registry.register(high_contrast());
        registry.register(serenity());
        registry.register(retro_wave());
        registry.register(forest_night());

        registry
    }

    fn register(&mut self, theme: Theme) {
        self.ordered_names.push(theme.name.clone());
        self.themes.insert(theme.name.clone(), theme);
    }

    /// Get a theme by name, or the default theme
    pub fn get_or_default(&self, name: &str) -> &Theme {
        self.themes
            .get(name)
            .or_else(|| self.themes.get("mitsuro"))
            .unwrap_or(&DEFAULT_THEME)
    }

    /// List all themes in registration order
    pub fn list(&self) -> Vec<(&String, &Theme)> {
        self.ordered_names
            .iter()
            .filter_map(|name| self.themes.get(name).map(|theme| (name, theme)))
            .collect()
    }
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
