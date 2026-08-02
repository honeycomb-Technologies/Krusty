//! Terminal pane handlers
//!
//! Terminal management operations.

use crate::tui::app::App;

impl App {
    /// Close a terminal pane by index
    pub fn close_terminal(&mut self, idx: usize) {
        // Get the process_id before we close (needed for message lookup)
        let process_id = if idx < self.runtime.blocks.terminal.len() {
            self.runtime.blocks.terminal[idx]
                .get_process_id()
                .map(|s| s.to_string())
        } else {
            None
        };

        // Unregister from process registry before removing
        if let Some(ref id) = process_id {
            let registry = self.runtime.process_registry.clone();
            let id_clone = id.clone();
            tokio::spawn(async move {
                registry.unregister(&id_clone).await;
            });
        }

        // Close the terminal (handles focus/pin adjustments)
        self.runtime.blocks.close_terminal(idx);

        // Remove the corresponding "terminal" message by process_id (reliable lookup)
        if let Some(ref pid) = process_id {
            if let Some(msg_idx) = self
                .runtime
                .chat
                .messages
                .iter()
                .position(|(role, content)| role == "terminal" && content == pid)
            {
                self.runtime.chat.messages.remove(msg_idx);
            }
        }
    }
}
