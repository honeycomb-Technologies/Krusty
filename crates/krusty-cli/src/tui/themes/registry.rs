//! Theme registry for discovering and accessing themes

use super::Theme;
use std::collections::HashMap;

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
        registry.register(krusty());
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
            .unwrap_or_else(|| self.themes.get("krusty").expect("Default theme must exist"))
    }

    /// List all themes in registration order
    pub fn list(&self) -> Vec<(&String, &Theme)> {
        self.ordered_names
            .iter()
            .filter_map(|name| self.themes.get(name).map(|theme| (name, theme)))
            .collect()
    }

    /// Get the number of registered themes
    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.themes.len()
    }
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
