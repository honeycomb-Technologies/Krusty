use anyhow::Result;
use serde_json::Value;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info};

use super::super::config::{anthropic_prompt_cache_control, CallOptions};
use super::super::core::AiClient;
use super::shared::{ensure_success_stream_response, log_request_metrics, start_sse_stream};
use crate::ai::format::anthropic::AnthropicFormat;
use crate::ai::format::FormatHandler;
use crate::ai::parsers::AnthropicParser;
use crate::ai::providers::{ProviderCapabilities, ProviderId};
use crate::ai::reasoning::ReasoningConfig;
use crate::ai::streaming::StreamPart;
use crate::ai::transform::apply_request_body_transform;
use crate::ai::types::ModelMessage;

impl AiClient {
    /// Streaming call using Anthropic format
    pub(super) async fn call_streaming_anthropic(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
        call_start: Instant,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        let format_handler = AnthropicFormat::new();
        let anthropic_messages =
            format_handler.convert_messages(&messages, Some(self.provider_id()));
        let prompt_sections = self.system_prompt_sections(
            &self.config().model,
            &messages,
            options.system_prompt.as_deref(),
            options.tools.as_deref(),
        );
        // Determine max_tokens based on reasoning format
        let fallback_tokens = options.max_tokens.unwrap_or(self.config().max_tokens) as u32;
        let legacy_thinking = options.thinking.is_some();
        let active_reasoning_format = legacy_thinking
            .then_some(options.reasoning_format)
            .flatten();
        let max_tokens = ReasoningConfig::max_tokens_for_format(
            active_reasoning_format,
            fallback_tokens,
            legacy_thinking,
        );

        // Build request body
        let mut body = serde_json::json!({
            "model": self.config().model,
            "messages": anthropic_messages,
            "max_tokens": max_tokens,
            "stream": true,
        });

        // Build system prompt blocks ordered by stability for optimal caching.
        //
        // Prompt caching is prefix-based: the API caches everything from the start
        // up to each cache_control breakpoint. Static content MUST come first so
        // that the maximum prefix is shared across requests.
        //
        // Order (most stable → least stable):
        //   1. CC identity (Anthropic OAuth only) — globally stable, cached
        //   2. Base system prompt (KRUSTY_SYSTEM_PROMPT) — globally stable, cached
        //   3. Project context (CLAUDE.md / KRAB.md) — stable per project, cached
        //   4. Session context (plan state, skills) — dynamic, NOT cached
        //
        // Dynamic session context is appended WITHOUT cache_control so it doesn't
        // invalidate the cached prefix when plan state changes between turns.
        let is_anthropic_oauth = self.provider_id() == ProviderId::Anthropic
            && crate::auth::is_anthropic_oauth_token(self.api_key());

        // Gate caching on both the caller's flag AND the provider's capability.
        // `enable_caching` defaults to true for all providers, but only Anthropic
        // actually supports cache_control blocks. Sending them to MiniMax, Z.ai,
        // etc. may cause errors since they use Anthropic format but don't support caching.
        let provider_caps = ProviderCapabilities::for_provider(self.provider_id());
        let cache_control = anthropic_prompt_cache_control(options, self.provider_id());
        let use_caching = cache_control.is_some();

        if let Some(cache_control) = cache_control.as_ref() {
            let mut system_blocks: Vec<Value> = Vec::new();

            // Block 1 (optional): Anthropic OAuth compatibility identity. Keep
            // this exact transport-required prefix; the shared prompt describes
            // the product role (operating inside Krusty) rather than asserting a
            // second underlying model identity.
            if is_anthropic_oauth {
                system_blocks.push(serde_json::json!({
                    "type": "text",
                    "text": "You are Claude Code, Anthropic's official CLI for Claude.",
                    "cache_control": cache_control.clone()
                }));
            }

            // Block 2: Base system prompt — globally cached, never changes
            if !prompt_sections.base_prompt.is_empty() {
                system_blocks.push(serde_json::json!({
                    "type": "text",
                    "text": prompt_sections.base_prompt.as_str(),
                    "cache_control": cache_control.clone()
                }));
            }

            // Block 3 (optional): Project context — cached per project, stable within session
            if !prompt_sections.project_context.is_empty() {
                system_blocks.push(serde_json::json!({
                    "type": "text",
                    "text": prompt_sections.project_context.as_str(),
                    "cache_control": cache_control.clone()
                }));
                debug!(
                    "Project context block added ({} chars, cached)",
                    prompt_sections.project_context.len()
                );
            }

            // Block 4 (optional): Session context — dynamic, NO cache_control
            // Plan state and skills change frequently. Placing them last without
            // a cache breakpoint means they don't invalidate the static prefix.
            if !prompt_sections.session_context.is_empty() {
                system_blocks.push(serde_json::json!({
                    "type": "text",
                    "text": prompt_sections.session_context.as_str()
                }));
                debug!(
                    "Session context block added ({} chars, not cached)",
                    prompt_sections.session_context.len()
                );
            }

            if !system_blocks.is_empty() {
                body["system"] = Value::Array(system_blocks);
            }
            debug!("System prompt split into cache-optimized blocks");
        } else {
            // No caching: combine everything into a single string
            let system = prompt_sections.combined();
            if !system.is_empty() {
                body["system"] = Value::String(system);
            }
        }

        // Temperature incompatible with reasoning - only add if reasoning is off
        let reasoning_enabled = options.thinking.is_some();
        if !reasoning_enabled {
            if let Some(temp) = options.temperature {
                body["temperature"] = serde_json::json!(temp);
            }
        }

        // Build tools array — sorted deterministically by name.
        // Tool ordering is part of the cached prefix. Non-deterministic ordering
        // (e.g., from HashMap iteration) silently breaks the cache between turns.
        let mut all_tools: Vec<Value> = Vec::new();

        if let Some(tools) = &options.tools {
            let mut sorted_tools: Vec<_> = tools.iter().collect();
            sorted_tools.sort_by(|a, b| a.name.cmp(&b.name));
            for tool in sorted_tools {
                all_tools.push(serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                }));
            }
        }

        // Add server-executed tools based on provider capabilities
        self.add_server_tools(&mut all_tools, &mut body, options, &provider_caps);

        // Add all tools to body — no manual cache breakpoint needed,
        // auto-caching handles the last-block breakpoint automatically.
        if !all_tools.is_empty() {
            body["tools"] = Value::Array(all_tools);
        }

        // Enable auto-caching at the request level.
        // The API automatically places a cache breakpoint on the last cacheable
        // block in the request, so we don't need to manually navigate JSON to
        // find the last tool or last message. Block-level breakpoints on system
        // prompt blocks above still work alongside auto-caching for the static prefix.
        if let Some(cache_control) = cache_control {
            body["cache_control"] = cache_control;
            debug!("Auto-caching enabled at request level");
        }

        // Add reasoning/thinking config
        self.add_reasoning_config(&mut body, options, reasoning_enabled);

        // Add context management
        self.add_context_management(&mut body, options);

        // Add provider-specific parameters
        self.add_provider_params(&mut body, reasoning_enabled);
        if let Some(service_tier) = options.service_tier_for_provider(self.provider_id()) {
            body["service_tier"] = serde_json::json!(service_tier);
        }
        if options.uses_anthropic_fast_mode(self.provider_id()) {
            body["speed"] = serde_json::json!("fast");
        }
        let body = apply_request_body_transform(
            body,
            self.provider_id(),
            self.config().api_format,
            &self.config().model,
        );
        log_request_metrics(
            "anthropic_stream",
            &prompt_sections,
            &messages,
            options.tools.as_deref(),
            options.system_prompt.is_some(),
            if use_caching { "ephemeral" } else { "none" },
            false,
            serde_json::to_vec(&body).map_or(0, |value| value.len()),
        );

        debug!("Calling {} API with streaming", self.provider_id());

        // Build beta headers
        let beta_headers = self.build_beta_headers(options);
        let request = self.build_request_with_beta(&self.config().api_url(), &beta_headers);

        // Send request
        info!("Sending API request...");
        let response = request.json(&body).send().await?;
        let response =
            ensure_success_stream_response(response, call_start, "API response", "API error")
                .await?;

        info!("Starting Anthropic stream processing task");
        Ok(start_sse_stream(
            response,
            AnthropicParser::new(),
            "Anthropic",
            self.provider_id(),
            self.config().api_format,
            &self.config().model,
        ))
    }
}
