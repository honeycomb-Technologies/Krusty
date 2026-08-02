use crate::tui::app::App;
use crate::tui::blocks::StreamBlock;

impl App {
    pub(super) fn handle_streaming_thinking_delta(&mut self, thinking: String) {
        let needs_block = self
            .runtime
            .blocks
            .thinking
            .last()
            .map(|block| !block.is_streaming())
            .unwrap_or(true);
        if needs_block {
            self.handle_thinking_start();
        }
        self.handle_thinking_delta(thinking);
    }

    /// Handle thinking start event
    pub(super) fn handle_thinking_start(&mut self) {
        self.complete_streaming_blocks();
        self.runtime
            .blocks
            .thinking
            .push(crate::tui::blocks::ThinkingBlock::new());
        self.runtime
            .chat
            .messages
            .push(("thinking".to_string(), String::new()));
    }

    /// Handle thinking delta event
    pub(super) fn handle_thinking_delta(&mut self, thinking: String) {
        if let Some(block) = self.runtime.blocks.thinking.last_mut() {
            block.append(&thinking);
        }
    }

    /// Handle thinking complete event
    pub(super) fn handle_thinking_complete(&mut self, signature: String) {
        let signature_len = signature.len();
        if let Some(block) = self.runtime.blocks.thinking.last_mut() {
            block.set_signature(signature);
            block.complete();
        }
        tracing::info!("ThinkingComplete - signature_len={}", signature_len);
    }
}
