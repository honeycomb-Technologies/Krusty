use crate::agent::AgentEvent;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::tui::app::App;

impl App {
    /// Handle stream error event
    pub(super) fn handle_stream_error(&mut self, error: String) {
        self.runtime.event_bus.emit(AgentEvent::StreamError {
            error: error.clone(),
        });

        self.stop_streaming();
        self.runtime.agent_state.interrupt();
        self.runtime
            .chat
            .messages
            .push(("system".to_string(), format!("Error: {}", error)));

        // If last message was a tool_result, add error assistant message
        let needs_assistant = self
            .runtime
            .chat
            .conversation
            .last()
            .map(|msg| {
                msg.role == Role::User
                    && msg
                        .content
                        .iter()
                        .any(|c| matches!(c, Content::ToolResult { .. }))
            })
            .unwrap_or(false);

        if needs_assistant {
            tracing::debug!("Adding error assistant message after stream error");
            let assistant_msg = ModelMessage {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: format!("[Error: {}]", error),
                }],
            };
            self.runtime.chat.conversation.push(assistant_msg);
            if let Some(saved_msg) = self.runtime.chat.conversation.last() {
                self.save_model_message(saved_msg);
            }
        }
    }
}
