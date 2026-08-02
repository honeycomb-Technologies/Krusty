use serde_json::Value;

use super::OpenAIFormat;
use crate::ai::client::config::{
    normalized_prompt_cache_key, openai_prompt_cache_options, openai_prompt_cache_retention,
    OpenAiPromptCacheMode,
};
use crate::ai::format::RequestOptions;
use crate::ai::types::AiTool;

impl OpenAIFormat {
    pub(super) fn convert_tools_impl(&self, tools: &[AiTool]) -> Vec<Value> {
        tools
            .iter()
            .map(|tool| {
                if self.is_responses_format() {
                    serde_json::json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema
                    })
                } else {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.input_schema
                        }
                    })
                }
            })
            .collect()
    }

    pub(super) fn build_request_body_impl(
        &self,
        model: &str,
        messages: Vec<Value>,
        options: &RequestOptions,
    ) -> Value {
        let (messages_key, max_tokens_key) = if self.is_responses_format() {
            ("input", "max_output_tokens")
        } else {
            ("messages", "max_tokens")
        };

        let mut body = serde_json::json!({
            "model": model,
        });

        body[messages_key] = serde_json::json!(messages);
        body[max_tokens_key] = serde_json::json!(options.max_tokens);

        if options.streaming {
            body["stream"] = serde_json::json!(true);
        }

        if self.is_responses_format() {
            if let Some(call_options) = options.call_options {
                if let Some(cache_key) = normalized_prompt_cache_key(call_options) {
                    body["prompt_cache_key"] = serde_json::json!(cache_key);
                }
                if let Some(cache_options) = openai_prompt_cache_options(
                    call_options,
                    model,
                    OpenAiPromptCacheMode::Implicit,
                ) {
                    body["prompt_cache_options"] = cache_options;
                }
                if let Some(retention) = openai_prompt_cache_retention(call_options, model) {
                    body["prompt_cache_retention"] = retention;
                }
            }
        }

        if let Some(system) = options.system_prompt {
            if let Some(msgs) = body.get_mut(messages_key).and_then(|m| m.as_array_mut()) {
                msgs.insert(
                    0,
                    serde_json::json!({
                        "role": "system",
                        "content": system
                    }),
                );
            }
        }

        if let Some(temp) = options.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        if let Some(tools) = options.tools {
            if !tools.is_empty() {
                body["tools"] = serde_json::json!(self.convert_tools_impl(tools));
            }
        }

        body
    }
}
