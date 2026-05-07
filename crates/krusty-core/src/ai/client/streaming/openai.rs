use anyhow::Result;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info};

use super::super::config::{CallOptions, CodexReasoningEffort};
use super::super::core::AiClient;
use super::shared::{ensure_success_stream_response, log_system_prompt_layers, start_sse_stream};
use crate::ai::format::openai::OpenAIFormat;
use crate::ai::format::FormatHandler;
use crate::ai::models::ApiFormat;
use crate::ai::parsers::OpenAIParser;
use crate::ai::streaming::StreamPart;
use crate::ai::transform::apply_request_body_transform;
use crate::ai::types::ModelMessage;

impl AiClient {
    /// Streaming call using OpenAI format
    pub(super) async fn call_streaming_openai(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
        call_start: Instant,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        // Check if we're using ChatGPT Codex API (OAuth) vs standard OpenAI API
        let is_chatgpt_codex = self
            .config()
            .base_url
            .as_ref()
            .map(|url| url.contains("chatgpt.com"))
            .unwrap_or(false);

        if is_chatgpt_codex {
            info!(
                "Using ChatGPT Codex format for {} (OAuth)",
                self.config().model
            );
            return self
                .call_streaming_chatgpt_codex_ws(messages, options, call_start)
                .await;
        } else {
            info!(
                "Using OpenAI chat/completions format for {}",
                self.config().model
            );
        }

        let format_handler = OpenAIFormat::new(self.config().api_format);
        let prompt_sections = self.system_prompt_sections(
            &self.config().model,
            &messages,
            options.system_prompt.as_deref(),
            options.tools.as_deref(),
        );
        log_system_prompt_layers(
            "openai_stream",
            &prompt_sections,
            options.system_prompt.is_some(),
        );
        let system_prompt = prompt_sections.combined();

        let max_tokens = options.max_tokens.unwrap_or(self.config().max_tokens);

        // Standard OpenAI format (Chat Completions or Responses API)
        let openai_messages = format_handler.convert_messages(&messages, Some(self.provider_id()));

        // Responses API uses "input", Chat Completions uses "messages"
        let (messages_key, max_tokens_key) = if matches!(
            self.config().api_format,
            crate::ai::models::ApiFormat::OpenAIResponses
        ) {
            ("input", "max_output_tokens")
        } else {
            ("messages", "max_tokens")
        };

        let mut body = serde_json::json!({
            "model": self.config().model,
            "stream": true,
        });
        body[max_tokens_key] = serde_json::json!(max_tokens);
        body[messages_key] = serde_json::json!(openai_messages);

        // Add system message at the start
        if let Some(msgs) = body.get_mut(messages_key).and_then(|m| m.as_array_mut()) {
            msgs.insert(
                0,
                serde_json::json!({
                    "role": "system",
                    "content": system_prompt
                }),
            );
        }

        // Add temperature
        if options.thinking.is_none() {
            if let Some(temp) = options.temperature {
                body["temperature"] = serde_json::json!(temp);
            }
        }

        if options.thinking.is_some()
            && matches!(self.config().api_format, ApiFormat::OpenAIResponses)
        {
            let effort = options
                .codex_reasoning_effort
                .unwrap_or(CodexReasoningEffort::High)
                .normalized_for_model(&self.config().model)
                .as_str();
            body["reasoning"] = serde_json::json!({
                "effort": effort
            });
        }

        // Add tools — sorted deterministically for stable prefix ordering.
        // OpenAI uses automatic prefix caching; consistent tool order maximizes hits.
        if let Some(tools) = &options.tools {
            let mut sorted: Vec<_> = tools.to_vec();
            sorted.sort_by(|a, b| a.name.cmp(&b.name));
            let openai_tools = format_handler.convert_tools(&sorted);
            if !openai_tools.is_empty() {
                body["tools"] = serde_json::json!(openai_tools);
            }
        }
        let body = apply_request_body_transform(
            body,
            self.provider_id(),
            self.config().api_format,
            &self.config().model,
        );

        debug!("OpenAI request to: {}", self.config().api_url());

        let request = self.build_request(&self.config().api_url());

        info!("Sending OpenAI format request...");
        let response = request.json(&body).send().await?;
        let response =
            ensure_success_stream_response(response, call_start, "API response", "API error")
                .await?;

        info!("Starting OpenAI stream processing task");
        Ok(start_sse_stream(
            response,
            OpenAIParser::new(),
            "OpenAI",
            self.provider_id(),
            self.config().api_format,
            &self.config().model,
        ))
    }
}
