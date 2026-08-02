use super::*;

impl App {
    /// Update hover state based on mouse position
    pub(super) fn update_hover_state(&mut self, x: u16, y: u16) {
        // Always update mouse position (cheap)
        self.ui.scroll_system.hover.mouse_pos = Some((x, y));

        // Check if hovering over plugin divider (cheap check, no throttle needed)
        self.ui.scroll_system.layout.plugin_divider_hovered =
            if let Some(area) = self.ui.scroll_system.layout.plugin_divider_area {
                area.contains(Position::new(x, y))
            } else {
                false
            };

        // Throttle expensive detection operations
        if !self.ui.scroll_system.hover.should_detect() {
            return;
        }

        // Check messages area for file references
        self.ui.scroll_system.hover.message_file_ref = self.detect_message_file_ref(x, y);

        // Check messages area for hyperlinks
        self.ui.scroll_system.hover.message_link = self.detect_message_link(x, y);

        // Check input area for file references
        self.ui.scroll_system.hover.input_file_ref = self.detect_input_file_ref(x, y);
    }

    /// Detect file reference in messages at position
    pub(super) fn detect_message_file_ref(&self, x: u16, y: u16) -> Option<(usize, String)> {
        use regex::Regex;
        use std::sync::LazyLock;

        static FILE_REF_PATTERN: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\[(Image|PDF): ([^\]]+)\]").unwrap());

        let area = self.ui.scroll_system.layout.messages_area?;
        if !area.contains(Position::new(x, y)) {
            return None;
        }

        let (line_idx, _col) = self.hit_test_messages(x, y)?;

        let wrap_width = area.width.saturating_sub(6) as usize;
        let mut current_line = 0usize;

        for (msg_idx, (role, content)) in self.runtime.chat.messages.iter().enumerate() {
            if role == "user" || role == "system" {
                let mut msg_lines = 0usize;
                for line in content.lines() {
                    if line.is_empty() {
                        msg_lines += 1;
                    } else {
                        msg_lines += crate::tui::utils::count_wrapped_lines(line, wrap_width);
                    }
                }
                msg_lines += 1;

                if line_idx >= current_line && line_idx < current_line + msg_lines {
                    if let Some(caps) = FILE_REF_PATTERN.captures(content) {
                        let display_name = caps.get(2).map(|m| m.as_str().to_string())?;
                        return Some((msg_idx, display_name));
                    }
                }
                current_line += msg_lines;
            } else if role == "assistant" {
                let line_count = self.get_markdown_line_count(content, wrap_width);
                current_line += line_count + 1;
            } else {
                current_line += 1;
            }
        }

        None
    }

    /// Detect file reference in input at position
    pub(super) fn detect_input_file_ref(
        &self,
        x: u16,
        y: u16,
    ) -> Option<(usize, usize, std::path::PathBuf)> {
        let area = self.ui.scroll_system.layout.input_area?;
        if !area.contains(Position::new(x, y)) {
            return None;
        }

        // Get byte position from click
        let (_line, _col) = self.hit_test_input(x, y)?;

        // Check if there's a file segment at this position
        self.ui
            .input
            .get_file_ref_at_click(x.saturating_sub(area.x), y.saturating_sub(area.y))
    }

    /// Detect hyperlink in messages at position
    pub(super) fn detect_message_link(
        &self,
        x: u16,
        y: u16,
    ) -> Option<crate::tui::state::HoveredLink> {
        use crate::tui::state::HoveredLink;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Local hash function matching markdown cache key format
        fn hash_content(s: &str) -> u64 {
            let mut hasher = DefaultHasher::new();
            s.hash(&mut hasher);
            hasher.finish()
        }

        let area = self.ui.scroll_system.layout.messages_area?;
        if !area.contains(Position::new(x, y)) {
            return None;
        }

        let (line_idx, col) = self.hit_test_messages(x, y)?;

        let wrap_width = area.width.saturating_sub(6) as usize;
        let mut current_line = 0usize;

        for (msg_idx, (role, content)) in self.runtime.chat.messages.iter().enumerate() {
            if role == "assistant" {
                // Get rendered markdown from cache
                let content_hash = hash_content(content);
                if let Some(rendered) = self
                    .ui
                    .markdown_cache
                    .get_rendered(content_hash, wrap_width)
                {
                    let msg_line_count = rendered.lines.len();

                    // Check if click is within this message's lines
                    if line_idx >= current_line && line_idx < current_line + msg_line_count {
                        let relative_line = line_idx - current_line;

                        // Check if any link spans contain this position
                        for link in &rendered.links {
                            if link.line == relative_line
                                && col >= link.start_col
                                && col < link.end_col
                            {
                                return Some(HoveredLink {
                                    msg_idx,
                                    line: relative_line,
                                    start_col: link.start_col,
                                    end_col: link.end_col,
                                    url: link.url.clone(),
                                });
                            }
                        }
                    }
                    current_line += msg_line_count + 1; // +1 for blank line
                } else {
                    // Fallback line count
                    let line_count = self.get_markdown_line_count(content, wrap_width);
                    current_line += line_count + 1;
                }
            } else {
                // User/system messages
                let mut msg_lines = 0usize;
                for line in content.lines() {
                    if line.is_empty() {
                        msg_lines += 1;
                    } else {
                        msg_lines += crate::tui::utils::count_wrapped_lines(line, wrap_width);
                    }
                }
                msg_lines += 1;
                current_line += msg_lines;
            }
        }

        None
    }

    /// Try to open a hyperlink at the click position
    /// Returns true if a link was clicked and opened
    pub(super) fn try_open_link(&mut self, x: u16, y: u16) -> bool {
        if let Some(link) = self.detect_message_link(x, y) {
            // Open the URL in the default browser
            if let Err(e) = webbrowser::open(&link.url) {
                tracing::warn!("Failed to open URL {}: {}", link.url, e);
            }
            return true;
        }
        false
    }

    /// Try to detect and open a file preview from a click position
    /// Returns true if a file reference was clicked and preview opened
    pub(super) fn try_open_file_preview(&mut self, x: u16, y: u16) -> bool {
        use regex::Regex;
        use std::sync::LazyLock;

        static FILE_REF_PATTERN: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\[(Image|PDF): ([^\]]+)\]").unwrap());

        // Check if click is in messages area
        let Some(area) = self.ui.scroll_system.layout.messages_area else {
            return false;
        };

        if !area.contains(Position::new(x, y)) {
            return false;
        }

        // Get the clicked line position
        let Some((line_idx, _col)) = self.hit_test_messages(x, y) else {
            return false;
        };

        // Find the message content at this line
        let wrap_width = area.width.saturating_sub(6) as usize;
        let mut current_line = 0usize;

        for (role, content) in &self.runtime.chat.messages {
            if role == "user" || role == "system" {
                // Calculate lines in this message
                let mut msg_lines = 0usize;
                for line in content.lines() {
                    if line.is_empty() {
                        msg_lines += 1;
                    } else {
                        msg_lines += crate::tui::utils::count_wrapped_lines(line, wrap_width);
                    }
                }
                msg_lines += 1; // blank line

                // Check if clicked line is within this message
                if line_idx >= current_line && line_idx < current_line + msg_lines {
                    // Check for file reference in the content
                    if let Some(caps) = FILE_REF_PATTERN.captures(content) {
                        let display_name = caps.get(2).map(|m| m.as_str()).unwrap_or("");

                        // Look up the file path
                        if let Some(path) = self.runtime.attached_files.get(display_name) {
                            // Open the preview popup
                            self.ui.popups.file_preview.open(path.clone());
                            self.ui.popup = Popup::FilePreview;
                            return true;
                        }
                    }
                }

                current_line += msg_lines;
            } else if role == "assistant" {
                let line_count = self.get_markdown_line_count(content, wrap_width);
                current_line += line_count + 1;
            } else {
                // Skip blocks (thinking, bash, etc.)
                current_line += 1;
            }
        }

        false
    }
}
