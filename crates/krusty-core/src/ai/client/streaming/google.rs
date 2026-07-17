use anyhow::Result;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info};

use super::super::config::CallOptions;
use super::super::core::AiClient;
use super::shared::{ensure_success_stream_response, log_request_metrics, start_sse_stream};
use crate::ai::format::google::GoogleFormat;
use crate::ai::format::FormatHandler;
use crate::ai::parsers::GoogleParser;
use crate::ai::streaming::StreamPart;
use crate::ai::transform::apply_request_body_transform;
use crate::ai::types::ModelMessage;

impl AiClient {
    /// Streaming call using Google format
    pub(super) async fn call_streaming_google(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
        call_start: Instant,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        info!("Using Google/Gemini format for {}", self.config().model);

        let format_handler = GoogleFormat::new();
        let contents = format_handler.convert_messages(&messages, Some(self.provider_id()));
        let prompt_sections = self.system_prompt_sections(
            &self.config().model,
            &messages,
            options.system_prompt.as_deref(),
            options.tools.as_deref(),
        );
        let system_instruction = prompt_sections.combined();

        let max_tokens = options.max_tokens.unwrap_or(self.config().max_tokens);

        let mut body = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": max_tokens,
            }
        });

        body["systemInstruction"] = serde_json::json!({
            "parts": [{"text": system_instruction}]
        });

        if let Some(temp) = options.temperature {
            body["generationConfig"]["temperature"] = serde_json::json!(temp);
        }

        // Sort tools deterministically — Gemini 2.5+ uses implicit prefix caching.
        if let Some(tools) = &options.tools {
            let mut sorted: Vec<_> = tools.to_vec();
            sorted.sort_by(|a, b| a.name.cmp(&b.name));
            let google_tools = format_handler.convert_tools(&sorted);
            if !google_tools.is_empty() {
                body["tools"] = serde_json::json!([{
                    "functionDeclarations": google_tools
                }]);
            }
        }
        let body = apply_request_body_transform(
            body,
            self.provider_id(),
            self.config().api_format,
            &self.config().model,
        );
        log_request_metrics(
            "google_stream",
            &prompt_sections,
            &messages,
            options.tools.as_deref(),
            options.system_prompt.is_some(),
            "implicit",
            false,
            serde_json::to_vec(&body).map_or(0, |value| value.len()),
        );

        debug!("Google request to: {}", self.config().api_url());

        let request = self.build_request(&self.config().api_url());

        info!("Sending Google format request...");
        let response = request.json(&body).send().await?;
        let response =
            ensure_success_stream_response(response, call_start, "API response", "API error")
                .await?;

        info!("Starting Google stream processing task");
        Ok(start_sse_stream(
            response,
            GoogleParser::new(),
            "Google",
            self.provider_id(),
            self.config().api_format,
            &self.config().model,
        ))
    }
}
