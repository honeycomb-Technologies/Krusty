use crate::tui::app::App;

impl App {
    pub(super) fn append_streaming_assistant_delta(&mut self, delta: String) {
        // Use cached streaming assistant index (O(1)) instead of O(n) scan per delta.
        let append_idx = if let Some(idx) = self.runtime.chat.streaming_assistant_idx {
            if idx < self.runtime.chat.messages.len()
                && idx + 1 == self.runtime.chat.messages.len()
                && self
                    .runtime
                    .chat
                    .messages
                    .get(idx)
                    .map(|(role, _)| role == "assistant")
                    .unwrap_or(false)
            {
                Some(idx)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(idx) = append_idx {
            self.runtime.chat.streaming_assistant_idx = Some(idx);
            if let Some((_, content)) = self.runtime.chat.messages.get_mut(idx) {
                content.push_str(&delta);
            }
        } else {
            // Create new assistant message at end and cache its index
            let new_idx = self.runtime.chat.messages.len();
            self.runtime
                .chat
                .messages
                .push(("assistant".to_string(), delta));
            self.runtime.chat.streaming_assistant_idx = Some(new_idx);
        }
    }
    /// Handle text delta from AI response
    pub(super) fn handle_text_delta(&mut self, delta: String) {
        // Mark all streaming blocks complete when AI starts responding
        self.complete_streaming_blocks();

        // Cache is cleared at the start of each new streaming session (start_streaming),
        // so a None cache means this is the first text delta of a new turn.
        self.append_streaming_assistant_delta(delta);

        if self.ui.scroll_system.scroll.auto_scroll {
            self.ui.scroll_system.scroll.request_scroll_to_bottom();
        }
    }

    /// Handle text delta with citations
    pub(super) fn handle_text_delta_with_citations(
        &mut self,
        delta: String,
        citations: Vec<crate::ai::types::Citation>,
    ) {
        self.append_streaming_assistant_delta(delta);

        if !citations.is_empty() {
            tracing::info!("Received {} citations", citations.len());
            for cite in &citations {
                tracing::debug!("  Citation: {} - {}", cite.title, cite.url);
            }
        }

        if self.ui.scroll_system.scroll.auto_scroll {
            self.ui.scroll_system.scroll.request_scroll_to_bottom();
        }
    }
}
