//! Slash command autocomplete with fuzzy matching

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem},
    Frame,
};

use crate::tui::themes::Theme;

#[derive(Debug, Clone)]
pub struct CommandSuggestion {
    pub primary: &'static str,
    pub aliases: Vec<&'static str>,
    pub description: &'static str,
}

/// Autocomplete popup for slash commands
#[derive(Debug, Clone)]
pub struct AutocompletePopup {
    pub suggestions: Vec<CommandSuggestion>,
    pub filtered: Vec<(usize, i32)>, // (index, score)
    pub selected: usize,
    pub visible: bool,
    pub query: String,
}

impl Default for AutocompletePopup {
    fn default() -> Self {
        Self::new()
    }
}

impl AutocompletePopup {
    pub fn new() -> Self {
        Self {
            suggestions: get_all_commands(),
            filtered: Vec::new(),
            selected: 0,
            visible: false,
            query: String::new(),
        }
    }

    pub fn show(&mut self, query: &str) {
        self.query = query.to_string();
        self.visible = true;
        self.filter();
        self.selected = 0;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected = 0;
    }

    pub fn update(&mut self, query: &str) {
        self.query = query.to_string();
        self.filter();
        if self.selected >= self.filtered.len() {
            self.selected = 0;
        }
    }

    pub fn next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.filtered.len() - 1);
        }
    }

    pub fn get_selected(&self) -> Option<&CommandSuggestion> {
        self.filtered
            .get(self.selected)
            .and_then(|(idx, _)| self.suggestions.get(*idx))
    }

    pub fn has_suggestions(&self) -> bool {
        !self.filtered.is_empty()
    }

    fn filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = self
                .suggestions
                .iter()
                .enumerate()
                .map(|(i, _)| (i, 100))
                .collect();
            return;
        }

        let query = self.query.to_lowercase();
        let mut scored: Vec<(usize, i32)> = Vec::new();

        for (idx, cmd) in self.suggestions.iter().enumerate() {
            let mut best = 0;

            // Match primary command (strip /)
            let primary = cmd.primary.trim_start_matches('/').to_lowercase();
            if let Some(score) = fuzzy_match(&primary, &query) {
                best = best.max(score + 20);
            }

            // Match aliases
            for alias in &cmd.aliases {
                if let Some(score) = fuzzy_match(alias, &query) {
                    best = best.max(score + 10);
                }
            }

            // Match description
            if let Some(score) = fuzzy_match(&cmd.description.to_lowercase(), &query) {
                best = best.max(score);
            }

            if best > 0 {
                scored.push((idx, best));
            }
        }

        scored.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        self.filtered = scored;
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        f.render_widget(Clear, area);

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .take(7)
            .enumerate()
            .map(|(i, (idx, _))| {
                let cmd = &self.suggestions[*idx];
                let is_selected = i == self.selected;

                let mut spans = vec![];

                if is_selected {
                    spans.push(Span::styled(" › ", Style::default().fg(theme.accent_color)));
                } else {
                    spans.push(Span::raw("   "));
                }

                spans.push(Span::styled(
                    cmd.primary,
                    Style::default()
                        .fg(theme.accent_color)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    cmd.description,
                    Style::default().fg(theme.text_color),
                ));

                let line = Line::from(spans);
                if is_selected {
                    ListItem::new(line).style(Style::default().bg(theme.border_color))
                } else {
                    ListItem::new(line)
                }
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border_color))
                .style(Style::default().bg(theme.bg_color)),
        );

        f.render_widget(list, area);
    }
}

/// Simple fuzzy match scoring
fn fuzzy_match(text: &str, pattern: &str) -> Option<i32> {
    if pattern.is_empty() {
        return Some(100);
    }

    // Exact match
    if text == pattern {
        return Some(200);
    }

    // Prefix match
    if text.starts_with(pattern) {
        return Some(150);
    }

    // Contains match
    if text.contains(pattern) {
        return Some(100);
    }

    // Character-by-character fuzzy
    let mut pattern_chars = pattern.chars();
    let mut current = pattern_chars.next()?;
    let mut score = 0;
    let mut consecutive = 0;

    for (i, ch) in text.chars().enumerate() {
        if ch == current {
            score += 10 + consecutive * 5;
            consecutive += 1;
            if let Some(next) = pattern_chars.next() {
                current = next;
            } else {
                return Some(score - i as i32);
            }
        } else {
            consecutive = 0;
        }
    }

    None
}

/// All available slash commands
pub fn get_all_commands() -> Vec<CommandSuggestion> {
    vec![
        CommandSuggestion {
            primary: "/home",
            aliases: vec![],
            description: "Return to start menu",
        },
        CommandSuggestion {
            primary: "/load",
            aliases: vec![],
            description: "Load previous session",
        },
        CommandSuggestion {
            primary: "/model",
            aliases: vec![],
            description: "Select AI model",
        },
        CommandSuggestion {
            primary: "/fast",
            aliases: vec![],
            description: "Toggle fast service tier",
        },
        CommandSuggestion {
            primary: "/auth",
            aliases: vec![],
            description: "Manage API providers",
        },
        CommandSuggestion {
            primary: "/init",
            aliases: vec![],
            description: "Initialize project (create KRAB.md)",
        },
        CommandSuggestion {
            primary: "/theme",
            aliases: vec![],
            description: "Change color theme",
        },
        CommandSuggestion {
            primary: "/clear",
            aliases: vec![],
            description: "Clear chat messages",
        },
        CommandSuggestion {
            primary: "/pinch",
            aliases: vec![],
            description: "Compact this session in place",
        },
        CommandSuggestion {
            primary: "/cmd",
            aliases: vec![],
            description: "Show all controls",
        },
        CommandSuggestion {
            primary: "/terminal",
            aliases: vec!["term", "shell"],
            description: "Open interactive terminal",
        },
        CommandSuggestion {
            primary: "/ps",
            aliases: vec!["processes"],
            description: "View background processes",
        },
        CommandSuggestion {
            primary: "/skills",
            aliases: vec![],
            description: "Browse and manage skills",
        },
        CommandSuggestion {
            primary: "/plugins",
            aliases: vec![],
            description: "Browse and manage installable plugins",
        },
        CommandSuggestion {
            primary: "/plan",
            aliases: vec![],
            description: "View or manage active plan",
        },
        CommandSuggestion {
            primary: "/mcp",
            aliases: vec![],
            description: "Browse and manage MCP servers",
        },
        CommandSuggestion {
            primary: "/hooks",
            aliases: vec![],
            description: "Configure tool execution hooks",
        },
        CommandSuggestion {
            primary: "/permissions",
            aliases: vec!["perm"],
            description: "Toggle supervised/autonomous mode",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_match() {
        assert_eq!(fuzzy_match("model", "model"), Some(200));
        assert!(fuzzy_match("model", "mod").unwrap() > 100);
        assert!(fuzzy_match("model", "mdl").is_some());
        assert!(fuzzy_match("model", "xyz").is_none());
    }

    #[test]
    fn test_autocomplete() {
        let mut ac = AutocompletePopup::new();
        ac.show("mod");
        assert!(!ac.filtered.is_empty());
        let first = ac.get_selected().unwrap();
        assert_eq!(first.primary, "/model");
    }
}
