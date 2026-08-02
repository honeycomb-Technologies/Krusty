//! File search with fuzzy matching and tree browser
//!
//! Triggered by `@` in input, similar to slash command autocomplete.
//! Two modes:
//! - Fuzzy search: type to filter files
//! - Tree browser: expandable/collapsible directory tree

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::tui::themes::Theme;

/// File search mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSearchMode {
    /// Fuzzy search across all files
    Fuzzy,
    /// Tree browser navigation
    Tree,
}

/// A file entry for display (used in fuzzy mode)
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Relative path from working directory
    pub path: String,
    /// Just the filename
    pub name: String,
    /// Is this a directory?
    pub is_dir: bool,
}

/// A visible entry in the tree (includes depth for indentation)
#[derive(Debug, Clone)]
pub struct TreeEntry {
    /// Relative path from working directory
    pub path: String,
    /// Just the filename
    pub name: String,
    /// Is this a directory?
    pub is_dir: bool,
    /// Depth in tree (for indentation)
    pub depth: usize,
    /// Is this directory expanded?
    pub expanded: bool,
}

/// Tree browser state with expand/collapse
#[derive(Debug, Clone)]
pub struct TreeState {
    /// Set of expanded directory paths
    pub expanded: HashSet<String>,
    /// Flattened list of visible entries
    pub visible: Vec<TreeEntry>,
    /// Selected index in visible list
    pub selected: usize,
    /// Scroll offset for long lists
    pub scroll_offset: usize,
}

impl TreeState {
    fn new() -> Self {
        Self {
            expanded: HashSet::new(),
            visible: Vec::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }
}

/// File search popup
#[derive(Debug, Clone)]
pub struct FileSearchPopup {
    /// Current mode
    pub mode: FileSearchMode,
    /// Search query (fuzzy mode)
    pub query: String,
    /// All indexed files
    files: Vec<FileEntry>,
    /// Filtered results with scores (fuzzy mode)
    filtered: Vec<(usize, i32)>,
    /// Selected index (fuzzy mode)
    pub selected: usize,
    /// Scroll offset (fuzzy mode)
    scroll_offset: usize,
    /// Tree browser state
    tree: TreeState,
    /// Whether popup is visible
    pub visible: bool,
    /// Working directory root
    working_dir: PathBuf,
    /// Area of the toggle button (for click detection)
    pub toggle_button_area: Option<Rect>,
}

impl FileSearchPopup {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            mode: FileSearchMode::Fuzzy,
            query: String::new(),
            files: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            tree: TreeState::new(),
            visible: false,
            working_dir,
            toggle_button_area: None,
        }
    }

    /// Index files in working directory (respects .gitignore patterns)
    pub fn index_files(&mut self) {
        self.files.clear();
        self.index_dir(&self.working_dir.clone(), &PathBuf::new());
        // Sort files: directories first, then by name
        self.files.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.path.cmp(&b.path),
        });
    }

    fn index_dir(&mut self, abs_path: &Path, rel_path: &Path) {
        let Ok(entries) = std::fs::read_dir(abs_path) else {
            return;
        };

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();

            // Skip hidden files and common ignore patterns
            if name.starts_with('.')
                || name == "node_modules"
                || name == "target"
                || name == "__pycache__"
                || name == "venv"
                || name == ".git"
                || name == "dist"
                || name == "build"
            {
                continue;
            }

            let path = entry.path();
            let rel = rel_path.join(&name);
            let is_dir = path.is_dir();

            self.files.push(FileEntry {
                path: rel.to_string_lossy().into_owned(),
                name,
                is_dir,
            });

            // Recurse into directories
            if is_dir {
                self.index_dir(&path, &rel);
            }
        }
    }

    /// Show the popup with a query
    pub fn show(&mut self, query: &str) {
        self.query = query.to_string();
        self.visible = true;
        self.mode = FileSearchMode::Fuzzy;

        // Index files if not already done
        if self.files.is_empty() {
            self.index_files();
        }

        self.filter();
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Hide the popup
    pub fn hide(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Update query and re-filter
    pub fn update(&mut self, query: &str) {
        self.query = query.to_string();
        if self.mode == FileSearchMode::Fuzzy {
            self.filter();
            if self.selected >= self.filtered.len() {
                self.selected = 0;
            }
            self.scroll_offset = 0;
        }
    }

    /// Toggle between fuzzy and tree mode
    pub fn toggle_mode(&mut self) {
        match self.mode {
            FileSearchMode::Fuzzy => {
                self.mode = FileSearchMode::Tree;
                self.build_tree();
            }
            FileSearchMode::Tree => {
                self.mode = FileSearchMode::Fuzzy;
                self.filter();
                self.selected = 0;
                self.scroll_offset = 0;
            }
        }
    }

    /// Check if click is on toggle button
    pub fn is_toggle_button_click(&self, x: u16, y: u16) -> bool {
        if let Some(area) = self.toggle_button_area {
            x >= area.x && x < area.x + area.width && y >= area.y && y < area.y + area.height
        } else {
            false
        }
    }

    /// Build the visible tree from the working directory
    fn build_tree(&mut self) {
        self.tree.visible.clear();
        self.tree.selected = 0;
        self.tree.scroll_offset = 0;
        self.build_tree_recursive(&self.working_dir.clone(), &PathBuf::new(), 0);
    }

    /// Recursively build tree entries
    fn build_tree_recursive(&mut self, abs_path: &Path, rel_path: &Path, depth: usize) {
        let Ok(entries) = std::fs::read_dir(abs_path) else {
            return;
        };

        let mut items: Vec<(String, PathBuf, bool)> = Vec::new();

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();

            // Skip hidden files and common ignores
            if name.starts_with('.')
                || name == "node_modules"
                || name == "target"
                || name == "__pycache__"
                || name == "venv"
                || name == "dist"
                || name == "build"
            {
                continue;
            }

            let path = entry.path();
            let rel = rel_path.join(&name);
            let is_dir = path.is_dir();
            items.push((name, rel, is_dir));
        }

        // Sort: directories first, then by name
        items.sort_by(|a, b| match (a.2, b.2) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.0.cmp(&b.0),
        });

        for (name, rel, is_dir) in items {
            let path_str = rel.to_string_lossy().into_owned();
            let expanded = self.tree.expanded.contains(&path_str);

            self.tree.visible.push(TreeEntry {
                path: path_str.clone(),
                name,
                is_dir,
                depth,
                expanded,
            });

            // If directory is expanded, recurse into it
            if is_dir && expanded {
                let child_abs = abs_path.join(rel.file_name().unwrap_or_default());
                self.build_tree_recursive(&child_abs, &rel, depth + 1);
            }
        }
    }

    /// Navigate to next item
    pub fn next(&mut self) {
        match self.mode {
            FileSearchMode::Fuzzy => {
                if !self.filtered.is_empty() {
                    self.selected = (self.selected + 1) % self.filtered.len();
                    self.ensure_visible_fuzzy();
                }
            }
            FileSearchMode::Tree => {
                if !self.tree.visible.is_empty() {
                    self.tree.selected = (self.tree.selected + 1) % self.tree.visible.len();
                    self.ensure_visible_tree();
                }
            }
        }
    }

    /// Navigate to previous item
    pub fn prev(&mut self) {
        match self.mode {
            FileSearchMode::Fuzzy => {
                if !self.filtered.is_empty() {
                    if self.selected == 0 {
                        self.selected = self.filtered.len() - 1;
                    } else {
                        self.selected -= 1;
                    }
                    self.ensure_visible_fuzzy();
                }
            }
            FileSearchMode::Tree => {
                if !self.tree.visible.is_empty() {
                    if self.tree.selected == 0 {
                        self.tree.selected = self.tree.visible.len() - 1;
                    } else {
                        self.tree.selected -= 1;
                    }
                    self.ensure_visible_tree();
                }
            }
        }
    }

    fn ensure_visible_fuzzy(&mut self) {
        let visible_count = 8;
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible_count {
            self.scroll_offset = self.selected - visible_count + 1;
        }
    }

    fn ensure_visible_tree(&mut self) {
        let visible_count = 8;
        if self.tree.selected < self.tree.scroll_offset {
            self.tree.scroll_offset = self.tree.selected;
        } else if self.tree.selected >= self.tree.scroll_offset + visible_count {
            self.tree.scroll_offset = self.tree.selected - visible_count + 1;
        }
    }

    /// Toggle expand/collapse on selected directory (Right arrow)
    pub fn enter_dir(&mut self) -> bool {
        if self.mode != FileSearchMode::Tree {
            return false;
        }

        if let Some(entry) = self.tree.visible.get(self.tree.selected).cloned() {
            if entry.is_dir {
                if self.tree.expanded.contains(&entry.path) {
                    // Already expanded - do nothing (Right on expanded = no-op)
                    return false;
                } else {
                    // Expand this directory
                    self.tree.expanded.insert(entry.path.clone());
                    self.rebuild_tree_preserving_selection(&entry.path);
                    return true;
                }
            }
        }
        false
    }

    /// Collapse selected directory or move to parent (Left arrow)
    pub fn go_up(&mut self) -> bool {
        if self.mode != FileSearchMode::Tree {
            return false;
        }

        if let Some(entry) = self.tree.visible.get(self.tree.selected).cloned() {
            if entry.is_dir && entry.expanded {
                // Collapse this directory
                self.tree.expanded.remove(&entry.path);
                self.rebuild_tree_preserving_selection(&entry.path);
                return true;
            } else if entry.depth > 0 {
                // Move selection to parent directory
                let parent_path = PathBuf::from(&entry.path)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(i) = self.tree.visible.iter().position(|e| e.path == parent_path) {
                    self.tree.selected = i;
                    self.ensure_visible_tree();
                    return true;
                }
            }
        }
        false
    }

    /// Rebuild tree and restore selection to the given path
    fn rebuild_tree_preserving_selection(&mut self, path: &str) {
        self.tree.visible.clear();
        self.build_tree_recursive(&self.working_dir.clone(), &PathBuf::new(), 0);

        // Re-select the path after rebuild
        if let Some(i) = self.tree.visible.iter().position(|e| e.path == path) {
            self.tree.selected = i;
            self.ensure_visible_tree();
            return;
        }
        // Fallback: keep selection in bounds
        if self.tree.selected >= self.tree.visible.len() {
            self.tree.selected = self.tree.visible.len().saturating_sub(1);
        }
    }

    /// Get the selected file/folder path
    pub fn get_selected(&self) -> Option<&str> {
        match self.mode {
            FileSearchMode::Fuzzy => self
                .filtered
                .get(self.selected)
                .and_then(|(idx, _)| self.files.get(*idx))
                .map(|f| f.path.as_str()),
            FileSearchMode::Tree => self
                .tree
                .visible
                .get(self.tree.selected)
                .map(|f| f.path.as_str()),
        }
    }

    /// Check if there are any results
    pub fn has_results(&self) -> bool {
        match self.mode {
            FileSearchMode::Fuzzy => !self.filtered.is_empty(),
            FileSearchMode::Tree => true, // Always show tree even if empty
        }
    }

    /// Filter files by fuzzy query
    fn filter(&mut self) {
        if self.query.is_empty() {
            // Show all files (limited)
            self.filtered = self
                .files
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.is_dir) // Only files in empty query
                .take(100)
                .map(|(i, _)| (i, 100))
                .collect();
            return;
        }

        let query = self.query.to_lowercase();
        let mut scored: Vec<(usize, i32)> = Vec::new();

        for (idx, file) in self.files.iter().enumerate() {
            let path_lower = file.path.to_lowercase();
            let name_lower = file.name.to_lowercase();

            let mut best = 0;

            // Exact name match
            if name_lower == query {
                best = 200;
            }
            // Name starts with query
            else if name_lower.starts_with(&query) {
                best = 150;
            }
            // Name contains query
            else if name_lower.contains(&query) {
                best = 100;
            }
            // Path contains query
            else if path_lower.contains(&query) {
                best = 80;
            }
            // Fuzzy match on name
            else if let Some(score) = fuzzy_match(&name_lower, &query) {
                best = score;
            }
            // Fuzzy match on path
            else if let Some(score) = fuzzy_match(&path_lower, &query) {
                best = score / 2;
            }

            if best > 0 {
                scored.push((idx, best));
            }
        }

        scored.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        self.filtered = scored.into_iter().take(50).collect();
    }

    /// Render the popup
    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        f.render_widget(Clear, area);

        // Main block
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_color))
            .style(Style::default().bg(theme.bg_color));

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Render content (full inner area)
        match self.mode {
            FileSearchMode::Fuzzy => self.render_fuzzy_content(f, inner, theme),
            FileSearchMode::Tree => self.render_tree_content(f, inner, theme),
        }

        // Render toggle button in top-right corner of border
        self.render_toggle_button(f, area, theme);
    }

    /// Render toggle button [≡] or [⋮] in top-right of border (like edit block)
    fn render_toggle_button(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        // Button position: 4 chars from right edge (includes border)
        // Format: [≡] for tree mode, [⋮] for search mode
        let button_x = area.x + area.width.saturating_sub(5);
        let button_y = area.y; // Top border row

        // Store button area for click detection
        self.toggle_button_area = Some(Rect::new(button_x, button_y, 3, 1));

        // Icon: ≡ (hamburger) for tree, ⋮ for search/fuzzy
        let icon = match self.mode {
            FileSearchMode::Fuzzy => '≡', // Switch to tree
            FileSearchMode::Tree => '⋮',  // Switch to search
        };

        // Render directly into buffer like edit block does
        let buf = f.buffer_mut();

        // [
        if let Some(cell) = buf.cell_mut((button_x, button_y)) {
            cell.set_char('[');
            cell.set_fg(theme.border_color);
        }
        // icon
        if let Some(cell) = buf.cell_mut((button_x + 1, button_y)) {
            cell.set_char(icon);
            cell.set_fg(theme.accent_color);
        }
        // ]
        if let Some(cell) = buf.cell_mut((button_x + 2, button_y)) {
            cell.set_char(']');
            cell.set_fg(theme.border_color);
        }
    }

    fn render_fuzzy_content(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let visible_count = area.height as usize;
        let mut lines: Vec<Line> = Vec::new();

        if self.filtered.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No matches",
                Style::default()
                    .fg(theme.dim_color)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else {
            for (display_idx, (file_idx, _)) in self
                .filtered
                .iter()
                .skip(self.scroll_offset)
                .take(visible_count)
                .enumerate()
            {
                let actual_idx = self.scroll_offset + display_idx;
                let file = &self.files[*file_idx];
                let is_selected = actual_idx == self.selected;

                let mut spans = vec![];

                // Selection indicator (matches slash command style)
                if is_selected {
                    spans.push(Span::styled(" › ", Style::default().fg(theme.accent_color)));
                } else {
                    spans.push(Span::raw("   "));
                }

                // Directory indicator
                if file.is_dir {
                    spans.push(Span::styled(
                        format!("{}/", file.path),
                        Style::default()
                            .fg(theme.accent_color)
                            .add_modifier(if is_selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ));
                } else {
                    spans.push(Span::styled(
                        &file.path,
                        Style::default()
                            .fg(theme.text_color)
                            .add_modifier(if is_selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ));
                }

                let line = Line::from(spans);
                lines.push(line);
            }
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, area);
    }

    fn render_tree_content(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let visible_count = area.height as usize;
        let mut lines: Vec<Line> = Vec::new();

        if self.tree.visible.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (empty)",
                Style::default()
                    .fg(theme.dim_color)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else {
            for (display_idx, entry) in self
                .tree
                .visible
                .iter()
                .skip(self.tree.scroll_offset)
                .take(visible_count)
                .enumerate()
            {
                let actual_idx = self.tree.scroll_offset + display_idx;
                let is_selected = actual_idx == self.tree.selected;

                let mut spans = vec![];

                // Selection indicator
                if is_selected {
                    spans.push(Span::styled(" › ", Style::default().fg(theme.accent_color)));
                } else {
                    spans.push(Span::raw("   "));
                }

                // Indentation based on depth (2 spaces per level)
                let indent = "  ".repeat(entry.depth);
                spans.push(Span::raw(indent));

                if entry.is_dir {
                    // Expand/collapse indicator
                    let indicator = if entry.expanded { "▼ " } else { "▶ " };
                    spans.push(Span::styled(
                        indicator,
                        Style::default().fg(theme.dim_color),
                    ));
                    // Directory name
                    spans.push(Span::styled(
                        &entry.name,
                        Style::default()
                            .fg(theme.accent_color)
                            .add_modifier(if is_selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ));
                } else {
                    // File - add spacing to align with directories
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        &entry.name,
                        Style::default()
                            .fg(theme.text_color)
                            .add_modifier(if is_selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ));
                }

                lines.push(Line::from(spans));
            }
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, area);
    }
}

/// Simple fuzzy match scoring
fn fuzzy_match(text: &str, pattern: &str) -> Option<i32> {
    if pattern.is_empty() {
        return Some(100);
    }

    let mut pattern_chars = pattern.chars().peekable();
    let mut current = pattern_chars.next()?;
    let mut score = 0;
    let mut consecutive = 0;

    for ch in text.chars() {
        if ch == current {
            score += 10 + consecutive * 5;
            consecutive += 1;
            if let Some(next) = pattern_chars.next() {
                current = next;
            } else {
                return Some(score);
            }
        } else {
            consecutive = 0;
        }
    }

    None
}
