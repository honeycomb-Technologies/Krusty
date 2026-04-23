use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::ai::client::AiClient;
use crate::ai::title::generate_title as ai_generate_title;
use crate::ai::types::{Content, ModelMessage, Role};

use super::super::loop_events::LoopEvent;
use super::persistence::save_title;

pub(super) fn maybe_generate_title(
    conversation: &[ModelMessage],
    ai_client: &Arc<AiClient>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    session_id: &str,
    db_path: &Path,
) {
    let first_user_msg = conversation
        .iter()
        .find(|m| m.role == Role::User)
        .and_then(|m| {
            m.content.iter().find_map(|c| {
                if let Content::Text { text } = c {
                    Some(text.clone())
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();

    if first_user_msg.is_empty() {
        return;
    }

    let title_client = ai_client.clone();
    let title_tx = event_tx.clone();
    let title_session_id = session_id.to_string();
    let title_db_path = db_path.to_path_buf();
    tokio::spawn(async move {
        let title = ai_generate_title(&title_client, &first_user_msg).await;
        if !title.is_empty() {
            save_title(&title_db_path, &title_session_id, &title);
            let _ = title_tx.send(LoopEvent::TitleGenerated { title });
        }
    });
}
