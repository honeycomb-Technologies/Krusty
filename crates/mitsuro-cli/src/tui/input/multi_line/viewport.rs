//! Viewport and cursor position management

use super::MultiLineInput;

impl MultiLineInput {
    pub(super) fn update_visual_cursor(&mut self) {
        let lines = self.get_wrapped_lines();
        let mut byte_pos = 0;
        let mut found = false;

        for (line_idx, line) in lines.iter().enumerate() {
            let mut line_byte_pos = 0;
            for ch in line.chars() {
                if byte_pos == self.cursor_position {
                    self.cursor_visual = (line_idx, line_byte_pos);
                    found = true;
                    break;
                }
                byte_pos += ch.len_utf8();
                line_byte_pos += ch.len_utf8();
            }

            if found {
                break;
            }

            // Account for newline - only if there's actually a newline character (not soft wrap)
            if line_idx < lines.len() - 1
                && byte_pos < self.content.len()
                && self.content.as_bytes().get(byte_pos) == Some(&b'\n')
            {
                if byte_pos == self.cursor_position {
                    self.cursor_visual = (line_idx, line.len());
                    found = true;
                    break;
                }
                byte_pos += 1;
            }
        }

        if !found {
            // Cursor at end
            if let Some(last_line) = lines.last() {
                self.cursor_visual = (lines.len() - 1, last_line.len());
            } else {
                self.cursor_visual = (0, 0);
            }
        }
    }

    pub fn set_cursor_to_visual_position(&mut self, line: usize, col: usize) {
        self.cursor_position = self.get_byte_position_from_visual(line, col);
        self.cursor_visual = (line, col);
        self.ensure_cursor_visible();
    }

    /// Set cursor to a specific byte position within a wrapped line.
    /// This is used by click handling where we calculate byte offset from visual position.
    fn set_cursor_to_byte_position_in_line(&mut self, line: usize, byte_in_line: usize) {
        self.cursor_position = self.get_byte_position_for_line_offset(line, byte_in_line);
        self.cursor_visual = (line, byte_in_line);
        self.ensure_cursor_visible();
    }

    pub(super) fn get_byte_position_from_visual(&self, line: usize, col: usize) -> usize {
        self.get_byte_position_for_line_offset(line, col)
    }

    /// Convert a line index and byte offset within that line to an absolute byte position.
    /// Uses O(1) byte access instead of O(n) chars().nth() for newline detection.
    pub(super) fn get_byte_position_for_line_offset(
        &self,
        line: usize,
        byte_in_line: usize,
    ) -> usize {
        let lines = self.get_wrapped_lines();
        let mut byte_pos = 0;

        for (idx, line_content) in lines.iter().enumerate() {
            if idx < line {
                byte_pos += line_content.len();
                // Check for hard newline (not soft wrap) using O(1) byte access
                if byte_pos < self.content.len()
                    && self.content.as_bytes().get(byte_pos) == Some(&b'\n')
                {
                    byte_pos += 1;
                }
            } else if idx == line {
                byte_pos += byte_in_line.min(line_content.len());
                break;
            }
        }

        byte_pos.min(self.content.len())
    }

    pub(super) fn ensure_cursor_visible(&mut self) {
        let (line, _) = self.cursor_visual;
        let visible = self.max_visible_lines as usize;

        // Scroll up if cursor above viewport
        if line < self.viewport_offset {
            self.viewport_offset = line;
        }
        // Scroll down if cursor below viewport
        else if line >= self.viewport_offset + visible {
            self.viewport_offset = line - visible + 1;
        }

        // Clamp viewport to valid range
        let total_lines = self.get_wrapped_lines().len();
        let max_offset = total_lines.saturating_sub(visible);
        self.viewport_offset = self.viewport_offset.min(max_offset);
    }

    pub(super) fn handle_click_impl(&mut self, x: u16, y: u16) {
        // Subtract 2 for: 1 (border) + 1 (left padding added in renderer)
        let content_x = x.saturating_sub(2);
        let content_y = y.saturating_sub(1);
        let clicked_line = self.viewport_offset + content_y as usize;

        let lines = self.get_wrapped_lines();
        if clicked_line < lines.len() {
            let line_content = &lines[clicked_line];
            let mut byte_offset = 0;
            let mut visual_width = 0;
            let target_visual_col = content_x as usize;

            for ch in line_content.chars() {
                let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                if visual_width + ch_width > target_visual_col {
                    // Snap to closer character boundary (left or right edge)
                    if target_visual_col > visual_width + ch_width / 2 {
                        byte_offset += ch.len_utf8();
                    }
                    break;
                }
                byte_offset += ch.len_utf8();
                visual_width += ch_width;
            }

            self.set_cursor_to_byte_position_in_line(clicked_line, byte_offset);
        }
    }

    pub(super) fn scroll_up_impl(&mut self) {
        if self.viewport_offset > 0 {
            self.viewport_offset -= 1;
        }
    }

    pub(super) fn scroll_down_impl(&mut self) {
        let total_lines = self.get_wrapped_lines().len();
        let max_offset = total_lines.saturating_sub(self.max_visible_lines as usize);
        if self.viewport_offset < max_offset {
            self.viewport_offset += 1;
        }
    }
}
