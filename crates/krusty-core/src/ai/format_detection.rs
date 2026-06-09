//! API format detection for multi-provider routing
//!
//! Determines the correct API format for a provider/model combination.
//! Used by both ACP and TUI to route requests correctly.

use super::models::ApiFormat;
use super::providers::ProviderId;

/// Detect the appropriate API format for a provider/model combination
///
/// This is the canonical format detection logic used across Krusty.
/// Provider-specific routing:
/// - Grok Build models: OpenAI Responses format through the Grok CLI proxy
/// - OpenAI: OpenAI chat/completions format unless auth/model routing overrides it
/// - All others (OpenRouter, MiniMax, ZAi): Anthropic format
pub fn detect_api_format(provider: ProviderId, model: &str) -> ApiFormat {
    match provider {
        ProviderId::Grok => grok_api_format(model),
        ProviderId::OpenAI => ApiFormat::OpenAI,
        _ => ApiFormat::Anthropic,
    }
}

fn grok_api_format(_model: &str) -> ApiFormat {
    ApiFormat::OpenAIResponses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_api_format_openai_provider() {
        assert!(matches!(
            detect_api_format(ProviderId::OpenAI, "gpt-4"),
            ApiFormat::OpenAI
        ));
    }

    #[test]
    fn test_detect_api_format_grok_provider() {
        assert!(matches!(
            detect_api_format(ProviderId::Grok, "grok-build"),
            ApiFormat::OpenAIResponses
        ));
        assert!(matches!(
            detect_api_format(ProviderId::Grok, "grok-composer-2.5-fast"),
            ApiFormat::OpenAIResponses
        ));
    }

    #[test]
    fn test_detect_api_format_minimax_provider() {
        assert!(matches!(
            detect_api_format(ProviderId::MiniMax, "MiniMax-M2.5"),
            ApiFormat::Anthropic
        ));
    }

    #[test]
    fn test_detect_api_format_openrouter_provider() {
        assert!(matches!(
            detect_api_format(ProviderId::OpenRouter, "anthropic/claude-sonnet-4"),
            ApiFormat::Anthropic
        ));
    }

    #[test]
    fn test_detect_api_format_zai_provider() {
        assert!(matches!(
            detect_api_format(ProviderId::ZAi, "GLM-5"),
            ApiFormat::Anthropic
        ));
    }
}
