//! Skills browser popup keyboard handler

use crossterm::event::KeyCode;
use mitsuro_core::skills::SkillPermission;

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
            KeyCode::Enter if !self.ui.popups.skills.search_active => {
                self.prepare_selected_skill_invocation();
            }
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
            KeyCode::Char('e') if !self.ui.popups.skills.search_active => {
                self.toggle_selected_skill();
            }
            KeyCode::Char('p') if !self.ui.popups.skills.search_active => {
                self.cycle_selected_skill_policy();
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

    fn prepare_selected_skill_invocation(&mut self) {
        let Some(skill) = self.ui.popups.skills.selected_skill().cloned() else {
            return;
        };
        if !skill.enabled {
            self.skill_browser_message(format!(
                "Skill '{}' is disabled. Press e to enable it.",
                skill.name
            ));
            return;
        }
        if skill.permission == SkillPermission::Deny {
            self.skill_browser_message(format!(
                "Skill '{}' is denied. Press p to change its policy.",
                skill.name
            ));
            return;
        }

        self.ui.popup = Popup::None;
        self.ui
            .input
            .insert_text(&format!("/skill:{} ", skill.name));
    }

    fn toggle_selected_skill(&mut self) {
        let Some(skill) = self.ui.popups.skills.selected_skill().cloned() else {
            return;
        };
        let result = self
            .services
            .skills_manager
            .try_write()
            .map_err(|_| "Skills manager is busy".to_string())
            .and_then(|mut manager| {
                manager
                    .set_skill_enabled(&skill.name, !skill.enabled)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => self.refresh_skills_browser_preserving(Some(&skill.name)),
            Err(error) => self.skill_browser_message(error),
        }
    }

    fn cycle_selected_skill_policy(&mut self) {
        let Some(skill) = self.ui.popups.skills.selected_skill().cloned() else {
            return;
        };
        let next = skill.permission.cycle();
        let result = self
            .services
            .skills_manager
            .try_write()
            .map_err(|_| "Skills manager is busy".to_string())
            .and_then(|mut manager| {
                manager
                    .set_skill_permission(&skill.name, next)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => self.refresh_skills_browser_preserving(Some(&skill.name)),
            Err(error) => self.skill_browser_message(error),
        }
    }

    fn refresh_skills_browser_preserving(&mut self, selected: Option<&str>) {
        let skills = match self.services.skills_manager.try_write() {
            Ok(mut manager) => manager.list_skills(),
            Err(_) => return,
        };
        self.ui
            .popups
            .skills
            .set_skills_preserving(skills, selected);
    }

    fn skill_browser_message(&mut self, message: impl Into<String>) {
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), message.into()));
    }
}
