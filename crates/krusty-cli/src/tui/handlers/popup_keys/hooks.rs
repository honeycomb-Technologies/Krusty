//! User hooks popup keyboard handler

use crossterm::event::KeyCode;

use crate::paths;
use crate::storage::Database;
use crate::tui::app::{App, Popup};
use crate::tui::popups::hooks::HooksStage;

impl App {
    /// Handle hooks popup keyboard events
    pub fn handle_hooks_popup_key(&mut self, code: KeyCode) {
        match &self.ui.popups.hooks.stage {
            HooksStage::List => match code {
                KeyCode::Esc => self.ui.popup = Popup::None,
                KeyCode::Up | KeyCode::Char('k') => self.ui.popups.hooks.prev(),
                KeyCode::Down | KeyCode::Char('j') => self.ui.popups.hooks.next(),
                KeyCode::Enter => {
                    if self.ui.popups.hooks.is_add_new_selected() {
                        self.ui.popups.hooks.start_add();
                    }
                }
                KeyCode::Char(' ') => {
                    self.toggle_selected_hook();
                }
                KeyCode::Char('d') => {
                    self.delete_selected_hook();
                }
                _ => {}
            },
            HooksStage::SelectType { .. } => match code {
                KeyCode::Esc => self.ui.popups.hooks.go_back(),
                KeyCode::Up | KeyCode::Char('k') => self.ui.popups.hooks.prev(),
                KeyCode::Down | KeyCode::Char('j') => self.ui.popups.hooks.next(),
                KeyCode::Enter => self.ui.popups.hooks.confirm_type(),
                _ => {}
            },
            HooksStage::EnterMatcher { .. } => match code {
                KeyCode::Esc => self.ui.popups.hooks.go_back(),
                KeyCode::Enter => self.ui.popups.hooks.confirm_matcher(),
                KeyCode::Backspace => self.ui.popups.hooks.backspace(),
                KeyCode::Char(c) => self.ui.popups.hooks.add_char(c),
                _ => {}
            },
            HooksStage::EnterCommand { .. } => match code {
                KeyCode::Esc => self.ui.popups.hooks.go_back(),
                KeyCode::Enter => self.ui.popups.hooks.confirm_command(),
                KeyCode::Backspace => self.ui.popups.hooks.backspace(),
                KeyCode::Char(c) => self.ui.popups.hooks.add_char(c),
                _ => {}
            },
            HooksStage::Confirm { .. } => match code {
                KeyCode::Esc => self.ui.popups.hooks.go_back(),
                KeyCode::Enter => {
                    self.save_pending_hook();
                }
                _ => {}
            },
        }
    }

    /// Toggle enabled state of selected hook
    fn toggle_selected_hook(&mut self) {
        if let Some(id) = self.ui.popups.hooks.get_selected_hook_id() {
            if let Ok(db) = Database::new(&paths::config_dir().join("krusty.db")) {
                let id = id.to_string();
                futures::executor::block_on(async {
                    let _ = self
                        .services
                        .user_hook_manager
                        .write()
                        .await
                        .toggle(&db, &id);
                });
                self.refresh_hooks_popup();
            }
        }
    }

    /// Delete selected hook
    fn delete_selected_hook(&mut self) {
        if let Some(id) = self.ui.popups.hooks.get_selected_hook_id() {
            if let Ok(db) = Database::new(&paths::config_dir().join("krusty.db")) {
                let id = id.to_string();
                futures::executor::block_on(async {
                    let _ = self
                        .services
                        .user_hook_manager
                        .write()
                        .await
                        .delete(&db, &id);
                });
                self.refresh_hooks_popup();
            }
        }
    }

    /// Save pending hook from wizard
    fn save_pending_hook(&mut self) {
        if let Some(hook) = self.ui.popups.hooks.get_pending_hook() {
            if let Ok(db) = Database::new(&paths::config_dir().join("krusty.db")) {
                futures::executor::block_on(async {
                    let _ = self
                        .services
                        .user_hook_manager
                        .write()
                        .await
                        .save(&db, hook);
                });
                self.refresh_hooks_popup();
                self.ui.popups.hooks.reset();
            }
        }
    }

    /// Refresh hooks popup with current hooks from database
    pub fn refresh_hooks_popup(&mut self) {
        let hooks = futures::executor::block_on(async {
            self.services
                .user_hook_manager
                .read()
                .await
                .hooks()
                .to_vec()
        });
        self.ui.popups.hooks.set_hooks(hooks);
    }
}
