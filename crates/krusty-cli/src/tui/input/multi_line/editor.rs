//! Keyboard handling for multi-line input

use arboard::Clipboard;
use crossterm::event::{KeyCode, KeyModifiers};

use super::MultiLineInput;

#[derive(Debug)]
pub enum InputAction {
    Continue,
    Submit(String),
    ContentChanged,
    /// Clipboard image pasted: (width, height, rgba_bytes, placeholder_id)
    ImagePasted {
        width: usize,
        height: usize,
        rgba_bytes: Vec<u8>,
        placeholder_id: String,
    },
}

impl MultiLineInput {
    pub(super) fn handle_key_impl(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> InputAction {
        match code {
            // Shift+Enter or Alt+Enter inserts newline
            KeyCode::Enter
                if modifiers.contains(KeyModifiers::SHIFT)
                    || modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_char('\n');
                InputAction::ContentChanged
            }
            // Ctrl+J also inserts newline (some terminals send this for Shift+Enter)
            KeyCode::Char('j') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert_char('\n');
                InputAction::ContentChanged
            }
            // Some terminals send raw newline/linefeed for Shift+Enter
            KeyCode::Char('\n') | KeyCode::Char('\r') => {
                self.insert_char('\n');
                InputAction::ContentChanged
            }
            // Plain Enter submits (if not empty/whitespace-only)
            // Don't clear here - let caller clear after validation succeeds
            KeyCode::Enter => {
                let content = self.content().to_string();
                if content.trim().is_empty() {
                    return InputAction::Continue;
                }
                InputAction::Submit(content)
            }
            // Ctrl+W - delete word backwards
            KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_word_backwards();
                InputAction::ContentChanged
            }
            // Ctrl+U - clear line
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.clear();
                InputAction::ContentChanged
            }
            // Ctrl+A - start of line
            KeyCode::Char('a') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_cursor_line_start();
                InputAction::Continue
            }
            // Ctrl+E - end of line
            KeyCode::Char('e') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_cursor_line_end();
                InputAction::Continue
            }
            // Ctrl+V - paste from clipboard (image or text)
            KeyCode::Char('v') if modifiers.contains(KeyModifiers::CONTROL) => {
                // Try image paste via arboard first (native tools can't do images)
                if let Ok(mut clipboard) = Clipboard::new() {
                    if let Ok(image_data) = clipboard.get_image() {
                        let placeholder_id = uuid::Uuid::new_v4().to_string();
                        let placeholder = format!("[clipboard:{}]", placeholder_id);
                        self.insert_text(&placeholder);
                        return InputAction::ImagePasted {
                            width: image_data.width,
                            height: image_data.height,
                            rgba_bytes: image_data.bytes.into_owned(),
                            placeholder_id,
                        };
                    }
                }
                // Text paste: use platform-aware helper (wl-paste/xclip on Linux)
                if let Some(text) = crate::tui::utils::clipboard::read_clipboard_text() {
                    self.insert_text(&text);
                    return InputAction::ContentChanged;
                }
                InputAction::Continue
            }
            // Ctrl+C - clear input
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.clear();
                InputAction::ContentChanged
            }
            // Ctrl+K - delete to end of line
            KeyCode::Char('k') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_to_end_of_line();
                InputAction::ContentChanged
            }
            // Regular character
            KeyCode::Char(ch) => {
                self.insert_char(ch);
                InputAction::ContentChanged
            }
            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    self.invalidate_cache();
                    let mut char_start = self.cursor_position - 1;
                    while char_start > 0 && !self.content.is_char_boundary(char_start) {
                        char_start -= 1;
                    }
                    self.content.drain(char_start..self.cursor_position);
                    self.cursor_position = char_start;
                    self.update_visual_cursor();
                    self.ensure_cursor_visible();
                    InputAction::ContentChanged
                } else {
                    InputAction::Continue
                }
            }
            KeyCode::Delete => {
                if self.cursor_position < self.content.len() {
                    self.invalidate_cache();
                    let mut char_end = self.cursor_position + 1;
                    while char_end < self.content.len() && !self.content.is_char_boundary(char_end)
                    {
                        char_end += 1;
                    }
                    self.content.drain(self.cursor_position..char_end);
                    self.update_visual_cursor();
                    InputAction::ContentChanged
                } else {
                    InputAction::Continue
                }
            }
            KeyCode::Left => {
                self.move_cursor_left();
                InputAction::Continue
            }
            KeyCode::Right => {
                self.move_cursor_right();
                InputAction::Continue
            }
            KeyCode::Up => {
                self.move_cursor_up();
                InputAction::Continue
            }
            KeyCode::Down => {
                self.move_cursor_down();
                InputAction::Continue
            }
            KeyCode::Home => {
                self.move_cursor_line_start();
                InputAction::Continue
            }
            KeyCode::End => {
                self.move_cursor_line_end();
                InputAction::Continue
            }
            _ => InputAction::Continue,
        }
    }

    pub(super) fn insert_char_impl(&mut self, ch: char) {
        self.invalidate_cache();
        self.content.insert(self.cursor_position, ch);
        self.cursor_position += ch.len_utf8();
        self.update_visual_cursor();
        self.ensure_cursor_visible();
    }

    pub(super) fn insert_text_impl(&mut self, text: &str) {
        self.invalidate_cache();
        self.content.insert_str(self.cursor_position, text);
        self.cursor_position += text.len();
        self.update_visual_cursor();
        self.ensure_cursor_visible();
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            let mut new_pos = self.cursor_position - 1;
            while new_pos > 0 && !self.content.is_char_boundary(new_pos) {
                new_pos -= 1;
            }
            self.cursor_position = new_pos;
            self.update_visual_cursor();
            self.ensure_cursor_visible();
        }
    }

    fn move_cursor_right(&mut self) {
        if self.cursor_position < self.content.len() {
            let mut new_pos = self.cursor_position + 1;
            while new_pos < self.content.len() && !self.content.is_char_boundary(new_pos) {
                new_pos += 1;
            }
            self.cursor_position = new_pos;
            self.update_visual_cursor();
            self.ensure_cursor_visible();
        }
    }

    fn move_cursor_up(&mut self) {
        let (line, col) = self.cursor_visual;
        if line > 0 {
            self.set_cursor_to_visual_position(line - 1, col);
        }
    }

    fn move_cursor_down(&mut self) {
        let lines = self.get_wrapped_lines();
        let (line, col) = self.cursor_visual;
        if line < lines.len() - 1 {
            self.set_cursor_to_visual_position(line + 1, col);
        }
    }

    fn move_cursor_line_start(&mut self) {
        let (line, _) = self.cursor_visual;
        self.set_cursor_to_visual_position(line, 0);
    }

    fn move_cursor_line_end(&mut self) {
        let lines = self.get_wrapped_lines();
        let (line, _) = self.cursor_visual;
        if let Some(line_content) = lines.get(line) {
            self.set_cursor_to_visual_position(line, line_content.len());
        }
    }

    fn delete_to_end_of_line(&mut self) {
        let lines = self.get_wrapped_lines();
        let (line_idx, _) = self.cursor_visual;

        if let Some(current_line) = lines.get(line_idx) {
            let line_end_pos = self.get_byte_position_from_visual(line_idx, current_line.len());
            if line_end_pos > self.cursor_position {
                self.invalidate_cache();
                self.content.drain(self.cursor_position..line_end_pos);
                self.update_visual_cursor();
            }
        }
    }

    fn delete_word_backwards(&mut self) {
        if self.cursor_position == 0 {
            return;
        }

        let mut new_pos = self.cursor_position;

        // Skip trailing whitespace
        while new_pos > 0 {
            if let Some(c) = self.content[..new_pos].chars().last() {
                if !c.is_whitespace() {
                    break;
                }
                new_pos -= c.len_utf8();
            } else {
                break;
            }
        }

        // Delete the word
        while new_pos > 0 {
            if let Some(c) = self.content[..new_pos].chars().last() {
                if c.is_whitespace() {
                    break;
                }
                new_pos -= c.len_utf8();
            } else {
                break;
            }
        }

        if new_pos < self.cursor_position {
            self.invalidate_cache();
            self.content.drain(new_pos..self.cursor_position);
            self.cursor_position = new_pos;
            self.update_visual_cursor();
            self.ensure_cursor_visible();
        }
    }
}
