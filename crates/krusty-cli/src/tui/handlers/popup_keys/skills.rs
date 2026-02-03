//! Skills browser popup keyboard handler

use crossterm::event::KeyCode;

use crate::tui::app::{App, Popup};

impl App {
    /// Handle skills browser popup keyboard events
    pub fn handle_skills_popup_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                if self.ui.popups.skills.search_active {
                    self.ui.popups.skills.toggle_search();
                } else {
                    self.ui.popup = Popup::None;
                }
            }
            KeyCode::Up => self.ui.popups.skills.prev(),
            KeyCode::Down => self.ui.popups.skills.next(),
            KeyCode::Char('k') if !self.ui.popups.skills.search_active => {
                self.ui.popups.skills.prev();
            }
            KeyCode::Char('j') if !self.ui.popups.skills.search_active => {
                self.ui.popups.skills.next();
            }
            KeyCode::Char('/') if !self.ui.popups.skills.search_active => {
                self.ui.popups.skills.toggle_search();
            }
            KeyCode::Char('r') if !self.ui.popups.skills.search_active => {
                self.refresh_skills_browser();
            }
            KeyCode::Backspace if self.ui.popups.skills.search_active => {
                self.ui.popups.skills.backspace_search();
            }
            KeyCode::Char(c) if self.ui.popups.skills.search_active => {
                self.ui.popups.skills.add_search_char(c);
            }
            _ => {}
        }
    }
}
