use crate::ai::providers::{builtin_providers, get_provider, AuthHeader, ProviderId};

#[test]
fn test_builtin_providers() {
    let providers = builtin_providers();
    assert_eq!(providers.len(), 5);
    assert!(providers.iter().any(|p| p.id == ProviderId::MiniMax));
    assert!(providers.iter().any(|p| p.id == ProviderId::OpenRouter));
    assert!(providers.iter().any(|p| p.id == ProviderId::Anthropic));
    assert!(providers.iter().any(|p| p.id == ProviderId::OpenAI));
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
    assert_eq!(provider.default_model(), "MiniMax-M2.5");
}

#[test]
fn test_openrouter_config() {
    let provider = get_provider(ProviderId::OpenRouter).unwrap();
    assert_eq!(provider.base_url, "https://openrouter.ai/api/v1/messages");
    assert_eq!(provider.auth_header, AuthHeader::Bearer);
    assert!(provider.dynamic_models);
}

#[test]
fn test_openai_config_uses_curated_models() {
    let provider = get_provider(ProviderId::OpenAI).unwrap();
    let ids: Vec<_> = provider
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();

    assert_eq!(provider.default_model(), "gpt-5.3-codex");
    assert_eq!(ids, vec!["gpt-5.3-codex", "gpt-5.4", "gpt-5.4-mini"]);
    assert!(provider
        .models
        .iter()
        .all(|model| model.reasoning.is_some()));
}

#[test]
fn test_model_validation() {
    let minimax = get_provider(ProviderId::MiniMax).unwrap();
    assert!(minimax.has_model("MiniMax-M2.5"));
    assert!(!minimax.has_model("anthropic/claude-opus-4.5"));

    let openrouter = get_provider(ProviderId::OpenRouter).unwrap();
    assert!(openrouter.has_model("anthropic/claude-opus-4.5"));
    assert!(openrouter.has_model("openai/gpt-4"));
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
