//! Installable plugin browser and catalog popup.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::common::{
    center_content, center_rect, popup_block, popup_title, render_popup_background,
    scroll_indicator, PopupSize,
};
use crate::plugins::{InstalledPlugin, PluginCatalogEntry, PluginRuntime, PluginSourceTrust};
use crate::tui::themes::Theme;
use crate::tui::utils::truncate_ellipsis;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginBrowserSelection {
    Installed { id: String },
    Catalog { id: String, package: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginBrowserItem {
    Installed(usize),
    Catalog(usize),
}

pub struct PluginsBrowserPopup {
    pub plugins: Vec<InstalledPlugin>,
    pub catalog: Vec<PluginCatalogEntry>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub search_query: String,
    pub search_active: bool,
    pub status_message: Option<String>,
}

impl Default for PluginsBrowserPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginsBrowserPopup {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            catalog: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            search_query: String::new(),
            search_active: false,
            status_message: None,
        }
    }

    pub fn set_plugins(&mut self, plugins: Vec<InstalledPlugin>) {
        self.plugins = plugins;
        self.clamp_selection();
    }

    pub fn set_catalog(&mut self, catalog: Vec<PluginCatalogEntry>) {
        self.catalog = catalog;
        self.clamp_selection();
    }

    pub fn set_status_message(&mut self, message: Option<String>) {
        self.status_message = message;
    }

    pub fn selected_item(&self) -> Option<PluginBrowserSelection> {
        match self.filtered_items().get(self.selected_index).copied()? {
            PluginBrowserItem::Installed(index) => {
                let plugin = self.plugins.get(index)?;
                Some(PluginBrowserSelection::Installed {
                    id: plugin.id.clone(),
                })
            }
            PluginBrowserItem::Catalog(index) => {
                let plugin = self.catalog.get(index)?;
                Some(PluginBrowserSelection::Catalog {
                    id: plugin.id.clone(),
                    package: plugin.package.clone(),
                })
            }
        }
    }

    pub fn next(&mut self) {
        let len = self.filtered_items().len();
        if self.selected_index < len.saturating_sub(1) {
            self.selected_index += 1;
            self.ensure_visible();
        }
    }

    pub fn prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.ensure_visible();
        }
    }

    pub fn toggle_search(&mut self) {
        self.search_active = !self.search_active;
        if !self.search_active {
            self.search_query.clear();
        }
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    pub fn add_search_char(&mut self, c: char) {
        if self.search_active {
            self.search_query.push(c);
            self.selected_index = 0;
            self.scroll_offset = 0;
        }
    }

    pub fn backspace_search(&mut self) {
        if self.search_active {
            self.search_query.pop();
            self.selected_index = 0;
            self.scroll_offset = 0;
        }
    }

    fn clamp_selection(&mut self) {
        let len = self.filtered_items().len();
        if len == 0 {
            self.selected_index = 0;
            self.scroll_offset = 0;
            return;
        }
        self.selected_index = self.selected_index.min(len - 1);
        self.ensure_visible();
    }

    fn ensure_visible(&mut self) {
        self.ensure_visible_with_height(8);
    }

    fn ensure_visible_with_height(&mut self, visible_height: usize) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index - visible_height + 1;
        }
    }

    fn filtered_items(&self) -> Vec<PluginBrowserItem> {
        let query = self.search_query.to_lowercase();
        let matches =
            |haystack: String| query.is_empty() || haystack.to_lowercase().contains(&query);

        let mut items = Vec::new();
        for (index, plugin) in self.plugins.iter().enumerate() {
            let haystack = format!(
                "{} {} {} {}",
                plugin.id,
                plugin.name,
                plugin.publisher,
                plugin.description.as_deref().unwrap_or_default()
            );
            if matches(haystack) {
                items.push(PluginBrowserItem::Installed(index));
            }
        }

        for (index, plugin) in self.catalog.iter().enumerate() {
            if self
                .plugins
                .iter()
                .any(|installed| installed.id == plugin.id)
            {
                continue;
            }
            let haystack = format!(
                "{} {} {} {} {}",
                plugin.id,
                plugin.name,
                plugin.publisher,
                plugin.description.as_deref().unwrap_or_default(),
                plugin.tags.join(" ")
            );
            if matches(haystack) {
                items.push(PluginBrowserItem::Catalog(index));
            }
        }

        items
    }

    pub fn render(&self, f: &mut Frame, theme: &Theme) {
        let (w, h) = PopupSize::Large.dimensions();
        let area = center_rect(w, h, f.area());
        render_popup_background(f, area, theme);

        let block = popup_block(theme);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let search_height = if self.search_active { 2 } else { 0 };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(search_height),
                Constraint::Min(6),
                Constraint::Length(2),
            ])
            .split(inner);

        let items = self.filtered_items();
        let title_text = if self.search_query.is_empty() {
            format!(
                "Plugins ({} installed, {} catalog)",
                self.plugins.len(),
                self.catalog.len()
            )
        } else {
            format!(
                "Plugins ({}/{} matches)",
                items.len(),
                self.plugins.len() + self.catalog.len()
            )
        };
        let title_lines = popup_title(&title_text, theme);
        f.render_widget(
            Paragraph::new(title_lines).alignment(Alignment::Center),
            chunks[0],
        );

        if self.search_active {
            let search = Paragraph::new(Line::from(vec![
                Span::styled("  Search: ", Style::default().fg(theme.accent_color)),
                Span::styled(&self.search_query, Style::default().fg(theme.text_color)),
                Span::styled(
                    "_",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ]));
            f.render_widget(search, chunks[1]);
        }

        let visible_height = (chunks[2].height as usize).saturating_sub(3) / 2;

        let mut lines = Vec::new();
        if items.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                if self.search_query.is_empty() {
                    "  No plugins installed or available from catalog sources."
                } else {
                    "  No plugins match your search."
                },
                Style::default().fg(theme.dim_color),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "  Add a catalog: /plugins add-source <catalog-url> [name]",
                Style::default().fg(theme.text_color),
            )]));
            lines.push(Line::from(vec![Span::styled(
                "  Or install directly: /plugins install <npm:package|package-dir|manifest>",
                Style::default().fg(theme.dim_color),
            )]));
        } else {
            if self.scroll_offset > 0 {
                lines.push(scroll_indicator("up", self.scroll_offset, theme));
            }

            let visible_end = (self.scroll_offset + visible_height).min(items.len());
            for (display_idx, item) in items
                .iter()
                .enumerate()
                .skip(self.scroll_offset)
                .take(visible_height)
            {
                let selected = display_idx == self.selected_index;
                self.render_item_line(&mut lines, *item, selected, theme);
            }

            let remaining = items.len().saturating_sub(visible_end);
            if remaining > 0 {
                lines.push(scroll_indicator("down", remaining, theme));
            }
        }

        if let Some(status) = &self.status_message {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                truncate_ellipsis(status, 70),
                Style::default().fg(theme.accent_color),
            )]));
        }

        let content = Paragraph::new(lines).style(Style::default().bg(theme.bg_color));
        f.render_widget(content, center_content(chunks[2], 2));

        let footer = if self.search_active {
            Paragraph::new(Line::from(vec![
                Span::styled("Type to search  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": close search", Style::default().fg(theme.text_color)),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "/",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": search  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "↑↓",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": nav  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "Enter",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": install/toggle  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "r",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": refresh  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": close", Style::default().fg(theme.text_color)),
            ]))
        };
        f.render_widget(footer.alignment(Alignment::Center), chunks[3]);
    }

    fn render_item_line<'a>(
        &'a self,
        lines: &mut Vec<Line<'a>>,
        item: PluginBrowserItem,
        selected: bool,
        theme: &'a Theme,
    ) {
        let prefix = if selected { " › " } else { "   " };
        let name_style = if selected {
            Style::default()
                .fg(theme.accent_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_color)
        };

        match item {
            PluginBrowserItem::Installed(index) => {
                let plugin = &self.plugins[index];
                let status = if plugin.enabled {
                    ("installed/enabled", theme.success_color)
                } else {
                    ("installed/disabled", theme.warning_color)
                };
                let mode = if plugin.entry_component_path.is_none() {
                    "bundle"
                } else if plugin
                    .render_capabilities
                    .iter()
                    .any(|cap| matches!(cap, crate::plugins::PluginRenderCapability::Frame))
                {
                    "frame"
                } else {
                    "text"
                };
                let trust = match plugin.source_trust {
                    PluginSourceTrust::SignedPublisher => "signed",
                    PluginSourceTrust::NpmUnsigned => "npm/unsigned",
                    PluginSourceTrust::LocalUnsigned => "local/unsigned",
                    PluginSourceTrust::LegacyUnknown => "unverified",
                };
                lines.push(Line::from(vec![
                    Span::styled(prefix.to_string(), name_style),
                    Span::styled(plugin.name.clone(), name_style),
                    Span::styled(
                        format!(" v{}", plugin.version),
                        Style::default().fg(theme.dim_color),
                    ),
                    Span::styled(
                        format!(" [{}]", mode),
                        Style::default().fg(theme.mode_view_color),
                    ),
                    Span::styled(
                        format!(
                            " ({}, {}, {})",
                            status.0,
                            if plugin.pinned { "pinned" } else { "updatable" },
                            trust
                        ),
                        Style::default().fg(status.1),
                    ),
                ]));
                let desc = plugin
                    .description
                    .as_ref()
                    .map(|text| truncate_ellipsis(text, 56).into_owned())
                    .unwrap_or_else(|| "No description".to_string());
                let permission_count = [
                    plugin.requested_permissions.fs_read,
                    plugin.requested_permissions.fs_write,
                    plugin.requested_permissions.network,
                    plugin.requested_permissions.process,
                ]
                .into_iter()
                .filter(|requested| *requested)
                .count();
                let bundle_count = plugin.skill_paths.len()
                    + plugin.agent_extension_paths.len()
                    + plugin.hook_paths.len()
                    + usize::from(plugin.mcp_servers_path.is_some())
                    + usize::from(plugin.assets_path.is_some());
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(
                        format!(
                            "{}{}{}",
                            desc,
                            if bundle_count > 0 {
                                format!(" · {} bundle component(s)", bundle_count)
                            } else {
                                String::new()
                            },
                            if permission_count > 0 {
                                format!(" · {} permission request(s)", permission_count)
                            } else {
                                String::new()
                            }
                        ),
                        Style::default().fg(theme.dim_color),
                    ),
                ]));
            }
            PluginBrowserItem::Catalog(index) => {
                let plugin = &self.catalog[index];
                let runtime = match plugin.runtime {
                    PluginRuntime::Native => "native",
                    PluginRuntime::Wasm => "wasm",
                    PluginRuntime::Js => "js",
                };
                lines.push(Line::from(vec![
                    Span::styled(prefix.to_string(), name_style),
                    Span::styled(plugin.name.clone(), name_style),
                    Span::styled(
                        format!(" v{}", plugin.version),
                        Style::default().fg(theme.dim_color),
                    ),
                    Span::styled(
                        format!(" [{}]", runtime),
                        Style::default().fg(theme.mode_view_color),
                    ),
                    Span::styled(
                        if plugin.official {
                            " (official)"
                        } else {
                            " (catalog)"
                        }
                        .to_string(),
                        Style::default().fg(if plugin.official {
                            theme.success_color
                        } else {
                            theme.warning_color
                        }),
                    ),
                ]));
                let desc = plugin
                    .description
                    .as_ref()
                    .map(|text| truncate_ellipsis(text, 56).into_owned())
                    .unwrap_or_else(|| plugin.package.clone());
                lines.push(Line::from(vec![
                    Span::raw("      "),
                    Span::styled(desc, Style::default().fg(theme.dim_color)),
                ]));
            }
        }
    }
}
