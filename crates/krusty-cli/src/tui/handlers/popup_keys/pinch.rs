//! Pinch (context compression) popup keyboard handler

use crossterm::event::{KeyCode, KeyModifiers};

use crate::tui::app::{App, Popup};
use crate::tui::popups::pinch::PinchStage;

impl App {
    /// Handle pinch popup keyboard events
    pub fn handle_pinch_popup_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match &self.ui.popups.pinch.stage {
            PinchStage::PreservationInput { .. } => match code {
                KeyCode::Esc => {
                    self.ui.popups.pinch.reset();
                    self.ui.popup = Popup::None;
                }
                KeyCode::Enter => {
                    self.start_pinch_summarization();
                }
                KeyCode::Backspace => self.ui.popups.pinch.backspace(),
                KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                    self.ui.popups.pinch.add_char(c);
                }
                _ => {}
            },
            PinchStage::Summarizing { .. } => {
                // Allow cancel during summarization
                if code == KeyCode::Esc {
                    self.runtime.cancellation.cancel();
                    self.ui.popups.pinch.reset();
                    self.ui.popup = Popup::None;
                }
            }
            PinchStage::DirectionInput { .. } => match code {
                KeyCode::Esc => {
                    self.ui.popups.pinch.reset();
                    self.ui.popup = Popup::None;
                }
                KeyCode::Up => self.ui.popups.pinch.scroll_up(),
                KeyCode::Down => self.ui.popups.pinch.scroll_down(),
                KeyCode::Enter => {
                    self.complete_pinch();
                }
                KeyCode::Backspace => self.ui.popups.pinch.backspace(),
                KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                    self.ui.popups.pinch.add_char(c);
                }
                _ => {}
            },
            PinchStage::Creating => {
                // No input during creation
            }
            PinchStage::Complete {
                new_session_id,
                auto_continue,
                ..
            } => {
                match code {
                    KeyCode::Enter => {
                        // Switch to new session
                        let id = new_session_id.clone();
                        let should_continue = *auto_continue;
                        self.save_block_ui_states();
                        if let Err(e) = self.load_session(&id) {
                            self.runtime.chat.messages.push((
                                "system".to_string(),
                                format!("Failed to load session: {}", e),
                            ));
                        } else if should_continue {
                            // Direction was provided - auto-start AI response
                            self.send_to_ai();
                        }
                        self.ui.popups.pinch.reset();
                        self.ui.popup = Popup::None;
                    }
                    KeyCode::Esc => {
                        self.ui.popups.pinch.reset();
                        self.ui.popup = Popup::None;
                    }
                    _ => {}
                }
            }
            PinchStage::Error { .. } => {
                if code == KeyCode::Esc {
                    self.ui.popups.pinch.reset();
                    self.ui.popup = Popup::None;
                }
            }
        }
    }
}
