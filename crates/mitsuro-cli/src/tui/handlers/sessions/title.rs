use crate::tui::app::App;
use crate::tui::utils::TitleUpdate;

impl App {
    /// Queue a zero-token local title update through the existing UI channel.
    pub(super) fn spawn_title_generation(&mut self, session_id: String, first_message: String) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.runtime.channels.title_update = Some(rx);
        let title = crate::ai::derive_title(&first_message);
        let _ = tx.send(TitleUpdate { session_id, title });
    }
}
