use anyhow::Result;
use futures::SinkExt;
use serde_json::Value;
use std::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::info;

use super::super::config::CallOptions;
use super::super::core::AiClient;

mod request;
mod response;

const OPENAI_WS_API_VERSION: &str = "responses_websockets=2026-02-06";

impl AiClient {
    pub(super) async fn call_with_tools_chatgpt_codex(
        &self,
        model: &str,
        options: &CallOptions,
        messages: Vec<Value>,
        tools: Vec<Value>,
    ) -> Result<Value> {
        let thinking_enabled = options.thinking.is_some();
        info!(model = model, provider = %self.provider_id(), "Sub-agent ChatGPT Codex API call starting (streaming)");
        let start = Instant::now();

        let body =
            self.build_codex_tool_call_body(model, options, messages, tools, thinking_enabled);

        let ws_url = request::resolve_codex_ws_url_for_tools(&self.config().api_url())?;
        let request = self.build_websocket_request(
            ws_url.as_str(),
            &[
                ("OpenAI-Beta", OPENAI_WS_API_VERSION),
                ("originator", "krusty"),
            ],
        )?;

        info!("Connecting sub-agent Codex websocket: {}", ws_url);
        let (mut ws_stream, _) = connect_async(request).await.map_err(|e| {
            anyhow::anyhow!(
                "Sub-agent Codex websocket connection failed (websocket-only mode): {}",
                e
            )
        })?;

        let create_payload = Self::codex_ws_create_payload(body);
        ws_stream
            .send(Message::Text(create_payload.to_string()))
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Sub-agent Codex websocket request send failed (websocket-only mode): {}",
                    e
                )
            })?;

        let collected_response = self
            .collect_codex_websocket_response(&mut ws_stream, model)
            .await?;

        info!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            "Sub-agent Codex API call complete"
        );
        Ok(collected_response)
    }
}
