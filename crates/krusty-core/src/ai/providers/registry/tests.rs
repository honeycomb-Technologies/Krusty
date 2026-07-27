use crate::ai::providers::{
    builtin_providers, get_provider, AuthHeader, FastMode, ProviderId, ReasoningControl,
    ReasoningEffort,
};

#[test]
fn test_builtin_providers() {
    let providers = builtin_providers();
    assert_eq!(providers.len(), 6);
    assert!(providers.iter().any(|p| p.id == ProviderId::MiniMax));
    assert!(providers.iter().any(|p| p.id == ProviderId::OpenRouter));
    assert!(providers.iter().any(|p| p.id == ProviderId::Anthropic));
    assert!(providers.iter().any(|p| p.id == ProviderId::OpenAI));
    assert!(providers.iter().any(|p| p.id == ProviderId::Grok));
    assert!(providers.iter().any(|p| p.id == ProviderId::ZAi));
}

#[test]
fn test_get_provider() {
    let minimax = get_provider(ProviderId::MiniMax).unwrap();
    assert_eq!(minimax.name, "MiniMax");
    assert!(!minimax.models.is_empty());
}

#[test]
fn test_minimax_config() {
    let provider = get_provider(ProviderId::MiniMax).unwrap();
    assert_eq!(
        provider.base_url,
        "https://api.minimax.io/anthropic/v1/messages"
    );
    assert_eq!(provider.auth_header, AuthHeader::XApiKey);
    assert_eq!(provider.default_model(), "MiniMax-M3");
    let m3 = provider.models.first().expect("M3 fallback");
    assert_eq!(
        m3.reasoning_control,
        Some(ReasoningControl::AnthropicAdaptive)
    );
    assert_eq!(m3.fast_mode, Some(FastMode::Priority));
    assert!(provider.models[1..]
        .iter()
        .all(|model| model.reasoning_is_mandatory));
}

#[test]
fn test_openrouter_config() {
    let provider = get_provider(ProviderId::OpenRouter).unwrap();
    assert_eq!(provider.base_url, "https://openrouter.ai/api/v1/messages");
    assert_eq!(provider.auth_header, AuthHeader::Bearer);
    assert!(provider.dynamic_models);
    let fable = provider
        .models
        .iter()
        .find(|model| model.id == "anthropic/claude-fable-5")
        .expect("OpenRouter Fable fallback");
    assert_eq!(fable.context_window, 1_000_000);
    assert!(fable.reasoning_is_mandatory);
    assert_eq!(fable.fast_mode, Some(FastMode::Priority));

    let sonnet = provider
        .models
        .iter()
        .find(|model| model.id == "anthropic/claude-sonnet-5")
        .expect("OpenRouter Sonnet fallback");
    assert_eq!(sonnet.context_window, 1_000_000);
    assert_eq!(sonnet.max_output, 128_000);
    assert_eq!(
        sonnet.default_reasoning_level,
        Some(ReasoningEffort::Medium)
    );
}

#[test]
fn test_anthropic_fallback_matches_current_public_models() {
    let provider = get_provider(ProviderId::Anthropic).unwrap();
    let fable = provider
        .models
        .iter()
        .find(|model| model.id == "claude-fable-5")
        .expect("Claude Fable 5 fallback");
    assert_eq!(fable.context_window, 1_000_000);
    assert_eq!(fable.max_output, 128_000);
    assert!(fable.reasoning_is_mandatory);
    assert_eq!(
        fable.reasoning_control,
        Some(ReasoningControl::AnthropicAdaptive)
    );

    let opus = provider
        .models
        .iter()
        .find(|model| model.id == "claude-opus-4-8")
        .expect("Claude Opus 4.8 fallback");
    assert_eq!(opus.context_window, 1_000_000);
    assert_eq!(opus.fast_mode, Some(FastMode::AnthropicFast));

    let sonnet = provider
        .models
        .iter()
        .find(|model| model.id == "claude-sonnet-5")
        .expect("Claude Sonnet 5 fallback");
    assert_eq!(sonnet.context_window, 1_000_000);
    assert_eq!(sonnet.max_output, 128_000);

    let haiku = provider
        .models
        .iter()
        .find(|model| model.id == "claude-haiku-4-5-20251001")
        .expect("Claude Haiku 4.5 fallback");
    assert_eq!(haiku.max_output, 64_000);
    assert_eq!(
        haiku.reasoning_control,
        Some(ReasoningControl::AnthropicBudget)
    );
}

#[test]
fn test_openai_config_uses_curated_models() {
    let provider = get_provider(ProviderId::OpenAI).unwrap();
    let ids: Vec<_> = provider
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();

    assert_eq!(provider.default_model(), "gpt-5.6-sol");
    assert_eq!(
        ids,
        vec![
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.5-pro",
            "gpt-5.4",
            "gpt-5.4-pro",
            "gpt-5.4-mini",
            "gpt-5.4-nano",
            "chat-latest",
            "gpt-5.3-codex",
            "gpt-5.3-codex-spark"
        ]
    );
    let sol = provider
        .models
        .iter()
        .find(|model| model.id == "gpt-5.6-sol")
        .expect("GPT-5.6 Sol fallback");
    assert_eq!(sol.context_window, 1_050_000);
    assert_eq!(sol.default_reasoning_level, Some(ReasoningEffort::Medium));
    assert!(!sol.reasoning_is_mandatory);
    assert!(sol
        .supported_reasoning_levels
        .contains(&ReasoningEffort::None));
    assert!(!sol
        .supported_reasoning_levels
        .contains(&ReasoningEffort::Ultra));
    let mini = provider
        .models
        .iter()
        .find(|model| model.id == "gpt-5.4-mini")
        .expect("GPT-5.4 Mini fallback");
    assert_eq!(mini.context_window, 400_000);
    assert_eq!(mini.default_reasoning_level, Some(ReasoningEffort::None));
    assert_eq!(mini.fast_mode, Some(FastMode::Priority));
    let chat_latest = provider
        .models
        .iter()
        .find(|model| model.id == "chat-latest")
        .expect("Chat Latest fallback");
    assert!(chat_latest.reasoning.is_none());
    assert!(provider
        .models
        .iter()
        .filter(|model| model.id != "chat-latest")
        .all(|model| model.reasoning.is_some()));
}

#[test]
fn test_grok_config() {
    let provider = get_provider(ProviderId::Grok).unwrap();
    assert_eq!(provider.name, "Grok");
    assert_eq!(
        provider.base_url,
        "https://cli-chat-proxy.grok.com/v1/responses"
    );
    assert_eq!(provider.auth_header, AuthHeader::Bearer);
    assert_eq!(provider.default_model(), "grok-build");
    assert!(provider.has_model("grok-4.5"));
    assert!(provider.has_model("grok-composer-2.5-fast"));
    assert!(provider
        .models
        .iter()
        .all(|model| model.reasoning.is_some()));

    let build = provider
        .models
        .iter()
        .find(|model| model.id == "grok-build")
        .expect("grok-build");
    assert_eq!(build.reasoning_control, Some(ReasoningControl::OutputOnly));

    let grok_45 = provider
        .models
        .iter()
        .find(|model| model.id == "grok-4.5")
        .expect("grok-4.5");
    assert_eq!(
        grok_45.reasoning_control,
        Some(ReasoningControl::OpenAiEffort)
    );
    assert_eq!(
        grok_45.supported_reasoning_levels,
        vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ]
    );
    assert!(grok_45.reasoning_is_mandatory);

    assert!(provider.supports_tools);
    assert!(provider.dynamic_models);
    assert!(provider
        .custom_headers
        .contains_key("x-grok-client-version"));
    assert_eq!(
        provider
            .custom_headers
            .get("X-XAI-Token-Auth")
            .map(String::as_str),
        Some("xai-grok-cli")
    );
}

#[test]
fn test_model_validation() {
    let minimax = get_provider(ProviderId::MiniMax).unwrap();
    assert!(minimax.has_model("MiniMax-M3"));
    assert!(minimax.has_model("MiniMax-Future"));

    let openrouter = get_provider(ProviderId::OpenRouter).unwrap();
    assert!(openrouter.has_model("anthropic/claude-opus-4.5"));
    assert!(openrouter.has_model("openai/gpt-4"));
}

#[test]
fn test_zai_coding_plan_uses_openai_compatible_transport() {
    let provider = get_provider(ProviderId::ZAi).unwrap();
    assert_eq!(
        provider.base_url,
        "https://api.z.ai/api/coding/paas/v4/chat/completions"
    );
    assert_eq!(provider.auth_header, AuthHeader::Bearer);
    assert_eq!(
        provider.models[0].reasoning_control,
        Some(ReasoningControl::OpenAiEffort)
    );
}

#[test]
fn test_openai_config() {
    let provider = get_provider(ProviderId::OpenAI).unwrap();
    assert_eq!(provider.name, "OpenAI");
    assert_eq!(
        provider.base_url,
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(provider.auth_header, AuthHeader::Bearer);
    assert!(provider.supports_tools);
    assert!(provider.dynamic_models);
    assert!(!provider.models.is_empty());
}
