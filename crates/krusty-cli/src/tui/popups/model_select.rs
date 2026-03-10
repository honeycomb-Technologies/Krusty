//! Model selection popup
//!
//! Displays models grouped by provider with search functionality.
//! Supports recent models, rich metadata, and dynamic model lists.

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
use crate::ai::format_detection::detect_api_format;
use crate::ai::models::{infer_model_metadata, ModelMetadata};
use crate::ai::providers::ProviderId;
use crate::tui::themes::Theme;

/// Entry in the model list
#[derive(Clone)]
pub enum ModelEntry {
    /// Section header (e.g., "RECENT", "ANTHROPIC")
    Header { name: String, count: Option<usize> },
    /// Sub-section header for OpenRouter sub-providers
    SubHeader { name: String },
    /// Model entry with rich metadata
    Model { metadata: ModelMetadata },
}

/// Model selection popup state
pub struct ModelSelectPopup {
    /// Currently selected entry index (in filtered list)
    pub selected_index: usize,
    /// Scroll offset for visible area
    pub scroll_offset: usize,
    /// Search query string
    pub search_query: String,
    /// Whether search input is active
    pub search_active: bool,
    /// All model entries (headers + models)
    entries: Vec<ModelEntry>,
    /// Loading state for dynamic providers
    pub loading: bool,
    /// Error message if fetch failed
    pub error: Option<String>,
}

impl Default for ModelSelectPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelSelectPopup {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            scroll_offset: 0,
            search_query: String::new(),
            search_active: false,
            entries: Vec::new(),
            loading: false,
            error: None,
        }
    }

    /// Set models from organized data (called from App)
    /// recent_models: Models recently used by user
    /// models_by_provider: Models grouped by provider
    pub fn set_models(
        &mut self,
        recent_models: Vec<ModelMetadata>,
        models_by_provider: Vec<(ProviderId, Vec<ModelMetadata>)>,
    ) {
        self.entries.clear();
        self.error = None;
        self.loading = false; // Clear loading state when models are set

        // Add RECENT section if we have recent models
        if !recent_models.is_empty() {
            self.entries.push(ModelEntry::Header {
                name: "RECENT".to_string(),
                count: Some(recent_models.len()),
            });
            for model in recent_models {
                self.entries.push(ModelEntry::Model { metadata: model });
            }
        }

        // Add each provider's models
        for (provider, models) in models_by_provider {
            if models.is_empty() {
                continue;
            }

            if provider == ProviderId::OpenRouter {
                // Group OpenRouter models by sub-provider
                self.add_openrouter_models(models);
            } else {
                // Normal provider handling
                self.entries.push(ModelEntry::Header {
                    name: provider.to_string().to_uppercase(),
                    count: Some(models.len()),
                });

                for model in models {
                    self.entries.push(ModelEntry::Model { metadata: model });
                }
            }
        }

        // Reset selection
        self.selected_index = 0;
        self.scroll_offset = 0;

        // Auto-select first model
        let selectable = self.selectable_indices();
        if let Some(&first) = selectable.first() {
            self.selected_index = first;
        }
    }

    /// Set loading state
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.error = None;
        }
    }

    /// Set error message
    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.loading = false;
    }

    /// Get filtered entries based on search query
    fn filtered_entries(&self) -> Vec<(usize, &ModelEntry)> {
        if self.search_query.is_empty() {
            self.entries.iter().enumerate().collect()
        } else {
            let query = self.search_query.to_lowercase();
            let mut result = Vec::new();
            let mut current_header: Option<(usize, &ModelEntry)> = None;
            let mut current_sub_header: Option<(usize, &ModelEntry)> = None;
            let mut header_has_matches = false;
            let mut sub_header_has_matches = false;

            for (idx, entry) in self.entries.iter().enumerate() {
                match entry {
                    ModelEntry::Header { .. } => {
                        current_header = Some((idx, entry));
                        current_sub_header = None;
                        header_has_matches = false;
                        sub_header_has_matches = false;
                    }
                    ModelEntry::SubHeader { .. } => {
                        current_sub_header = Some((idx, entry));
                        sub_header_has_matches = false;
                    }
                    ModelEntry::Model { metadata } => {
                        // Also search in sub_provider
                        let sub_provider_match = metadata
                            .sub_provider
                            .as_ref()
                            .map(|s| s.to_lowercase().contains(&query))
                            .unwrap_or(false);

                        let matches = metadata.id.to_lowercase().contains(&query)
                            || metadata.display_name.to_lowercase().contains(&query)
                            || metadata
                                .provider
                                .to_string()
                                .to_lowercase()
                                .contains(&query)
                            || sub_provider_match;

                        if matches {
                            // Include header if this is first match for it
                            if !header_has_matches {
                                if let Some(h) = current_header {
                                    result.push(h);
                                }
                                header_has_matches = true;
                            }
                            // Include sub-header if this is first match for it
                            if !sub_header_has_matches {
                                if let Some(sh) = current_sub_header {
                                    result.push(sh);
                                }
                                sub_header_has_matches = true;
                            }
                            result.push((idx, entry));
                        }
                    }
                }
            }
            result
        }
    }

    /// Get selectable entries (models only, not headers)
    fn selectable_indices(&self) -> Vec<usize> {
        self.filtered_entries()
            .iter()
            .enumerate()
            .filter_map(|(filtered_idx, (_, entry))| {
                matches!(entry, ModelEntry::Model { .. }).then_some(filtered_idx)
            })
            .collect()
    }

    pub fn next(&mut self) {
        self.error = None; // Clear any error when navigating
        let selectable = self.selectable_indices();
        if selectable.is_empty() {
            return;
        }

        if let Some(pos) = selectable.iter().position(|&i| i == self.selected_index) {
            if pos + 1 < selectable.len() {
                self.selected_index = selectable[pos + 1];
            }
        } else if let Some(&next) = selectable.iter().find(|&&i| i > self.selected_index) {
            self.selected_index = next;
        }
        self.ensure_visible();
    }

    pub fn prev(&mut self) {
        self.error = None; // Clear any error when navigating
        let selectable = self.selectable_indices();
        if selectable.is_empty() {
            return;
        }

        if let Some(pos) = selectable.iter().position(|&i| i == self.selected_index) {
            if pos > 0 {
                self.selected_index = selectable[pos - 1];
            }
        } else if let Some(&prev) = selectable.iter().rev().find(|&&i| i < self.selected_index) {
            self.selected_index = prev;
        }
        self.ensure_visible();
    }

    fn ensure_visible(&mut self) {
        self.ensure_visible_with_height(12); // Default fallback
    }

    fn ensure_visible_with_height(&mut self, visible_height: usize) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index - visible_height + 1;
        }
    }

    pub fn toggle_search(&mut self) {
        self.search_active = !self.search_active;
        if !self.search_active {
            self.search_query.clear();
            self.selected_index = 0;
            self.scroll_offset = 0;
            // Re-select first model
            let selectable = self.selectable_indices();
            if let Some(&first) = selectable.first() {
                self.selected_index = first;
            }
        }
    }

    pub fn add_search_char(&mut self, c: char) {
        if self.search_active {
            self.search_query.push(c);
            let selectable = self.selectable_indices();
            self.selected_index = selectable.first().copied().unwrap_or(0);
            self.scroll_offset = 0;
        }
    }

    pub fn backspace_search(&mut self) {
        if self.search_active {
            self.search_query.pop();
            let selectable = self.selectable_indices();
            self.selected_index = selectable.first().copied().unwrap_or(0);
            self.scroll_offset = 0;
        }
    }

    /// Close search but keep filtered results (for Enter key)
    pub fn close_search(&mut self) {
        self.search_active = false;
        // DON'T clear search_query - keep filtered results for navigation
        // Re-select first visible model
        let selectable = self.selectable_indices();
        if let Some(&first) = selectable.first() {
            self.selected_index = first;
        }
    }

    pub fn has_selectable_results(&self) -> bool {
        !self.selectable_indices().is_empty()
    }

    /// Add OpenRouter models grouped by sub-provider
    fn add_openrouter_models(&mut self, models: Vec<ModelMetadata>) {
        use std::collections::HashMap;

        // Group by sub-provider
        let mut by_sub: HashMap<String, Vec<ModelMetadata>> = HashMap::new();
        for model in &models {
            let key = model
                .sub_provider
                .clone()
                .unwrap_or_else(|| "other".to_string());
            by_sub.entry(key).or_default().push(model.clone());
        }

        // Add header for OpenRouter section
        self.entries.push(ModelEntry::Header {
            name: "OPENROUTER".to_string(),
            count: Some(models.len()),
        });

        // Define sub-provider order (major providers first)
        let sub_order = [
            "anthropic",
            "openai",
            "google",
            "meta-llama",
            "mistralai",
            "deepseek",
            "qwen",
            "x-ai",
            "cohere",
            "nvidia",
            "perplexity",
            "databricks",
        ];

        // Add sub-headers for each sub-provider in order
        for sub in sub_order {
            if let Some(sub_models) = by_sub.remove(sub) {
                self.entries.push(ModelEntry::SubHeader {
                    name: sub.to_uppercase(),
                });
                for model in sub_models {
                    self.entries.push(ModelEntry::Model { metadata: model });
                }
            }
        }

        // Add remaining sub-providers (if any)
        let mut remaining: Vec<_> = by_sub.into_iter().collect();
        remaining.sort_by(|a, b| a.0.cmp(&b.0));
        for (sub, sub_models) in remaining {
            self.entries.push(ModelEntry::SubHeader {
                name: sub.to_uppercase(),
            });
            for model in sub_models {
                self.entries.push(ModelEntry::Model { metadata: model });
            }
        }
    }

    /// Get the full metadata for the selected model
    pub fn get_selected_metadata(&self) -> Option<&ModelMetadata> {
        let filtered = self.filtered_entries();
        filtered
            .get(self.selected_index)
            .and_then(|(_, entry)| match entry {
                ModelEntry::Model { metadata } => Some(metadata),
                ModelEntry::Header { .. } | ModelEntry::SubHeader { .. } => None,
            })
    }

    pub fn custom_model_metadata(&self, provider: ProviderId) -> Option<ModelMetadata> {
        let custom_id = self.search_query.trim();
        if custom_id.is_empty() || self.has_selectable_results() {
            return None;
        }

        Some(infer_model_metadata(
            provider,
            custom_id,
            detect_api_format(provider, custom_id),
        ))
    }

    /// Count total models (not headers)
    fn model_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, ModelEntry::Model { .. }))
            .count()
    }

    pub fn render(
        &self,
        f: &mut Frame,
        theme: &Theme,
        active_provider: ProviderId,
        current_model: &str,
        context_tokens_used: usize,
    ) {
        let (w, h) = PopupSize::Large.dimensions();
        let area = center_rect(w, h, f.area());
        render_popup_background(f, area, theme);

        let block = popup_block(theme);
        let inner = block.inner(area);
        f.render_widget(block, area);

        // Calculate search bar height
        let search_height = if self.search_active { 2 } else { 0 };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),             // Title
                Constraint::Length(search_height), // Search
                Constraint::Min(5),                // Content
                Constraint::Length(2),             // Footer
            ])
            .split(inner);

        // Calculate dynamic visible height from content area
        // Reserve 2 lines for scroll indicators
        let visible_height = (chunks[2].height as usize).saturating_sub(2);

        // Title
        let filtered = self.filtered_entries();
        let filtered_models: usize = filtered
            .iter()
            .filter(|(_, e)| matches!(e, ModelEntry::Model { .. }))
            .count();

        let title_text = if self.loading {
            "Loading Models...".to_string()
        } else if !self.search_query.is_empty() {
            format!("Models ({}/{})", filtered_models, self.model_count())
        } else {
            format!("Models ({})", self.model_count())
        };
        let title_lines = popup_title(&title_text, theme);
        let title = Paragraph::new(title_lines).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Search bar
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

        // Model list or loading/error state
        let mut lines: Vec<Line> = Vec::new();

        if self.loading {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "    Fetching models from providers...",
                Style::default().fg(theme.dim_color),
            )]));
        } else if let Some(ref error) = self.error {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("    Error: {}", error),
                Style::default().fg(theme.error_color),
            )]));
        } else if filtered.is_empty() && !self.search_query.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "    No catalog match. Press Enter to use '{}' on {}.",
                    self.search_query, active_provider
                ),
                Style::default().fg(theme.accent_color),
            )]));
        } else if self.entries.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "    No models available. Configure a provider first.",
                Style::default().fg(theme.dim_color),
            )]));
        } else {
            // Scroll up indicator
            if self.scroll_offset > 0 {
                lines.push(scroll_indicator("up", self.scroll_offset, theme));
            }

            // Visible entries
            let visible_end = (self.scroll_offset + visible_height).min(filtered.len());
            for (display_idx, (_, entry)) in filtered
                .iter()
                .enumerate()
                .skip(self.scroll_offset)
                .take(visible_height)
            {
                match entry {
                    ModelEntry::Header { name, count } => {
                        let header_text = match count {
                            Some(n) => format!("  {} ({})", name, n),
                            None => format!("  {}", name),
                        };
                        lines.push(Line::from(vec![Span::styled(
                            header_text,
                            Style::default()
                                .fg(theme.dim_color)
                                .add_modifier(Modifier::BOLD),
                        )]));
                    }
                    ModelEntry::SubHeader { name } => {
                        // Sub-header with extra indent
                        lines.push(Line::from(vec![Span::styled(
                            format!("    {}", name),
                            Style::default().fg(theme.dim_color),
                        )]));
                    }
                    ModelEntry::Model { metadata } => {
                        let is_selected = display_idx == self.selected_index;
                        let is_current = metadata.id == current_model;
                        let is_too_small = context_tokens_used > metadata.context_window;

                        // Fixed-width prefix (6 chars)
                        let prefix = if is_selected {
                            "    ▶ "
                        } else if is_current {
                            "    ✓ "
                        } else if is_too_small {
                            "    ⊘ " // Indicates model context is too small
                        } else {
                            "      "
                        };

                        let style = if is_too_small {
                            // Gray out models that can't fit current context
                            Style::default().fg(theme.dim_color)
                        } else if is_selected {
                            Style::default()
                                .fg(theme.accent_color)
                                .add_modifier(Modifier::BOLD)
                        } else if is_current {
                            Style::default().fg(theme.success_color)
                        } else {
                            Style::default().fg(theme.text_color)
                        };

                        // Fixed-width model name column (25 chars)
                        let name_width = 25;
                        let char_count = metadata.display_name.chars().count();
                        let display_name = if char_count > name_width {
                            let truncated: String =
                                metadata.display_name.chars().take(name_width - 1).collect();
                            format!("{}…", truncated)
                        } else {
                            format!("{:<width$}", metadata.display_name, width = name_width)
                        };

                        let mut spans = vec![
                            Span::styled(prefix, style),
                            Span::styled(display_name, style),
                        ];

                        // Fixed-width indicators column (6 chars: 🧠👁 or spaces)
                        let thinking = if metadata.supports_thinking {
                            "🧠"
                        } else {
                            "  "
                        };
                        let vision = if metadata.supports_vision {
                            "👁"
                        } else {
                            "  "
                        };
                        spans.push(Span::styled(
                            format!(" {}{}", thinking, vision),
                            Style::default().fg(theme.accent_color),
                        ));

                        // Fixed-width context column (6 chars right-aligned)
                        spans.push(Span::styled(
                            format!(" {:>6}", metadata.context_display()),
                            Style::default().fg(theme.dim_color),
                        ));

                        // Fixed-width pricing column (4 chars)
                        let pricing = if metadata.is_free {
                            " FREE".to_string()
                        } else {
                            let tier = metadata.pricing_tier();
                            format!(" {:>4}", tier)
                        };
                        let pricing_color = if metadata.is_free {
                            theme.success_color
                        } else {
                            theme.dim_color
                        };
                        spans.push(Span::styled(pricing, Style::default().fg(pricing_color)));

                        // Current indicator (appended, not fixed width)
                        if is_current && !is_selected {
                            spans
                                .push(Span::styled(" ←", Style::default().fg(theme.success_color)));
                        }

                        lines.push(Line::from(spans));
                    }
                }
            }

            // Scroll down indicator
            let remaining = filtered.len().saturating_sub(visible_end);
            if remaining > 0 {
                lines.push(scroll_indicator("down", remaining, theme));
            }
        }

        let content = Paragraph::new(lines).style(Style::default().bg(theme.bg_color));
        let content_area = center_content(chunks[2], 4);
        f.render_widget(content, content_area);

        // Footer
        let footer = if self.search_active {
            let enter_hint = if self.has_selectable_results() {
                "close search"
            } else {
                "use query as model id"
            };
            Paragraph::new(Line::from(vec![
                Span::styled("Type to search  ", Style::default().fg(theme.text_color)),
                Span::styled(
                    "Enter",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(": {}  ", enter_hint),
                    Style::default().fg(theme.text_color),
                ),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(": cancel", Style::default().fg(theme.text_color)),
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
                Span::styled(": select", Style::default().fg(theme.text_color)),
            ]))
        };
        f.render_widget(footer.alignment(Alignment::Center), chunks[3]);
    }
}
