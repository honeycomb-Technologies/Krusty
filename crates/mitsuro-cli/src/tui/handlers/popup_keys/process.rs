//! Process list popup keyboard handler

use crossterm::event::KeyCode;

use crate::tui::app::{App, Popup};

impl App {
    /// Handle process list popup keyboard events
    pub fn handle_process_popup_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.ui.popup = Popup::None,
            KeyCode::Up | KeyCode::Char('k') => self.ui.popups.process.prev(),
            KeyCode::Down | KeyCode::Char('j') => self.ui.popups.process.next(),
            KeyCode::Char('s') => {
                self.toggle_process_suspend();
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                self.kill_selected_process();
            }
            _ => {}
        }
    }

    /// Toggle suspend/resume for selected process
    fn toggle_process_suspend(&mut self) {
        if let Some(proc) = self.ui.popups.process.get_selected() {
            let id = proc.id.clone();
            let registry = self.runtime.process_registry.clone();

            if proc.is_running() {
                tokio::spawn(async move {
                    if let Err(e) = registry.suspend(&id).await {
                        tracing::error!("Failed to suspend process: {}", e);
                    }
                });
            } else if proc.is_suspended() {
                tokio::spawn(async move {
                    if let Err(e) = registry.resume(&id).await {
                        tracing::error!("Failed to resume process: {}", e);
                    }
                });
            }
        }
    }

    /// Kill selected process
    fn kill_selected_process(&mut self) {
        if let Some(proc) = self.ui.popups.process.get_selected() {
            if proc.is_running() {
                let id = proc.id.clone();

                // Check if this is a terminal pane and close it
                if let Some(idx) = self
                    .runtime
                    .blocks
                    .terminal
                    .iter()
                    .position(|t| t.get_process_id() == Some(&id))
                {
                    self.close_terminal(idx);
                } else {
                    // Not a terminal - just kill via registry
                    let registry = self.runtime.process_registry.clone();
                    tokio::spawn(async move {
                        if let Err(e) = registry.kill(&id).await {
                            tracing::error!("Failed to kill process: {}", e);
                        }
                    });
                }
            }
        }
    }
}
