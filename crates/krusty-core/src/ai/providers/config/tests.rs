use crate::ai::models::ApiFormat;
use crate::ai::providers::{ProviderConfig, ProviderId};
use crate::auth::{AuthMethod, OpenAIAuthType};

#[test]
fn test_provider_id_display() {
    assert_eq!(ProviderId::MiniMax.to_string(), "MiniMax");
    assert_eq!(ProviderId::OpenRouter.to_string(), "OpenRouter");
    assert_eq!(ProviderId::ZAi.to_string(), "Z.ai");
    assert_eq!(ProviderId::Anthropic.to_string(), "Anthropic");
    assert_eq!(ProviderId::OpenAI.to_string(), "OpenAI");
}

#[test]
fn test_storage_keys() {
    assert_eq!(ProviderId::MiniMax.storage_key(), "minimax");
    assert_eq!(ProviderId::ZAi.storage_key(), "z_ai");
    assert_eq!(ProviderId::Anthropic.storage_key(), "anthropic");
    assert_eq!(ProviderId::OpenAI.storage_key(), "openai");
}

#[test]
fn test_openai_routing_prefers_responses_for_gpt5_and_codex() {
    assert!(ProviderConfig::openai_prefers_responses_api("gpt-5"));
    assert!(ProviderConfig::openai_prefers_responses_api(
        "gpt-5.3-codex"
    ));
    assert!(ProviderConfig::openai_prefers_responses_api("gpt-6.4"));
    assert!(ProviderConfig::openai_prefers_responses_api("o5"));
    assert_eq!(
        ProviderConfig::openai_format_for_auth("gpt-5.3-codex", OpenAIAuthType::ApiKey),
        ApiFormat::OpenAIResponses
    );
    assert_eq!(
        ProviderConfig::openai_format_for_auth("gpt-4.1", OpenAIAuthType::ApiKey),
        ApiFormat::OpenAI
    );
}

#[test]
fn test_oauth_support() {
    assert!(ProviderId::OpenAI.supports_oauth());
    let openai_methods = ProviderId::OpenAI.auth_methods();
    assert!(openai_methods.contains(&AuthMethod::OAuthBrowser));
    assert!(openai_methods.contains(&AuthMethod::OAuthDevice));
    assert!(openai_methods.contains(&AuthMethod::ApiKey));

    assert!(ProviderId::Anthropic.supports_oauth());
    let anthropic_methods = ProviderId::Anthropic.auth_methods();
    assert!(anthropic_methods.contains(&AuthMethod::OAuthBrowser));
    assert!(!anthropic_methods.contains(&AuthMethod::OAuthDevice));
    assert!(anthropic_methods.contains(&AuthMethod::ApiKey));

    assert!(!ProviderId::MiniMax.supports_oauth());
    let minimax_methods = ProviderId::MiniMax.auth_methods();
    assert_eq!(minimax_methods, vec![AuthMethod::ApiKey]);
}
