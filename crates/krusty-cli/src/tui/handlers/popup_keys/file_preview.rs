//! File preview popup keyboard handler

use crossterm::event::KeyCode;

use crate::tui::app::{App, Popup};

impl App {
    pub fn handle_file_preview_popup_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.popups.file_preview.reset();
                self.ui.popup = Popup::None;
            }
            KeyCode::Left | KeyCode::Char('h') => self.popups.file_preview.prev_page(),
            KeyCode::Right | KeyCode::Char('l') => self.popups.file_preview.next_page(),
            KeyCode::Char('o') | KeyCode::Char('O') => self.popups.file_preview.open_external(),
            _ => {}
        }
    }
}
