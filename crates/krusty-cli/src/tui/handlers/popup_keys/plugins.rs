//! Installable plugins popup keyboard handler

use crossterm::event::KeyCode;

use crate::tui::app::{App, Popup};

impl App {
    pub fn handle_plugins_popup_key(&mut self, code: KeyCode) {
        if self.ui.popups.plugins.search_active {
            match code {
                KeyCode::Esc => self.ui.popups.plugins.toggle_search(),
                KeyCode::Backspace => self.ui.popups.plugins.backspace_search(),
                KeyCode::Enter => self.toggle_selected_plugin_from_popup(),
                KeyCode::Char(c) => self.ui.popups.plugins.add_search_char(c),
                KeyCode::Up => self.ui.popups.plugins.prev(),
                KeyCode::Down => self.ui.popups.plugins.next(),
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Esc => {
                self.ui.popup = Popup::None;
            }
            KeyCode::Char('/') => self.ui.popups.plugins.toggle_search(),
            KeyCode::Up | KeyCode::Char('k') => self.ui.popups.plugins.prev(),
            KeyCode::Down | KeyCode::Char('j') => self.ui.popups.plugins.next(),
            KeyCode::Enter | KeyCode::Char('e') => self.toggle_selected_plugin_from_popup(),
            KeyCode::Char('r') => {
                self.refresh_plugins_browser();
            }
            _ => {}
        }
    }
}
