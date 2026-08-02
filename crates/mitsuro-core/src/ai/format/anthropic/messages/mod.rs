mod content;
mod sanitize;

use serde_json::Value;
use tracing::{debug, info};

use super::AnthropicFormat;
use crate::ai::format::needs_role_alternation_filler;
use crate::ai::providers::{ProviderCapabilities, ProviderId};
use crate::ai::types::{Content, ModelMessage, Role};

impl AnthropicFormat {
    pub(super) fn convert_messages_impl(
        &self,
        messages: &[ModelMessage],
        provider_id: Option<ProviderId>,
    ) -> Vec<Value> {
        let mut result: Vec<Value> = Vec::new();
        let mut last_role: Option<&str> = None;

        info!("Converting {} messages for Anthropic API", messages.len());

        let preserve_all_thinking = provider_id == Some(ProviderId::MiniMax);
        let include_signature = provider_id != Some(ProviderId::MiniMax);
        let strip_images = provider_id
            .map(|pid| !ProviderCapabilities::for_provider(pid).supports_vision)
            .unwrap_or(false);

        let non_system_messages: Vec<_> =
            messages.iter().filter(|m| m.role != Role::System).collect();

        let last_assistant_with_tools_idx = if preserve_all_thinking {
            None
        } else {
            let mut idx = None;
            for (i, msg) in non_system_messages.iter().enumerate() {
                if msg.role == Role::Assistant
                    && msg
                        .content
                        .iter()
                        .any(|c| matches!(c, Content::ToolUse { .. }))
                    && i + 1 < non_system_messages.len()
                    && (non_system_messages[i + 1].role == Role::Tool
                        || non_system_messages[i + 1]
                            .content
                            .iter()
                            .any(|c| matches!(c, Content::ToolResult { .. })))
                {
                    idx = Some(i);
                }
            }
            idx
        };

        for (i, msg) in non_system_messages.iter().enumerate() {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "user",
                Role::System => unreachable!(),
            };

            if let Some(filler_role) = needs_role_alternation_filler(last_role, role, &[]) {
                debug!(
                    "Inserting filler {} message to maintain alternation",
                    filler_role
                );
                result.push(serde_json::json!({
                    "role": filler_role,
                    "content": [{
                        "type": "text",
                        "text": "."
                    }]
                }));
            }

            let include_thinking =
                preserve_all_thinking || last_assistant_with_tools_idx == Some(i);
            let content: Vec<Value> = msg
                .content
                .iter()
                .filter_map(|c| {
                    content::convert_content(c, include_thinking, include_signature, strip_images)
                })
                .collect();

            result.push(serde_json::json!({
                "role": role,
                "content": content
            }));

            last_role = Some(role);
        }

        sanitize::sanitize_tool_results(&mut result);

        result
    }
}

#[cfg(test)]
impl AnthropicFormat {
    pub(super) fn sanitize_tool_results(messages: &mut Vec<Value>) {
        sanitize::sanitize_tool_results(messages);
    }
}
