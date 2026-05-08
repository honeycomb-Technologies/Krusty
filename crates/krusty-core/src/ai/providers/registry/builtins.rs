use std::collections::HashMap;

use super::super::config::{AuthHeader, ModelInfo, ProviderConfig, ProviderId, ReasoningFormat};

pub(super) fn curated_providers() -> Vec<ProviderConfig> {
    vec![
        openrouter_provider(),
        zai_provider(),
        minimax_provider(),
        anthropic_provider(),
        openai_provider(),
    ]
}

fn openrouter_provider() -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::OpenRouter,
        name: "OpenRouter".to_string(),
        description: "100+ models (GPT, Gemini, Llama, Claude)".to_string(),
        base_url: "https://openrouter.ai/api/v1/messages".to_string(),
        auth_header: AuthHeader::Bearer,
        models: vec![
            ModelInfo::new(
                "anthropic/claude-opus-4.5",
                "Claude Opus 4.5",
                200_000,
                16_384,
            )
            .with_anthropic_thinking(),
            ModelInfo::new(
                "anthropic/claude-sonnet-4.5",
                "Claude Sonnet 4.5",
                1_000_000,
                16_384,
            )
            .with_anthropic_thinking(),
            ModelInfo::new(
                "anthropic/claude-sonnet-4",
                "Claude Sonnet 4",
                200_000,
                8_192,
            ),
            ModelInfo::new(
                "anthropic/claude-haiku-4.5",
                "Claude Haiku 4.5",
                200_000,
                16_384,
            ),
            ModelInfo::new("anthropic/claude-opus-4", "Claude Opus 4", 200_000, 16_384),
            ModelInfo::new("openai/gpt-5-codex", "GPT-5 Codex", 400_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI),
            ModelInfo::new(
                "google/gemini-2.5-pro-preview",
                "Gemini 2.5 Pro",
                1_000_000,
                65_536,
            ),
            ModelInfo::new(
                "google/gemini-2.5-flash-preview",
                "Gemini 2.5 Flash",
                1_000_000,
                65_536,
            ),
            ModelInfo::new(
                "google/gemini-2.0-flash-001",
                "Gemini 2.0 Flash",
                1_000_000,
                8_192,
            ),
            ModelInfo::new("deepseek/deepseek-r1", "DeepSeek R1", 64_000, 8_192),
            ModelInfo::new(
                "deepseek/deepseek-chat-v3-0324",
                "DeepSeek V3",
                64_000,
                8_192,
            ),
            ModelInfo::new(
                "meta-llama/llama-4-maverick",
                "Llama 4 Maverick",
                1_000_000,
                256_000,
            ),
            ModelInfo::new(
                "meta-llama/llama-4-scout",
                "Llama 4 Scout",
                512_000,
                128_000,
            ),
            ModelInfo::new("qwen/qwen3-235b-a22b", "Qwen 3 235B", 128_000, 8_192),
            ModelInfo::new("qwen/qwq-32b", "QwQ 32B", 128_000, 16_384),
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
        description: "GLM Coding Plan (GLM-5)".to_string(),
        base_url: "https://api.z.ai/api/anthropic/v1/messages".to_string(),
        auth_header: AuthHeader::XApiKey,
        models: vec![ModelInfo::new("GLM-5", "GLM 5", 200_000, 131_072)],
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
        description: "M2.5 (fast, interleaved thinking)".to_string(),
        base_url: "https://api.minimax.io/anthropic/v1/messages".to_string(),
        auth_header: AuthHeader::XApiKey,
        models: vec![
            ModelInfo::new("MiniMax-M2.5", "MiniMax M2.5", 204_800, 131_072)
                .with_anthropic_thinking(),
        ],
        supports_tools: true,
        dynamic_models: false,
        pricing_hint: None,
        custom_headers: HashMap::new(),
    }
}

fn anthropic_provider() -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::Anthropic,
        name: "Anthropic".to_string(),
        description: "Claude Opus 4.6 + Haiku (OAuth or API key)".to_string(),
        base_url: "https://api.anthropic.com/v1/messages".to_string(),
        auth_header: AuthHeader::Bearer,
        models: vec![
            ModelInfo::new("claude-opus-4-6", "Claude Opus 4.6", 200_000, 128_000)
                .with_anthropic_thinking(),
            ModelInfo::new(
                "claude-haiku-4-5-20251001",
                "Claude Haiku 4.5",
                200_000,
                16_384,
            ),
        ],
        supports_tools: true,
        dynamic_models: false,
        pricing_hint: None,
        custom_headers: HashMap::new(),
    }
}

fn openai_provider() -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::OpenAI,
        name: "OpenAI".to_string(),
        description: "GPT-5.5 + Mini + Codex (OAuth or API key)".to_string(),
        base_url: "https://api.openai.com/v1/chat/completions".to_string(),
        auth_header: AuthHeader::Bearer,
        models: vec![
            ModelInfo::new("gpt-5.5", "GPT-5.5", 400_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI),
            ModelInfo::new("gpt-5.5-mini", "GPT-5.5 Mini", 400_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI),
            ModelInfo::new("gpt-5.3-codex", "GPT-5.3 Codex", 400_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI),
            ModelInfo::new("gpt-5.4", "GPT-5.4", 400_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI),
            ModelInfo::new("gpt-5.4-mini", "GPT-5.4 Mini", 400_000, 128_000)
                .with_reasoning(ReasoningFormat::OpenAI),
        ],
        supports_tools: true,
        dynamic_models: true,
        pricing_hint: None,
        custom_headers: HashMap::new(),
    }
}
