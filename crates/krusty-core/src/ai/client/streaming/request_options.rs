use serde_json::Value;
use tracing::{debug, info};

use super::super::config::CallOptions;
use super::super::core::AiClient;
use crate::ai::providers::{ProviderCapabilities, ProviderId, ReasoningControl, ReasoningFormat};
use crate::ai::reasoning::{ReasoningConfig, DEFAULT_THINKING_BUDGET};
use crate::ai::transform::build_provider_params;

fn add_anthropic_server_tools(
    all_tools: &mut Vec<Value>,
    options: &CallOptions,
    capabilities: &ProviderCapabilities,
) {
    if capabilities.web_search {
        if let Some(search) = &options.web_search {
            let mut spec = serde_json::json!({
                "type": "web_search_20250305",
                "name": "web_search",
            });
            if let Some(max_uses) = search.max_uses {
                spec["max_uses"] = serde_json::json!(max_uses);
            }
            all_tools.push(spec);
            debug!("Anthropic web search tool enabled (server-side)");
        }
    }

    if capabilities.web_fetch {
        if let Some(fetch) = &options.web_fetch {
            let mut spec = serde_json::json!({
                "type": "web_fetch_20250910",
                "name": "web_fetch",
                "citations": { "enabled": fetch.citations_enabled },
            });
            if let Some(max_uses) = fetch.max_uses {
                spec["max_uses"] = serde_json::json!(max_uses);
            }
            if let Some(max_tokens) = fetch.max_content_tokens {
                spec["max_content_tokens"] = serde_json::json!(max_tokens);
            }
            all_tools.push(spec);
            debug!("Anthropic web fetch tool enabled (server-side)");
        }
    }
}

fn add_openrouter_server_tools(
    all_tools: &mut Vec<Value>,
    options: &CallOptions,
    capabilities: &ProviderCapabilities,
) {
    if capabilities.web_search && options.web_search.is_some() {
        all_tools.push(serde_json::json!({ "type": "openrouter:web_search" }));
        debug!("OpenRouter web search tool enabled (server-side)");
    }

    if capabilities.web_fetch {
        if let Some(fetch) = &options.web_fetch {
            let mut spec = serde_json::json!({ "type": "openrouter:web_fetch" });
            let mut parameters = serde_json::Map::new();
            if let Some(max_uses) = fetch.max_uses {
                parameters.insert("max_uses".to_string(), serde_json::json!(max_uses));
            }
            if let Some(max_tokens) = fetch.max_content_tokens {
                parameters.insert(
                    "max_content_tokens".to_string(),
                    serde_json::json!(max_tokens),
                );
            }
            if !parameters.is_empty() {
                spec["parameters"] = Value::Object(parameters);
            }
            all_tools.push(spec);
            debug!("OpenRouter web fetch tool enabled (server-side)");
        }
    }
}

impl AiClient {
    /// Add server-executed tools (web search, web fetch) to the request
    pub(super) fn add_server_tools(
        &self,
        all_tools: &mut Vec<Value>,
        body: &mut Value,
        options: &CallOptions,
        capabilities: &ProviderCapabilities,
    ) {
        match self.provider_id() {
            ProviderId::Anthropic => {
                add_anthropic_server_tools(all_tools, options, capabilities);
            }
            ProviderId::OpenRouter => {
                add_openrouter_server_tools(all_tools, options, capabilities);
            }
            _ => {}
        }

        // OpenRouter legacy web-search plugin: append :online suffix to model name.
        // Kept only as a fallback for older OpenRouter configurations; current
        // OpenRouter server tools use `openrouter:web_search` / `openrouter:web_fetch`.
        if self.provider_id() == ProviderId::OpenRouter
            && capabilities.web_plugins
            && options.web_search.is_some()
            && !capabilities.web_search
        {
            if let Some(model) = body.get("model").and_then(|m| m.as_str()) {
                if !model.ends_with(":online") {
                    let online_model = format!("{}:online", model);
                    body["model"] = serde_json::json!(online_model);
                    info!(
                        "OpenRouter web search enabled via model suffix: {}",
                        online_model
                    );
                }
            }
        }
    }

    /// Add reasoning/thinking config to the request body
    pub(crate) fn add_reasoning_config(
        &self,
        body: &mut Value,
        options: &CallOptions,
        reasoning_enabled: bool,
    ) {
        if !reasoning_enabled {
            // Sonnet 5 changed Anthropic's default: omitting `thinking` enables
            // adaptive thinking. Preserve Krusty's explicit Off selection with
            // the provider's required disabled wire value. Always-on Fable and
            // Mythos families deliberately keep the field omitted.
            if self.provider_id() == ProviderId::Anthropic
                && options.reasoning_control == Some(ReasoningControl::AnthropicAdaptive)
                && anthropic_adaptive_defaults_on_but_can_disable(&self.config().model)
            {
                body["thinking"] = serde_json::json!({ "type": "disabled" });
            }
            return;
        }

        // MiniMax M3 uses the Anthropic-compatible adaptive toggle without an
        // effort/budget. M2-family models reason mandatorily and reject an
        // explicit Anthropic budget object, so omission is intentional there.
        if self.provider_id() == ProviderId::MiniMax {
            if options.reasoning_control == Some(ReasoningControl::AnthropicAdaptive) {
                body["thinking"] = serde_json::json!({ "type": "adaptive" });
            }
            return;
        }

        // OpenRouter's Messages surface owns the wire shape regardless of the
        // routed upstream provider. Preserve the selected effort through its
        // `thinking` + `output_config.effort` contract.
        if self.provider_id() == ProviderId::OpenRouter {
            let effort = options
                .codex_reasoning_effort
                .map(|value| value.as_str())
                .or_else(|| {
                    options
                        .anthropic_adaptive_effort
                        .map(|value| value.as_str())
                })
                .unwrap_or("high");
            body["thinking"] = serde_json::json!({ "type": "enabled" });
            body["output_config"] = serde_json::json!({ "effort": effort });
            return;
        }

        // Current Claude adaptive-thinking families.
        if options.reasoning_control == Some(ReasoningControl::AnthropicAdaptive) {
            let effort = options
                .anthropic_adaptive_effort
                .map(|e| e.as_str())
                .unwrap_or("high");
            body["thinking"] = serde_json::json!({ "type": "adaptive" });
            body["output_config"] = serde_json::json!({ "effort": effort });
            debug!("Anthropic adaptive thinking enabled (effort={})", effort);
            return;
        }

        let budget_tokens = options.thinking.as_ref().map(|t| t.budget_tokens);

        if let Some(reasoning_config) = ReasoningConfig::build(
            options.reasoning_format,
            reasoning_enabled,
            budget_tokens,
            None,
        ) {
            match options.reasoning_format {
                Some(ReasoningFormat::Anthropic) => {
                    body["thinking"] = reasoning_config;
                    debug!(
                        "Anthropic thinking enabled with budget: {}",
                        budget_tokens.unwrap_or(DEFAULT_THINKING_BUDGET)
                    );
                }
                Some(ReasoningFormat::OpenAI) => {
                    if let Some(obj) = reasoning_config.as_object() {
                        for (k, v) in obj {
                            body[k] = v.clone();
                        }
                    }
                    debug!("OpenAI reasoning enabled with high effort");
                }
                Some(ReasoningFormat::DeepSeek) => {
                    if let Some(obj) = reasoning_config.as_object() {
                        for (k, v) in obj {
                            body[k] = v.clone();
                        }
                    }
                    debug!("DeepSeek reasoning enabled");
                }
                None => {}
            }

            // Opus 4.5 effort config
            if let Some(effort_config) =
                ReasoningConfig::build_opus_effort(&self.config().model, reasoning_enabled)
            {
                body["output_config"] = effort_config;
                debug!("Using high effort for Opus 4.5");
            }
        } else if let Some(thinking) = &options.thinking {
            // Legacy support: if thinking is set without format, assume Anthropic
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": thinking.budget_tokens
            });
            debug!(
                "Legacy thinking enabled with budget: {}",
                thinking.budget_tokens
            );

            if let Some(effort_config) =
                ReasoningConfig::build_opus_effort(&self.config().model, true)
            {
                body["output_config"] = effort_config;
            }
        }
    }

    /// Add context management to the request body
    pub(super) fn add_context_management(&self, body: &mut Value, options: &CallOptions) {
        if let Some(ctx_mgmt) = &options.context_management {
            let caps = ProviderCapabilities::for_provider(self.provider_id());
            if caps.context_management {
                body["context_management"] =
                    serde_json::to_value(ctx_mgmt).unwrap_or(serde_json::Value::Null);
                info!("Context management enabled: {} edits", ctx_mgmt.edits.len());
            } else {
                debug!(
                    "Skipping context_management for provider {:?} (not supported)",
                    self.provider_id()
                );
            }
        }
    }

    /// Add provider-specific parameters to the request body
    pub(crate) fn add_provider_params(&self, body: &mut Value, thinking_enabled: bool) {
        let provider_params =
            build_provider_params(&self.config().model, self.provider_id(), thinking_enabled);

        // Temperature incompatible with reasoning
        if !thinking_enabled {
            if let Some(temp) = provider_params.temperature {
                if let Some(number) = serde_json::Number::from_f64(f64::from(temp)) {
                    body["temperature"] = Value::Number(number);
                    debug!(
                        "Setting temperature: {} for model {}",
                        temp,
                        self.config().model
                    );
                }
            }
        }

        if let Some(top_p) = provider_params.top_p {
            if let Some(num) = serde_json::Number::from_f64(top_p as f64) {
                body["top_p"] = Value::Number(num);
                debug!("Setting top_p: {} for model {}", top_p, self.config().model);
            }
        }

        if let Some(top_k) = provider_params.top_k {
            body["top_k"] = Value::Number(serde_json::Number::from(top_k));
            debug!("Setting top_k: {} for model {}", top_k, self.config().model);
        }

        if let Some(chat_args) = provider_params.chat_template_args {
            body["chat_template_args"] = chat_args;
            info!(
                "Enabling chat_template_args for thinking model {}",
                self.config().model
            );
        }
    }

    /// Build beta headers based on options
    pub(crate) fn build_beta_headers(&self, options: &CallOptions) -> Vec<&'static str> {
        let mut beta_headers: Vec<&str> = Vec::new();

        let is_anthropic_provider = self.provider_id() == ProviderId::Anthropic;
        let is_anthropic_oauth =
            is_anthropic_provider && crate::auth::is_anthropic_oauth_token(self.api_key());

        // Anthropic OAuth: CC identity betas
        if is_anthropic_oauth {
            beta_headers.push("claude-code-20250219");
            beta_headers.push("oauth-2025-04-20");
        }

        // Add thinking beta headers for Anthropic reasoning format
        let anthropic_thinking = options.thinking.is_some()
            && matches!(options.reasoning_format, Some(ReasoningFormat::Anthropic));
        if anthropic_thinking {
            beta_headers.push("interleaved-thinking-2025-05-14");

            // Effort beta for Opus 4.5
            if self.config().model.contains("opus-4-5") {
                beta_headers.push("effort-2025-11-24");
            }
        }

        // Anthropic adaptive thinking needs interleaved-thinking beta.
        if options.reasoning_control == Some(ReasoningControl::AnthropicAdaptive)
            && options.anthropic_adaptive_effort.is_some()
            && !beta_headers.contains(&"interleaved-thinking-2025-05-14")
        {
            beta_headers.push("interleaved-thinking-2025-05-14");
        }

        if options.uses_anthropic_fast_mode(self.provider_id()) {
            beta_headers.push("fast-mode-2026-02-01");
        }

        // Context management beta
        if options.context_management.is_some() {
            beta_headers.push("context-management-2025-06-27");
        }

        // Anthropic web tool beta headers.
        let caps = ProviderCapabilities::for_provider(self.provider_id());
        if self.provider_id() == ProviderId::Anthropic {
            if options.web_search.is_some() && caps.web_search {
                beta_headers.push("web-search-2025-03-05");
            }
            if options.web_fetch.is_some() && caps.web_fetch {
                beta_headers.push("web-fetch-2025-09-10");
            }
        }

        beta_headers
    }
}

fn anthropic_adaptive_defaults_on_but_can_disable(model: &str) -> bool {
    model
        .trim()
        .to_ascii_lowercase()
        .replace(['.', '_', '/', ' '], "-")
        .starts_with("claude-sonnet-5")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::client::{AiClientConfig, AnthropicAdaptiveEffort};
    use crate::ai::models::ApiFormat;
    use crate::ai::providers::{AuthHeader, ReasoningControl, ReasoningFormat};
    use crate::ai::types::ThinkingConfig;
    use crate::ai::types::{WebFetchConfig, WebSearchConfig};

    fn client(provider_id: ProviderId, model: &str) -> AiClient {
        AiClient::new(
            AiClientConfig {
                model: model.to_string(),
                max_tokens: 4096,
                base_url: Some("http://127.0.0.1".to_string()),
                auth_header: AuthHeader::Bearer,
                provider_id,
                api_format: ApiFormat::Anthropic,
                custom_headers: Default::default(),
            },
            "test-key".to_string(),
        )
    }

    #[test]
    fn openrouter_server_tools_use_openrouter_tool_types() {
        let mut tools = Vec::new();
        let options = CallOptions {
            web_search: Some(WebSearchConfig::default()),
            web_fetch: Some(WebFetchConfig::default()),
            ..Default::default()
        };
        let caps = ProviderCapabilities::for_provider(ProviderId::OpenRouter);

        add_openrouter_server_tools(&mut tools, &options, &caps);

        assert_eq!(
            tools[0],
            serde_json::json!({ "type": "openrouter:web_search" })
        );
        assert_eq!(tools[1]["type"], "openrouter:web_fetch");
        assert_eq!(tools[1]["parameters"]["max_uses"], 10);
        assert_eq!(tools[1]["parameters"]["max_content_tokens"], 100_000);
    }

    #[test]
    fn anthropic_server_tools_use_anthropic_tool_versions() {
        let mut tools = Vec::new();
        let options = CallOptions {
            web_search: Some(WebSearchConfig { max_uses: Some(3) }),
            web_fetch: Some(WebFetchConfig::default()),
            ..Default::default()
        };
        let caps = ProviderCapabilities::for_provider(ProviderId::Anthropic);

        add_anthropic_server_tools(&mut tools, &options, &caps);

        assert_eq!(tools[0]["type"], "web_search_20250305");
        assert_eq!(tools[0]["name"], "web_search");
        assert_eq!(tools[0]["max_uses"], 3);
        assert_eq!(tools[1]["type"], "web_fetch_20250910");
        assert_eq!(tools[1]["name"], "web_fetch");
    }

    #[test]
    fn minimax_m3_uses_adaptive_toggle_without_anthropic_budget() {
        let client = client(ProviderId::MiniMax, "MiniMax-M3");
        let options = CallOptions {
            thinking: Some(ThinkingConfig::default()),
            reasoning_format: Some(ReasoningFormat::Anthropic),
            reasoning_control: Some(ReasoningControl::AnthropicAdaptive),
            ..Default::default()
        };
        let mut body = serde_json::json!({});

        client.add_reasoning_config(&mut body, &options, true);

        assert_eq!(body["thinking"], serde_json::json!({"type": "adaptive"}));
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn anthropic_adaptive_wire_shape_comes_from_metadata_not_model_name() {
        let client = client(ProviderId::Anthropic, "claude-future-family");
        let options = CallOptions {
            thinking: Some(ThinkingConfig::default()),
            reasoning_format: Some(ReasoningFormat::Anthropic),
            reasoning_control: Some(ReasoningControl::AnthropicAdaptive),
            anthropic_adaptive_effort: Some(AnthropicAdaptiveEffort::Max),
            ..Default::default()
        };
        let mut body = serde_json::json!({});

        client.add_reasoning_config(&mut body, &options, true);

        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "max");
    }

    #[test]
    fn sonnet_five_off_is_explicit_while_fable_omits_disabled() {
        let options = CallOptions {
            reasoning_format: Some(ReasoningFormat::Anthropic),
            reasoning_control: Some(ReasoningControl::AnthropicAdaptive),
            ..Default::default()
        };

        let sonnet = client(ProviderId::Anthropic, "claude-sonnet-5");
        let mut sonnet_body = serde_json::json!({});
        sonnet.add_reasoning_config(&mut sonnet_body, &options, false);
        assert_eq!(sonnet_body["thinking"]["type"], "disabled");

        let fable = client(ProviderId::Anthropic, "claude-fable-5");
        let mut fable_body = serde_json::json!({});
        fable.add_reasoning_config(&mut fable_body, &options, false);
        assert!(fable_body.get("thinking").is_none());
    }

    #[test]
    fn fractional_temperature_is_not_truncated_to_integer_json() {
        let client = client(ProviderId::OpenRouter, "qwen/example");
        let mut body = serde_json::json!({});

        client.add_provider_params(&mut body, false);

        let temperature = body["temperature"].as_f64().expect("numeric temperature");
        assert!((temperature - 0.55).abs() < 1e-6);
    }
}
