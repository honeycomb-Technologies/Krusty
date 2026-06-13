//! Manual in-place compaction (`/pinch`).

use crate::agent::{
    run_compaction_pipeline, CompactionManager, CompactionRequest, CompactionTrigger,
};
use crate::ai::models::resolve_context_window;
use crate::paths;
use crate::tui::app::App;
use crate::tui::blocks::PinchBlock;
use crate::tui::utils::CompactionUpdate;

impl App {
    /// Start the in-chat pinch animation (orchestrator-driven compaction).
    pub fn show_compaction_started(&mut self) {
        if self.runtime.active_pinch_block.is_some() || self.runtime.channels.compaction.is_some() {
            return;
        }
        self.start_pinch_animation();
    }

    fn start_pinch_animation(&mut self) -> usize {
        let idx = self.runtime.blocks.pinch.len();
        self.runtime.blocks.pinch.push(PinchBlock::new());
        self.runtime
            .chat
            .messages
            .push(("pinch".to_string(), String::new()));
        self.runtime.active_pinch_block = Some(idx);
        self.ui.needs_redraw = true;
        idx
    }

    pub(crate) fn finish_pinch_animation(&mut self, success: bool) {
        if let Some(idx) = self.runtime.active_pinch_block.take() {
            if let Some(block) = self.runtime.blocks.pinch.get_mut(idx) {
                block.complete(success);
            }
        }
        self.ui.needs_redraw = true;
    }

    /// Start manual in-place compaction for the current session.
    pub fn start_manual_compaction(&mut self, auto_continue: bool) {
        if self.runtime.chat.conversation.is_empty() {
            self.runtime.chat.messages.push((
                "system".to_string(),
                "No conversation to compact. Start a chat first.".to_string(),
            ));
            return;
        }

        if self.runtime.channels.compaction.is_some() {
            return;
        }

        let Some(session_id) = self.runtime.current_session_id.clone() else {
            self.runtime
                .chat
                .messages
                .push(("system".to_string(), "No active session.".to_string()));
            return;
        };

        self.start_pinch_animation();

        let db_path = paths::config_dir().join("krusty.db");
        let conversation = self.runtime.chat.conversation.clone();
        let working_dir = self.runtime.working_dir.clone();
        let current_model = self.runtime.current_model.clone();
        let project_dir = Some(self.runtime.working_dir.to_string_lossy().into_owned());

        let client = self.create_ai_client();
        let compaction_manager =
            client
                .as_ref()
                .map_or_else(CompactionManager::default, |ai_client| {
                    CompactionManager::for_model(
                        ai_client.provider_id(),
                        ai_client.config().api_format,
                        &current_model,
                        resolve_context_window(
                            ai_client.provider_id(),
                            &current_model,
                            ai_client.config().api_format,
                        ),
                    )
                });

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.runtime.channels.compaction = Some(rx);

        tokio::spawn(async move {
            let result = async {
                let ai_client = client.as_ref();
                run_compaction_pipeline(CompactionRequest {
                    db_path: &db_path,
                    session_id: &session_id,
                    conversation: &conversation,
                    working_dir: &working_dir,
                    ai_client,
                    model: Some(current_model.as_str()),
                    trigger: CompactionTrigger::Manual {
                        preservation_hints: None,
                        direction: None,
                    },
                    compaction_manager,
                    triggering_token_estimate: None,
                    last_usage_prompt_tokens: None,
                    messages_after_usage: 0,
                    summary_override: None,
                    project_dir: project_dir.as_deref(),
                    user_id: None,
                })
                .await
                .map_err(|error| error.to_string())
            }
            .await;

            let _ = tx.send(CompactionUpdate {
                result,
                auto_continue,
            });
        });
    }

    /// Poll for compaction completion.
    pub fn poll_compaction(&mut self) {
        let rx = match self.runtime.channels.compaction.as_mut() {
            Some(rx) => rx,
            None => return,
        };

        match rx.try_recv() {
            Ok(update) => {
                self.runtime.channels.compaction = None;
                match update.result {
                    Ok(result) => {
                        self.runtime.chat.conversation = result.compacted_conversation;
                        self.runtime.context_tokens_used = result.estimated_tokens_after;
                        self.finish_pinch_animation(true);
                        if update.auto_continue {
                            self.send_to_ai();
                        }
                    }
                    Err(error) => {
                        self.finish_pinch_animation(false);
                        self.runtime
                            .chat
                            .messages
                            .push(("system".to_string(), format!("Compaction failed: {error}")));
                    }
                }
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.runtime.channels.compaction = None;
                self.finish_pinch_animation(false);
            }
        }
    }

    /// Show a completed pinch indicator for orchestrator-driven auto compaction.
    pub fn show_auto_compaction_complete(&mut self) {
        let mut block = PinchBlock::new();
        block.complete(true);
        self.runtime.blocks.pinch.push(block);
        self.runtime
            .chat
            .messages
            .push(("pinch".to_string(), String::new()));
        self.ui.needs_redraw = true;
    }
}
