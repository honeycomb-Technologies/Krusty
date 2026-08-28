use anyhow::Result;
use std::time::Instant;
use tokio::sync::mpsc;

use super::super::config::CallOptions;
use super::super::core::AiClient;
use super::super::RemoteAttemptPolicy;
use crate::ai::streaming::StreamPart;
use crate::ai::types::ModelMessage;

mod request;
pub(crate) mod session;
mod websocket;

impl AiClient {
    pub(super) async fn call_streaming_chatgpt_codex_ws(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
        call_start: Instant,
        attempt_policy: RemoteAttemptPolicy,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        websocket::call_streaming_chatgpt_codex_ws(
            self,
            messages,
            options,
            call_start,
            attempt_policy,
        )
        .await
    }
}
