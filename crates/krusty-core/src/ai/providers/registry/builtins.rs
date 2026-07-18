use std::collections::HashMap;

use super::super::config::{
    AuthHeader, FastMode, ModelInfo, ProviderConfig, ProviderId, ReasoningControl, ReasoningEffort,
    ReasoningFormat,
};

pub(super) fn curated_providers() -> Vec<ProviderConfig> {
    vec![
        openrouter_provider(),
        zai_provider(),
        minimax_provider(),
        anthropic_provider(),
        openai_provider(),
        grok_provider(),
    ]
}

fn openrouter_provider() -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::OpenRouter,
        name: "OpenRouter".to_string(),
        description: "Live catalog of routed models".to_string(),
        base_url: "https://openrouter.ai/api/v1/messages".to_string(),
        auth_header: AuthHeader::Bearer,
        models: vec![
            ModelInfo::new(
                "anthropic/claude-opus-4.8",
                "Claude Opus 4.8",
                1_000_000,
                128_000,
            )
            .with_anthropic_thinking()
            .with_reasoning_levels(
                &[
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                    ReasoningEffort::Max,
                ],
                ReasoningEffort::Medium,
            )
            .with_reasoning_control(ReasoningControl::AnthropicAdaptive)
            .with_fast_mode(FastMode::Priority),
            ModelInfo::new(
                "anthropic/claude-fable-5",
                "Claude Fable 5",
                1_000_000,
                128_000,
            )
            .with_anthropic_thinking()
            .with_reasoning_levels(
                &[
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                    ReasoningEffort::Max,
                ],
                ReasoningEffort::Medium,
            )
            .with_mandatory_reasoning()
            .with_reasoning_control(ReasoningControl::AnthropicAdaptive)
            .with_fast_mode(FastMode::Priority),
            ModelInfo::new(
                "anthropic/claude-sonnet-5",
                "Claude Sonnet 5",
                1_000_000,
                128_000,
            )
            .with_anthropic_thinking()
            .with_reasoning_levels(
                &[
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                    ReasoningEffort::Max,
                ],
                ReasoningEffort::Medium,
            )
            .with_reasoning_control(ReasoningControl::AnthropicAdaptive)
            .with_fast_mode(FastMode::Priority),
            ModelInfo::new(
                "anthropic/claude-haiku-4.5",
                "Claude Haiku 4.5",
                200_000,
                64_000,
            )
            .with_anthropic_thinking()
            .with_reasoning_levels(
                &[ReasoningEffort::None, ReasoningEffort::High],
                ReasoningEffort::High,
            )
            .with_reasoning_control(ReasoningControl::Boolean)
            .with_fast_mode(FastMode::Priority),
            ModelInfo::new("openai/gpt-5.6-sol", "GPT-5.6 Sol", 1_050_000, 128_000)
                .with_reasoning(ReasoningFormat::Anthropic)
                .with_reasoning_levels(
                    &[
                        ReasoningEffort::None,
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::XHigh,
                        ReasoningEffort::Max,
                    ],
                    ReasoningEffort::Medium,
                )
                .with_reasoning_control(ReasoningControl::OpenAiEffort)
                .with_fast_mode(FastMode::Priority),
        ],
        supports_tools: true,
        dynamic_models: true,
        pricing_hint: None,
        custom_headers: HashMap::new(),
    }
}

fn zai_provider() -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::ZAi,
        name: "Z.ai".to_string(),
        description: "GLM Coding Plan".to_string(),
        base_url: "https://api.z.ai/api/coding/paas/v4/chat/completions".to_string(),
        auth_header: AuthHeader::Bearer,
        models: vec![
            ModelInfo::new("glm-5.2", "GLM 5.2", 1_000_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI)
                .with_reasoning_levels(
                    &[
                        ReasoningEffort::None,
                        ReasoningEffort::High,
                        ReasoningEffort::Max,
                    ],
                    ReasoningEffort::Max,
                )
                .with_reasoning_control(ReasoningControl::OpenAiEffort),
            ModelInfo::new("glm-5-turbo", "GLM 5 Turbo", 200_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI)
                .with_reasoning_levels(
                    &[ReasoningEffort::None, ReasoningEffort::High],
                    ReasoningEffort::High,
                )
                .with_reasoning_control(ReasoningControl::Boolean),
            ModelInfo::new("glm-4.7", "GLM 4.7", 200_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI)
                .with_reasoning_levels(
                    &[ReasoningEffort::None, ReasoningEffort::High],
                    ReasoningEffort::High,
                )
                .with_reasoning_control(ReasoningControl::Boolean),
        ],
        supports_tools: true,
        dynamic_models: false,
        pricing_hint: None,
        custom_headers: HashMap::new(),
    }
}

fn minimax_provider() -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::MiniMax,
        name: "MiniMax".to_string(),
        description: "M3 + M2 coding models".to_string(),
        base_url: "https://api.minimax.io/anthropic/v1/messages".to_string(),
        auth_header: AuthHeader::XApiKey,
        models: vec![
            ModelInfo::new("MiniMax-M3", "MiniMax M3", 1_000_000, 131_072)
                .with_anthropic_thinking()
                .with_reasoning_levels(
                    &[ReasoningEffort::None, ReasoningEffort::High],
                    ReasoningEffort::High,
                )
                .with_reasoning_control(ReasoningControl::AnthropicAdaptive)
                .with_fast_mode(FastMode::Priority),
            ModelInfo::new("MiniMax-M2.7", "MiniMax M2.7", 204_800, 131_072)
                .with_anthropic_thinking()
                .with_reasoning_levels(&[ReasoningEffort::High], ReasoningEffort::High)
                .with_mandatory_reasoning()
                .with_reasoning_control(ReasoningControl::Boolean),
            ModelInfo::new(
                "MiniMax-M2.7-highspeed",
                "MiniMax M2.7 Highspeed",
                204_800,
                131_072,
            )
            .with_anthropic_thinking()
            .with_reasoning_levels(&[ReasoningEffort::High], ReasoningEffort::High)
            .with_mandatory_reasoning()
            .with_reasoning_control(ReasoningControl::Boolean),
            ModelInfo::new("MiniMax-M2.5", "MiniMax M2.5", 204_800, 131_072)
                .with_anthropic_thinking()
                .with_reasoning_levels(&[ReasoningEffort::High], ReasoningEffort::High)
                .with_mandatory_reasoning()
                .with_reasoning_control(ReasoningControl::Boolean),
            ModelInfo::new(
                "MiniMax-M2.5-highspeed",
                "MiniMax M2.5 Highspeed",
                204_800,
                131_072,
            )
            .with_anthropic_thinking()
            .with_reasoning_levels(&[ReasoningEffort::High], ReasoningEffort::High)
            .with_mandatory_reasoning()
            .with_reasoning_control(ReasoningControl::Boolean),
        ],
        supports_tools: true,
        dynamic_models: true,
        pricing_hint: None,
        custom_headers: HashMap::new(),
    }
}

fn anthropic_provider() -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::Anthropic,
        name: "Anthropic".to_string(),
        description: "Claude 5 + 4.8 (OAuth or API key)".to_string(),
        base_url: "https://api.anthropic.com/v1/messages".to_string(),
        auth_header: AuthHeader::Bearer,
        models: vec![
            ModelInfo::new("claude-opus-4-8", "Claude Opus 4.8", 1_000_000, 128_000)
                .with_anthropic_thinking()
                .with_reasoning_levels(
                    &[
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::XHigh,
                        ReasoningEffort::Max,
                    ],
                    ReasoningEffort::High,
                )
                .with_reasoning_control(ReasoningControl::AnthropicAdaptive)
                .with_fast_mode(FastMode::AnthropicFast),
            ModelInfo::new("claude-fable-5", "Claude Fable 5", 1_000_000, 128_000)
                .with_anthropic_thinking()
                .with_reasoning_levels(
                    &[
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::XHigh,
                        ReasoningEffort::Max,
                    ],
                    ReasoningEffort::High,
                )
                .with_mandatory_reasoning()
                .with_reasoning_control(ReasoningControl::AnthropicAdaptive),
            ModelInfo::new("claude-sonnet-5", "Claude Sonnet 5", 1_000_000, 128_000)
                .with_anthropic_thinking()
                .with_reasoning_levels(
                    &[
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::XHigh,
                        ReasoningEffort::Max,
                    ],
                    ReasoningEffort::High,
                )
                .with_reasoning_control(ReasoningControl::AnthropicAdaptive),
            ModelInfo::new(
                "claude-haiku-4-5-20251001",
                "Claude Haiku 4.5",
                200_000,
                64_000,
            )
            .with_anthropic_thinking()
            .with_reasoning_control(ReasoningControl::AnthropicBudget),
        ],
        supports_tools: true,
        dynamic_models: true,
        pricing_hint: None,
        custom_headers: HashMap::new(),
    }
}

fn grok_provider() -> ProviderConfig {
    let mut custom_headers = HashMap::new();
    custom_headers.insert(
        "x-grok-client-version".to_string(),
        grok_auth::DEFAULT_CLIENT_VERSION.to_string(),
    );
    custom_headers.insert("x-grok-client-identifier".to_string(), "krusty".to_string());
    custom_headers.insert("X-XAI-Token-Auth".to_string(), "xai-grok-cli".to_string());

    ProviderConfig {
        id: ProviderId::Grok,
        name: "Grok".to_string(),
        description: "Grok Build via X subscription auth".to_string(),
        base_url: "https://cli-chat-proxy.grok.com/v1/responses".to_string(),
        auth_header: AuthHeader::Bearer,
        models: vec![
            ModelInfo::new("grok-build", "Grok Build", 512_000, 32_768)
                .with_reasoning(ReasoningFormat::OpenAI)
                .with_reasoning_control(ReasoningControl::OutputOnly),
            ModelInfo::new("grok-composer-2.5-fast", "Composer 2.5", 200_000, 32_768)
                .with_reasoning(ReasoningFormat::OpenAI)
                .with_reasoning_control(ReasoningControl::OutputOnly),
            ModelInfo::new("grok-4.5", "Grok 4.5", 500_000, 32_768)
                .with_reasoning(ReasoningFormat::OpenAI)
                .with_reasoning_control(ReasoningControl::OutputOnly),
        ],
        supports_tools: true,
        dynamic_models: true,
        pricing_hint: Some("Uses your X/Grok subscription session".to_string()),
        custom_headers,
    }
}

fn openai_provider() -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::OpenAI,
        name: "OpenAI".to_string(),
        description: "GPT-5.6 + Codex (OAuth or API key)".to_string(),
        base_url: "https://api.openai.com/v1/chat/completions".to_string(),
        auth_header: AuthHeader::Bearer,
        models: vec![
            // The offline fallback follows the public API contract. Signed-in
            // ChatGPT/Codex discovery is identity-scoped and replaces these
            // rows with the account's exact subscription metadata.
            openai_model(
                "gpt-5.6-sol",
                "GPT-5.6 Sol",
                1_050_000,
                &[
                    ReasoningEffort::None,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                    ReasoningEffort::Max,
                ],
                ReasoningEffort::Medium,
                false,
                true,
            ),
            openai_model(
                "gpt-5.6-terra",
                "GPT-5.6 Terra",
                1_050_000,
                &[
                    ReasoningEffort::None,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                    ReasoningEffort::Max,
                ],
                ReasoningEffort::Medium,
                false,
                true,
            ),
            openai_model(
                "gpt-5.6-luna",
                "GPT-5.6 Luna",
                1_050_000,
                &[
                    ReasoningEffort::None,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                    ReasoningEffort::Max,
                ],
                ReasoningEffort::Medium,
                false,
                true,
            ),
            openai_model(
                "gpt-5.5",
                "GPT-5.5",
                1_050_000,
                &[
                    ReasoningEffort::None,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                ],
                ReasoningEffort::Medium,
                false,
                true,
            ),
            openai_model(
                "gpt-5.5-pro",
                "GPT-5.5 Pro",
                1_050_000,
                &[
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                ],
                ReasoningEffort::High,
                true,
                false,
            ),
            openai_model(
                "gpt-5.4",
                "GPT-5.4",
                1_050_000,
                &[
                    ReasoningEffort::None,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                ],
                ReasoningEffort::None,
                false,
                true,
            ),
            openai_model(
                "gpt-5.4-pro",
                "GPT-5.4 Pro",
                1_050_000,
                &[
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                ],
                ReasoningEffort::Medium,
                true,
                false,
            ),
            openai_model(
                "gpt-5.4-mini",
                "GPT-5.4 Mini",
                400_000,
                &[
                    ReasoningEffort::None,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                ],
                ReasoningEffort::None,
                false,
                true,
            ),
            openai_model(
                "gpt-5.4-nano",
                "GPT-5.4 Nano",
                400_000,
                &[
                    ReasoningEffort::None,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                ],
                ReasoningEffort::None,
                false,
                false,
            ),
            ModelInfo::new("chat-latest", "Chat Latest", 400_000, 128_000),
            ModelInfo::new("gpt-5.3-codex", "GPT-5.3 Codex", 400_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI)
                .with_reasoning_levels(
                    &[
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::XHigh,
                    ],
                    ReasoningEffort::Medium,
                )
                .with_mandatory_reasoning(),
            ModelInfo::new(
                "gpt-5.3-codex-spark",
                "GPT-5.3 Codex Spark",
                128_000,
                32_000,
            )
            .with_reasoning(ReasoningFormat::OpenAI)
            .with_reasoning_levels(
                &[
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                ],
                ReasoningEffort::High,
            )
            .with_mandatory_reasoning(),
        ],
        supports_tools: true,
        dynamic_models: true,
        pricing_hint: None,
        custom_headers: HashMap::new(),
    }
}

fn openai_model(
    id: &str,
    display_name: &str,
    context_window: usize,
    levels: &[ReasoningEffort],
    default: ReasoningEffort,
    mandatory: bool,
    supports_fast: bool,
) -> ModelInfo {
    let mut model = ModelInfo::new(id, display_name, context_window, 128_000)
        .with_reasoning(ReasoningFormat::OpenAI)
        .with_reasoning_levels(levels, default)
        .with_reasoning_control(ReasoningControl::OpenAiEffort);
    if mandatory {
        model = model.with_mandatory_reasoning();
    }
    if supports_fast {
        model = model.with_fast_mode(FastMode::Priority);
    }
    model
}
