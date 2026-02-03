//! Text selection handling
//!
//! Handles text selection, edge scrolling during selection, and clipboard operations.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::tui::app::App;
use crate::tui::blocks::StreamBlock;
use crate::tui::state::{EdgeScrollDirection, SelectionArea};
use crate::tui::utils::wrap_line;

impl App {
    /// Handle selection drag - updates selection and triggers edge scrolling
    ///
    /// Returns true if selection drag was handled.
    pub fn handle_selection_drag(&mut self, x: u16, y: u16) -> bool {
        if !self.ui.scroll_system.selection.is_selecting {
            return false;
        }

        match self.ui.scroll_system.selection.area {
            SelectionArea::Messages => {
                if let Some(area) = self.ui.scroll_system.layout.messages_area {
                    let edge_zone = 2; // rows from edge to trigger scroll

                    // Auto-scroll at edges and set continuous scroll state
                    if y <= area.y + edge_zone && self.ui.scroll_system.scroll.can_scroll_up() {
                        self.ui.scroll_system.scroll.scroll_up(1);
                        self.ui.scroll_system.edge_scroll.direction = Some(EdgeScrollDirection::Up);
                        self.ui.scroll_system.edge_scroll.area = SelectionArea::Messages;
                        self.ui.scroll_system.edge_scroll.last_x = x;
                    } else if y >= area.y + area.height.saturating_sub(edge_zone)
                        && self.ui.scroll_system.scroll.can_scroll_down()
                    {
                        self.ui.scroll_system.scroll.scroll_down(1);
                        self.ui.scroll_system.edge_scroll.direction =
                            Some(EdgeScrollDirection::Down);
                        self.ui.scroll_system.edge_scroll.area = SelectionArea::Messages;
                        self.ui.scroll_system.edge_scroll.last_x = x;
                    } else {
                        self.ui.scroll_system.edge_scroll.direction = None;
                    }

                    // Update selection end
                    if let Some(pos) = self.hit_test_messages(x, y) {
                        self.ui.scroll_system.selection.end = Some(pos);
                    }
                }
                true
            }
            SelectionArea::Input => {
                if let Some(area) = self.ui.scroll_system.layout.input_area {
                    let edge_zone = 1;

                    if y <= area.y + edge_zone {
                        self.ui.input.scroll_up();
                        self.ui.scroll_system.edge_scroll.direction = Some(EdgeScrollDirection::Up);
                        self.ui.scroll_system.edge_scroll.area = SelectionArea::Input;
                        self.ui.scroll_system.edge_scroll.last_x = x;
                    } else if y >= area.y + area.height.saturating_sub(edge_zone) {
                        self.ui.input.scroll_down();
                        self.ui.scroll_system.edge_scroll.direction =
                            Some(EdgeScrollDirection::Down);
                        self.ui.scroll_system.edge_scroll.area = SelectionArea::Input;
                        self.ui.scroll_system.edge_scroll.last_x = x;
                    } else {
                        self.ui.scroll_system.edge_scroll.direction = None;
                    }

                    if let Some(pos) = self.hit_test_input(x, y) {
                        self.ui.scroll_system.selection.end = Some(pos);
                    }
                }
                true
            }
            SelectionArea::None => false,
        }
    }

    /// Process continuous edge scrolling during selection
    /// Called from main loop to keep scrolling while mouse is held at edge
    pub fn process_edge_scroll(&mut self) {
        let Some(direction) = self.ui.scroll_system.edge_scroll.direction else {
            return;
        };

        if !self.ui.scroll_system.selection.is_selecting {
            self.ui.scroll_system.edge_scroll.direction = None;
            return;
        }

        let x = self.ui.scroll_system.edge_scroll.last_x;

        match self.ui.scroll_system.edge_scroll.area {
            SelectionArea::Messages => {
                if let Some(area) = self.ui.scroll_system.layout.messages_area {
                    match direction {
                        EdgeScrollDirection::Up => {
                            if self.ui.scroll_system.scroll.can_scroll_up() {
                                self.ui.scroll_system.scroll.scroll_up(1);
                            } else {
                                self.ui.scroll_system.edge_scroll.direction = None;
                            }
                        }
                        EdgeScrollDirection::Down => {
                            if self.ui.scroll_system.scroll.can_scroll_down() {
                                self.ui.scroll_system.scroll.scroll_down(1);
                            } else {
                                self.ui.scroll_system.edge_scroll.direction = None;
                            }
                        }
                    }
                    let y = match direction {
                        EdgeScrollDirection::Up => area.y + 1,
                        EdgeScrollDirection::Down => area.y + area.height.saturating_sub(1),
                    };
                    if let Some(pos) = self.hit_test_messages(x, y) {
                        self.ui.scroll_system.selection.end = Some(pos);
                    }
                }
            }
            SelectionArea::Input => {
                if let Some(area) = self.ui.scroll_system.layout.input_area {
                    match direction {
                        EdgeScrollDirection::Up => self.ui.input.scroll_up(),
                        EdgeScrollDirection::Down => self.ui.input.scroll_down(),
                    }
                    let y = match direction {
                        EdgeScrollDirection::Up => area.y + 1,
                        EdgeScrollDirection::Down => area.y + area.height.saturating_sub(1),
                    };
                    if let Some(pos) = self.hit_test_input(x, y) {
                        self.ui.scroll_system.selection.end = Some(pos);
                    }
                }
            }
            SelectionArea::None => {
                self.ui.scroll_system.edge_scroll.direction = None;
            }
        }
    }

    /// Copy selected text to clipboard, returns true on success
    pub fn copy_selection_to_clipboard(&self) -> bool {
        let text = match self.ui.scroll_system.selection.area {
            SelectionArea::Messages => self.get_selected_messages_text(),
            SelectionArea::Input => self.get_selected_input_text(),
            SelectionArea::None => return false,
        };

        if text.is_empty() {
            return false;
        }

        // On Linux, prefer native clipboard tools to avoid arboard's Wayland issues
        // (arboard drops clipboard contents immediately on Wayland)
        #[cfg(target_os = "linux")]
        {
            use std::io::Write;

            // Check if running on Wayland
            let is_wayland = std::env::var("XDG_SESSION_TYPE")
                .map(|s| s == "wayland")
                .unwrap_or(false)
                || std::env::var("WAYLAND_DISPLAY").is_ok();

            if is_wayland {
                // Use wl-copy for Wayland (handles clipboard persistence)
                // Don't wait - just spawn and let it run in background
                if let Ok(mut child) = std::process::Command::new("wl-copy")
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                        // Closing stdin signals EOF to wl-copy
                        drop(stdin);
                        // Spawn a thread to reap the child to avoid zombies
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                        return true;
                    }
                }
            } else {
                // X11 - try xclip first (don't wait)
                if let Ok(mut child) = std::process::Command::new("xclip")
                    .args(["-selection", "clipboard"])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                        drop(stdin);
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                        return true;
                    }
                }

                // Try xsel as fallback (don't wait)
                if let Ok(mut child) = std::process::Command::new("xsel")
                    .args(["--clipboard", "--input"])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                        drop(stdin);
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                        return true;
                    }
                }
            }
        }

        // Fallback to arboard for non-Linux or if native tools fail
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if clipboard.set_text(&text).is_ok() {
                return true;
            }
        }

        false
    }

    /// Extract selected text from messages
    ///
    /// Rebuilds the text content in the same order as render_messages to ensure
    /// accurate line-to-text mapping for selection extraction.
    fn get_selected_messages_text(&self) -> String {
        let Some(((start_line, start_col), (end_line, end_col))) =
            self.ui.scroll_system.selection.normalized()
        else {
            return String::new();
        };

        if self.runtime.chat.messages.is_empty() {
            return String::new();
        }

        // Calculate wrap width (same as render_messages: inner.width - 4 for scrollbar)
        let wrap_width = self
            .ui
            .scroll_system
            .layout
            .messages_area
            .map(|a| a.width.saturating_sub(6) as usize) // border (2) + scrollbar padding (4)
            .unwrap_or(80);
        let content_width = wrap_width as u16;

        // Build flat list of text lines in same order as rendering
        let mut all_lines: Vec<String> = Vec::new();
        let mut thinking_idx = 0;
        let mut bash_idx = 0;
        let mut terminal_idx = 0;
        let mut tool_result_idx = 0;
        let mut read_idx = 0;
        let mut edit_idx = 0;
        let mut write_idx = 0;
        let mut web_search_idx = 0;

        for (role, content) in self.runtime.chat.messages.iter() {
            match role.as_str() {
                "thinking" => {
                    if let Some(block) = self.runtime.blocks.thinking.get(thinking_idx) {
                        let height = block.height(content_width, &self.ui.theme) as usize;
                        // Get text content and pad/truncate to match rendered height
                        let text_lines = block
                            .get_text_content()
                            .map(|t| t.lines().map(|s| s.to_string()).collect::<Vec<_>>())
                            .unwrap_or_default();
                        push_block_lines(&mut all_lines, &text_lines, height);
                    }
                    thinking_idx += 1;
                    all_lines.push(String::new()); // blank after
                }
                "bash" => {
                    if let Some(block) = self.runtime.blocks.bash.get(bash_idx) {
                        let height = block.height(content_width, &self.ui.theme) as usize;
                        let text_lines = block
                            .get_text_content()
                            .map(|t| t.lines().map(|s| s.to_string()).collect::<Vec<_>>())
                            .unwrap_or_default();
                        push_block_lines(&mut all_lines, &text_lines, height);
                    }
                    bash_idx += 1;
                    all_lines.push(String::new());
                }
                "terminal" => {
                    // Skip pinned terminals (same as render)
                    if self.runtime.blocks.pinned_terminal != Some(terminal_idx) {
                        if let Some(block) = self.runtime.blocks.terminal.get(terminal_idx) {
                            let height = block.height(content_width, &self.ui.theme) as usize;
                            let text_lines = block
                                .get_text_content()
                                .map(|t| t.lines().map(|s| s.to_string()).collect::<Vec<_>>())
                                .unwrap_or_default();
                            push_block_lines(&mut all_lines, &text_lines, height);
                            all_lines.push(String::new());
                        }
                    }
                    terminal_idx += 1;
                }
                "tool_result" => {
                    if let Some(block) = self.runtime.blocks.tool_result.get(tool_result_idx) {
                        let height = block.height(content_width, &self.ui.theme) as usize;
                        let text_lines = block
                            .get_text_content()
                            .map(|t| t.lines().map(|s| s.to_string()).collect::<Vec<_>>())
                            .unwrap_or_default();
                        push_block_lines(&mut all_lines, &text_lines, height);
                    }
                    tool_result_idx += 1;
                    all_lines.push(String::new());
                }
                "read" => {
                    if let Some(block) = self.runtime.blocks.read.get(read_idx) {
                        let height = block.height(content_width, &self.ui.theme) as usize;
                        let text_lines = block
                            .get_text_content()
                            .map(|t| t.lines().map(|s| s.to_string()).collect::<Vec<_>>())
                            .unwrap_or_default();
                        push_block_lines(&mut all_lines, &text_lines, height);
                    }
                    read_idx += 1;
                    all_lines.push(String::new());
                }
                "edit" => {
                    if let Some(block) = self.runtime.blocks.edit.get(edit_idx) {
                        let height = block.height(content_width, &self.ui.theme) as usize;
                        let text_lines = block
                            .get_text_content()
                            .map(|t| t.lines().map(|s| s.to_string()).collect::<Vec<_>>())
                            .unwrap_or_default();
                        push_block_lines(&mut all_lines, &text_lines, height);
                    }
                    edit_idx += 1;
                    all_lines.push(String::new());
                }
                "write" => {
                    if let Some(block) = self.runtime.blocks.write.get(write_idx) {
                        let height = block.height(content_width, &self.ui.theme) as usize;
                        let text_lines = block
                            .get_text_content()
                            .map(|t| t.lines().map(|s| s.to_string()).collect::<Vec<_>>())
                            .unwrap_or_default();
                        push_block_lines(&mut all_lines, &text_lines, height);
                    }
                    write_idx += 1;
                    all_lines.push(String::new());
                }
                "web_search" => {
                    if let Some(block) = self.runtime.blocks.web_search.get(web_search_idx) {
                        let height = block.height(content_width, &self.ui.theme) as usize;
                        let text_lines = block
                            .get_text_content()
                            .map(|t| t.lines().map(|s| s.to_string()).collect::<Vec<_>>())
                            .unwrap_or_default();
                        push_block_lines(&mut all_lines, &text_lines, height);
                    }
                    web_search_idx += 1;
                    all_lines.push(String::new());
                }
                "assistant" => {
                    // Use markdown cache to get rendered text (must use get_rendered, not get!)
                    let content_hash = hash_content(content);
                    if let Some(cached) = self
                        .ui
                        .markdown_cache
                        .get_rendered(content_hash, wrap_width)
                    {
                        // Extract text from cached Line spans
                        for line in cached.lines.iter() {
                            let text: String =
                                line.spans.iter().map(|s| s.content.as_ref()).collect();
                            all_lines.push(text);
                        }
                    } else {
                        // Fallback: use raw content with wrapping
                        for line in content.lines() {
                            if line.is_empty() {
                                all_lines.push(String::new());
                            } else {
                                for wrapped in wrap_line(line, wrap_width) {
                                    all_lines.push(wrapped);
                                }
                            }
                        }
                    }
                    all_lines.push(String::new()); // blank after
                }
                "user" | "system" => {
                    // Plain text, wrapped (same as render)
                    for line in content.lines() {
                        if line.is_empty() {
                            all_lines.push(String::new());
                        } else {
                            for wrapped in wrap_line(line, wrap_width) {
                                all_lines.push(wrapped);
                            }
                        }
                    }
                    all_lines.push(String::new()); // blank after
                }
                _ => {
                    // Unknown role - use raw content
                    for line in content.lines() {
                        all_lines.push(line.to_string());
                    }
                    all_lines.push(String::new());
                }
            }
        }

        // Extract selection from built lines
        extract_selection(&all_lines, start_line, start_col, end_line, end_col)
    }

    /// Extract selected text from input
    fn get_selected_input_text(&self) -> String {
        let Some(((start_line, start_col), (end_line, end_col))) =
            self.ui.scroll_system.selection.normalized()
        else {
            return String::new();
        };

        let lines = self.ui.input.get_wrapped_lines();
        extract_selection(&lines, start_line, start_col, end_line, end_col)
    }
}

/// Simple content hash for cache lookup
fn hash_content(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Push block text lines, padding or truncating to match rendered height
fn push_block_lines(all_lines: &mut Vec<String>, text_lines: &[String], height: usize) {
    // Push actual text lines (up to height)
    for line in text_lines.iter().take(height) {
        all_lines.push(line.clone());
    }
    // Pad with empty lines if text is shorter than rendered height
    for _ in text_lines.len()..height {
        all_lines.push(String::new());
    }
}

/// Extract text from lines given selection coordinates (CHARACTER indices, not bytes)
fn extract_selection(
    lines: &[String],
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
) -> String {
    if start_line >= lines.len() {
        return String::new();
    }

    let end_line = end_line.min(lines.len().saturating_sub(1));

    if start_line == end_line {
        let line = &lines[start_line];
        let char_count = line.chars().count(); // Use CHARACTER count, not byte len
        let start = start_col.min(char_count);
        let end = end_col.min(char_count);
        if start < end {
            line.chars().skip(start).take(end - start).collect()
        } else {
            String::new()
        }
    } else {
        let mut result = String::new();

        let first_line = &lines[start_line];
        let first_char_count = first_line.chars().count();
        let start = start_col.min(first_char_count);
        result.push_str(&first_line.chars().skip(start).collect::<String>());
        result.push('\n');

        for line in lines.iter().take(end_line).skip(start_line + 1) {
            result.push_str(line);
            result.push('\n');
        }

        let last_line = &lines[end_line];
        let last_char_count = last_line.chars().count();
        let end = end_col.min(last_char_count);
        result.push_str(&last_line.chars().take(end).collect::<String>());

        result
    }
}
