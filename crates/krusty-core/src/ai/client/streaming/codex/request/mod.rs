mod content;
mod shared;

use serde_json::Value;
use tracing::debug;

use super::super::super::config::{CallOptions, CodexReasoningEffort};
use super::super::super::core::AiClient;
use crate::ai::format::openai::OpenAIFormat;
use crate::ai::format::FormatHandler;
use crate::ai::transform::apply_request_body_transform;
use crate::ai::types::{Content, ModelMessage, Role};

impl AiClient {
    fn codex_prompt_cache_key(options: &CallOptions) -> Option<String> {
        options.session_id.clone()
    }

    pub(super) fn build_chatgpt_codex_body(
        &self,
        messages: &[ModelMessage],
        system_prompt: &str,
        _max_tokens: usize,
        options: &CallOptions,
        format_handler: &OpenAIFormat,
    ) -> Value {
        let mut input_messages: Vec<Value> = Vec::new();

        for msg in messages.iter().filter(|m| m.role != Role::System) {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
                Role::System => continue,
            };

            let has_tool_results = msg
                .content
                .iter()
                .any(|c| matches!(c, Content::ToolResult { .. }));

            if has_tool_results {
                for content in &msg.content {
                    if let Content::ToolResult {
                        tool_use_id,
                        output,
                        ..
                    } = content
                    {
                        let output_str = match output {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        input_messages.push(serde_json::json!({
                            "type": "function_call_output",
                            "call_id": tool_use_id,
                            "output": output_str
                        }));
                    }
                }
                continue;
            }

            let has_tool_use = msg
                .content
                .iter()
                .any(|c| matches!(c, Content::ToolUse { .. }));

            if has_tool_use && role == "assistant" {
                let text_content = content::collect_message_text(&msg.content, "\n");

                if !text_content.is_empty() {
                    input_messages.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": text_content
                        }]
                    }));
                }

                for content in &msg.content {
                    if let Content::ToolUse { id, name, input } = content {
                        input_messages.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": input.to_string()
                        }));
                    }
                }
                continue;
            }

            if role == "user" {
                let user_content = content::build_codex_user_content(&msg.content);
                if !user_content.is_empty() {
                    input_messages.push(serde_json::json!({
                        "type": "message",
                        "role": "user",
                        "content": user_content
                    }));
                }
                continue;
            }

            let text = content::collect_message_text(&msg.content, "\n");
            if !text.is_empty() {
                let content_type = if role == "assistant" {
                    "output_text"
                } else {
                    "input_text"
                };
                input_messages.push(serde_json::json!({
                    "type": "message",
                    "role": role,
                    "content": [{
                        "type": content_type,
                        "text": text
                    }]
                }));
            }
        }

        let prompt_cache_key = Self::codex_prompt_cache_key(options);
        let thinking_enabled = options.thinking.is_some();
        let reasoning_effort = options
            .codex_reasoning_effort
            .unwrap_or(CodexReasoningEffort::Medium)
            .normalized_for_model(&self.config().model)
            .as_str();

        let mut body = serde_json::json!({
            "model": self.config().model,
            "instructions": system_prompt,
            "input": input_messages,
            "tools": [],
            "tool_choice": "auto",
            "parallel_tool_calls": options.codex_parallel_tool_calls,
            "store": false,
            "stream": true,
            "include": [],
            "text": {
                "verbosity": "medium"
            }
        });

        if let Some(cache_key) = prompt_cache_key {
            body["prompt_cache_key"] = serde_json::json!(cache_key);
        }

        if thinking_enabled {
            body["reasoning"] = serde_json::json!({
                "effort": reasoning_effort,
                "summary": "auto"
            });
            body["include"] = serde_json::json!(["reasoning.encrypted_content"]);
            debug!(
                "ChatGPT Codex: reasoning enabled (effort={}, summary=auto)",
                reasoning_effort
            );
        } else {
            debug!("ChatGPT Codex: reasoning disabled");
        }

        if let Some(tools) = &options.tools {
            let codex_tools = format_handler.convert_tools(tools);
            if !codex_tools.is_empty() {
                body["tools"] = serde_json::json!(codex_tools);
            }
        }

        debug!(
            "ChatGPT Codex request: model={}, {} messages, {} tools",
            self.config().model,
            input_messages.len(),
            options.tools.as_ref().map(|t| t.len()).unwrap_or(0)
        );

        apply_request_body_transform(
            body,
            self.provider_id(),
            self.config().api_format,
            &self.config().model,
        )
    }
}
