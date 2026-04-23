use crate::ai::client::AiClient;
use crate::tui::app::App;
use crate::tui::utils::TitleUpdate;

impl App {
    /// Spawn background task to generate AI title
    pub(super) fn spawn_title_generation(&mut self, session_id: String, first_message: String) {
        // Need an AI client to generate title
        let client = match self.create_title_client() {
            Some(c) => c,
            None => {
                tracing::debug!("No AI client available for title generation");
                return;
            }
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.runtime.channels.title_update = Some(rx);

        tokio::spawn(async move {
            let title = crate::ai::generate_title(&client, &first_message).await;
            let _ = tx.send(TitleUpdate { session_id, title });
        });
    }

    /// Create AI client for title generation
    fn create_title_client(&self) -> Option<AiClient> {
        self.create_ai_client()
    }
}
