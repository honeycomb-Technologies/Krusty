use serde_json::Value;

use super::AnthropicFormat;
use crate::ai::format::RequestOptions;
use crate::ai::types::AiTool;

impl AnthropicFormat {
    pub(super) fn convert_tools_impl(&self, tools: &[AiTool]) -> Vec<Value> {
        tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect()
    }

    pub(super) fn build_request_body_impl(
        &self,
        model: &str,
        messages: Vec<Value>,
        options: &RequestOptions,
    ) -> Value {
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "max_tokens": options.max_tokens,
        });

        if options.streaming {
            body["stream"] = serde_json::json!(true);
        }

        if let Some(system) = options.system_prompt {
            body["system"] = serde_json::json!(system);
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
