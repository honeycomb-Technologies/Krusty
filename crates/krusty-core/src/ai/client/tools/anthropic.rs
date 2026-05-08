use anyhow::Result;
use serde_json::Value;
use std::time::Instant;
use tracing::{error, info};

use super::super::config::CallOptions;
use super::super::core::AiClient;
use crate::ai::transform::apply_request_body_transform;

impl AiClient {
    pub(super) async fn call_with_tools_anthropic(
        &self,
        model: &str,
        options: &CallOptions,
        messages: Vec<Value>,
        tools: Vec<Value>,
    ) -> Result<Value> {
        let system_prompt = options.system_prompt.as_deref().unwrap_or_default();
        let max_tokens = options.max_tokens.unwrap_or(self.config().max_tokens);
        let thinking_enabled = options.thinking.is_some();

        // Sort tools deterministically to maintain stable cache prefix.
        // Tool order is part of the cached prefix; non-deterministic order breaks caching.
        let mut sorted_tools = tools;
        sorted_tools.sort_by(|a, b| {
            let name_a = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let name_b = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
            name_a.cmp(name_b)
        });

        // Only apply cache_control for providers that support prompt caching.
        // MiniMax, Z.ai, etc. use Anthropic format but don't support caching —
        // sending cache_control or array-format system prompts may cause errors.
        let capabilities =
            crate::ai::providers::ProviderCapabilities::for_provider(self.provider_id());
        let enable_caching = capabilities.prompt_caching;

        let system_value: Value = if enable_caching {
            serde_json::json!([{
                "type": "text",
                "text": system_prompt,
                "cache_control": {"type": "ephemeral"}
            }])
        } else {
            Value::String(system_prompt.to_string())
        };

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": messages,
            "system": system_value,
            "tools": sorted_tools
        });

        // Enable auto-caching at the request level. The API automatically places
        // the cache breakpoint on the last cacheable block, replacing the need for
        // manual breakpoints on the last tool and last message.
        if enable_caching {
            body["cache_control"] = serde_json::json!({"type": "ephemeral"});
        }

        // Add thinking configuration when enabled
        // MiniMax: Simple thinking without budget_tokens (their API doesn't support it)
        // Z.ai/others: No thinking support for sub-agents
        if thinking_enabled {
            let provider = self.provider_id();
            if provider == crate::ai::providers::ProviderId::MiniMax {
                // MiniMax uses Anthropic-compatible thinking but without budget_tokens
                body["thinking"] = serde_json::json!({
                    "type": "enabled"
                });
            }
        }

        if let Some(service_tier) = options.service_tier_for_provider(self.provider_id()) {
            body["service_tier"] = serde_json::json!(service_tier);
        }

        let body =
            apply_request_body_transform(body, self.provider_id(), self.config().api_format, model);
        let request = self.build_request(&self.config().api_url());

        info!(model = model, provider = %self.provider_id(), "Sub-agent API call starting");
        let start = Instant::now();

        let response = match request.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent API request failed");
                return Err(anyhow::anyhow!("API request failed: {}", e));
            }
        };

        let status = response.status();
        info!(status = %status, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent API response received");

        let response = self.handle_error_response(response).await?;
        let json: Value = response.json().await?;

        info!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            "Sub-agent API call complete"
        );
        Ok(json)
    }
}
