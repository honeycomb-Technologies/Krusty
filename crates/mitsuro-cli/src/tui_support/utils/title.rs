//! Title Editor
//!
//! Handles session title editing state and input.

use crossterm::event::{KeyCode, KeyModifiers};

/// Result of handling a key in title edit mode
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleAction {
    /// Key was consumed, continue editing
    Continue,
    /// User confirmed the title (Enter pressed)
    Save,
    /// User cancelled editing (Esc pressed)
    Cancel,
}

/// Title editing state
#[derive(Debug, Default)]
pub struct TitleEditor {
    /// Whether currently editing
    pub is_editing: bool,
    /// Current edit buffer
    pub buffer: String,
}

impl TitleEditor {
    /// Create a new title editor
    pub fn new() -> Self {
        Self::default()
    }

    /// Start editing with the given initial value
    pub fn start(&mut self, current_title: Option<&str>) {
        self.is_editing = true;
        self.buffer = current_title
            .map(|s| s.to_string())
            .unwrap_or_else(|| "New Chat".to_string());
    }

    /// Cancel editing and clear buffer
    pub fn cancel(&mut self) {
        self.is_editing = false;
        self.buffer.clear();
    }

    /// Finish editing and return the new title if valid
    /// Returns Some(title) if the buffer is non-empty, None otherwise
    pub fn finish(&mut self) -> Option<String> {
        self.is_editing = false;
        let trimmed = self.buffer.trim();
        let result = if !trimmed.is_empty() {
            Some(trimmed.to_string())
        } else {
            None
        };
        self.buffer.clear();
        result
    }

    /// Handle a key event while editing
    /// Returns the action to take
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> TitleAction {
        match code {
            KeyCode::Enter => TitleAction::Save,
            KeyCode::Esc => TitleAction::Cancel,
            KeyCode::Backspace => {
                self.buffer.pop();
                TitleAction::Continue
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                if self.buffer.len() < 50 {
                    self.buffer.push(c);
                }
                TitleAction::Continue
            }
            _ => TitleAction::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_with_title() {
        let mut editor = TitleEditor::new();
        editor.start(Some("Test Title"));
        assert!(editor.is_editing);
        assert_eq!(editor.buffer, "Test Title");
    }

    #[test]
    fn test_start_without_title() {
        let mut editor = TitleEditor::new();
        editor.start(None);
        assert!(editor.is_editing);
        assert_eq!(editor.buffer, "New Chat");
    }

    #[test]
    fn test_cancel() {
        let mut editor = TitleEditor::new();
        editor.start(Some("Test"));
        editor.cancel();
        assert!(!editor.is_editing);
        assert!(editor.buffer.is_empty());
    }

    #[test]
    fn test_finish_with_content() {
        let mut editor = TitleEditor::new();
        editor.start(Some("Test"));
        let result = editor.finish();
        assert_eq!(result, Some("Test".to_string()));
        assert!(!editor.is_editing);
    }

    #[test]
    fn test_finish_empty() {
        let mut editor = TitleEditor::new();
        editor.start(Some(""));
        let result = editor.finish();
        assert_eq!(result, None);
    }

    #[test]
    fn test_handle_key_char() {
        let mut editor = TitleEditor::new();
        editor.start(Some(""));
        editor.handle_key(KeyCode::Char('H'), KeyModifiers::empty());
        editor.handle_key(KeyCode::Char('i'), KeyModifiers::empty());
        assert_eq!(editor.buffer, "Hi");
    }

    #[test]
    fn test_handle_key_backspace() {
        let mut editor = TitleEditor::new();
        editor.start(Some("Test"));
        editor.handle_key(KeyCode::Backspace, KeyModifiers::empty());
        assert_eq!(editor.buffer, "Tes");
    }
}
