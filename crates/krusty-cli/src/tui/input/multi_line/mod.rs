//! Multi-line input handler with proper text wrapping and cursor management

use std::cell::RefCell;

use crossterm::event::{KeyCode, KeyModifiers};

mod editor;
mod patterns;
mod renderer;
mod viewport;
mod wrapper;

pub use editor::InputAction;
pub(crate) use patterns::FILE_REF_PATTERN;

/// Multi-line input handler with proper text wrapping and cursor management
pub struct MultiLineInput {
    /// The actual text content
    pub(crate) content: String,
    /// Current cursor position in the content (byte offset)
    pub(crate) cursor_position: usize,
    /// Visual cursor position (line, column in bytes)
    pub(crate) cursor_visual: (usize, usize),
    /// Width of the input area for wrapping
    pub(crate) width: u16,
    /// Viewport offset for scrolling
    pub(crate) viewport_offset: usize,
    /// Maximum visible lines
    pub(crate) max_visible_lines: u16,
    /// Cached wrapped lines (invalidated on content/width change)
    wrapped_lines_cache: RefCell<Option<Vec<String>>>,
}

impl MultiLineInput {
    pub fn new(max_visible_lines: u16) -> Self {
        Self {
            content: String::new(),
            cursor_position: 0,
            cursor_visual: (0, 0),
            width: 80,
            viewport_offset: 0,
            max_visible_lines,
            wrapped_lines_cache: RefCell::new(None),
        }
    }

    /// Invalidate the wrapped lines cache (call when content or width changes)
    pub(crate) fn invalidate_cache(&self) {
        *self.wrapped_lines_cache.borrow_mut() = None;
    }

    pub fn set_width(&mut self, width: u16) {
        // Account for borders + padding + scrollbar
        let new_width = width.saturating_sub(4).max(10);
        if self.width != new_width {
            self.width = new_width;
            self.invalidate_cache();
            // Recalculate cursor position after width change affects wrapping
            self.update_visual_cursor();
            self.ensure_cursor_visible();
        }
    }

    pub fn clear(&mut self) {
        self.content.clear();
        self.cursor_position = 0;
        self.cursor_visual = (0, 0);
        self.viewport_offset = 0;
        self.invalidate_cache();
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn set_max_visible_lines(&mut self, lines: u16) {
        if self.max_visible_lines != lines {
            self.max_visible_lines = lines;
            self.ensure_cursor_visible();
        }
    }

    // Editor methods
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> InputAction {
        self.handle_key_impl(code, modifiers)
    }

    pub fn insert_char(&mut self, ch: char) {
        self.insert_char_impl(ch)
    }

    pub fn insert_text(&mut self, text: &str) {
        self.insert_text_impl(text)
    }

    // Wrapper methods
    pub fn get_wrapped_lines(&self) -> Vec<String> {
        self.get_wrapped_lines_impl()
    }

    pub fn get_wrapped_lines_count(&self) -> usize {
        self.get_wrapped_lines().len()
    }

    // Viewport methods
    pub fn handle_click(&mut self, x: u16, y: u16) {
        self.handle_click_impl(x, y)
    }

    pub fn scroll_up(&mut self) {
        self.scroll_up_impl()
    }

    pub fn scroll_down(&mut self) {
        self.scroll_down_impl()
    }

    pub fn get_max_visible_lines(&self) -> u16 {
        self.max_visible_lines
    }

    pub fn get_viewport_offset(&self) -> usize {
        self.viewport_offset
    }

    pub fn set_viewport_offset(&mut self, offset: usize) {
        let total_lines = self.get_wrapped_lines().len();
        let max_offset = total_lines.saturating_sub(self.max_visible_lines as usize);
        self.viewport_offset = offset.min(max_offset);
    }

    /// Get file reference at click position (relative to input area)
    /// Returns (byte_start, byte_end, path) if click is on a file reference
    pub fn get_file_ref_at_click(
        &self,
        x: u16,
        y: u16,
    ) -> Option<(usize, usize, std::path::PathBuf)> {
        // Convert click to content coordinates (subtract border/padding)
        let content_x = x.saturating_sub(2) as usize;
        let content_y = y.saturating_sub(1) as usize;
        let clicked_line = self.viewport_offset + content_y;

        let lines = self.get_wrapped_lines();
        if clicked_line >= lines.len() {
            return None;
        }

        // Calculate byte offset within the clicked line using visual width
        let line_content = &lines[clicked_line];
        let mut byte_in_line = 0;
        let mut visual_width = 0;

        for ch in line_content.chars() {
            let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
            if visual_width >= content_x {
                break;
            }
            byte_in_line += ch.len_utf8();
            visual_width += ch_width;
        }

        // Convert line + byte_in_line to absolute byte position using the same
        // logic as handle_click_impl (accounts for soft wraps AND hard newlines)
        let mut absolute_byte_pos = 0;
        for (idx, line) in lines.iter().enumerate() {
            if idx < clicked_line {
                absolute_byte_pos += line.len();
                // Check for hard newline after this line (not soft wrap)
                if absolute_byte_pos < self.content.len()
                    && self.content.as_bytes().get(absolute_byte_pos) == Some(&b'\n')
                {
                    absolute_byte_pos += 1;
                }
            } else {
                // On the clicked line, add the byte offset within the line
                absolute_byte_pos += byte_in_line;
                break;
            }
        }

        // Find bracketed file refs in content using shared pattern
        for caps in FILE_REF_PATTERN.captures_iter(&self.content) {
            let m = caps.get(0)?;
            let path_str = caps.get(1)?.as_str();

            let start = m.start();
            let end = m.end();

            // Check if the clicked byte position falls within this file reference
            if absolute_byte_pos >= start && absolute_byte_pos < end {
                let path = std::path::PathBuf::from(path_str);
                if path.exists() {
                    return Some((start, end, path));
                }
            }
        }

        None
    }

    /// Get all file reference ranges in content for styling
    /// Returns vec of (byte_start, byte_end) for each file reference
    pub fn get_file_ref_ranges(&self) -> Vec<(usize, usize)> {
        FILE_REF_PATTERN
            .find_iter(&self.content)
            .map(|m| (m.start(), m.end()))
            .collect()
    }
}
