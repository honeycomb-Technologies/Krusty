//! Streaming API calls
//!
//! Handles SSE streaming responses from different providers.

mod anthropic;
mod codex;
mod google;
mod openai;
mod request_options;
mod shared;

use anyhow::Result;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::info;

use super::config::CallOptions;
use super::core::AiClient;
use crate::ai::streaming::StreamPart;
use crate::ai::types::ModelMessage;

impl AiClient {
    /// Call the API with streaming response
    pub async fn call_streaming(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        let canonical_options = self.canonical_call_options(&self.config().model, options);
        let call_start = Instant::now();
        info!("=== API CALL START ===");
        info!(
            "Model: {}, Messages: {}, Tools: {}, Thinking: {}, Format: {:?}",
            self.config().model,
            messages.len(),
            canonical_options
                .tools
                .as_ref()
                .map(|t| t.len())
                .unwrap_or(0),
            canonical_options.thinking.is_some(),
            self.config().api_format
        );

        if self.config().uses_openai_format() {
            return self
                .call_streaming_openai(messages, &canonical_options, call_start)
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_streaming_google(messages, &canonical_options, call_start)
                .await;
        }

        self.call_streaming_anthropic(messages, &canonical_options, call_start)
            .await
    }
}
