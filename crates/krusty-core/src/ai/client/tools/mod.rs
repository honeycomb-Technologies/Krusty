//! Tool-calling API methods
//!
//! Non-streaming calls with tool support, used by sub-agents.

mod anthropic;
mod codex;
mod google;
mod openai;

use anyhow::Result;
use serde_json::Value;

use super::config::{CallOptions, CodexReasoningEffort};
use super::core::AiClient;
use crate::ai::types::ThinkingConfig;

impl AiClient {
    /// Call the API with tools (non-streaming, for sub-agents)
    ///
    /// Used by sub-agents that need tool execution but don't need streaming.
    /// Routes to appropriate format handler based on API format.
    pub async fn call_with_tools(
        &self,
        model: &str,
        system_prompt: &str,
        messages: Vec<Value>,
        tools: Vec<Value>,
        max_tokens: usize,
        thinking_enabled: bool,
        session_id: Option<&str>,
    ) -> Result<Value> {
        let options = self.canonical_call_options(
            model,
            &CallOptions {
                max_tokens: Some(max_tokens),
                system_prompt: Some(system_prompt.to_string()),
                thinking: thinking_enabled.then(ThinkingConfig::default),
                codex_reasoning_effort: thinking_enabled.then_some(CodexReasoningEffort::Medium),
                codex_parallel_tool_calls: true,
                session_id: session_id.map(ToString::to_string),
                ..Default::default()
            },
        );

        if self.config().uses_openai_format() {
            return self
                .call_with_tools_openai(model, &options, messages, tools)
                .await;
        }

        if self.config().uses_google_format() {
            return self
                .call_with_tools_google(model, &options, messages, tools)
                .await;
        }

        self.call_with_tools_anthropic(model, &options, messages, tools)
            .await
    }
}
