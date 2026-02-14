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
/// - OpenAI: OpenAI chat/completions format
/// - All others (OpenRouter, MiniMax, ZAi): Anthropic format
pub fn detect_api_format(provider: ProviderId, _model: &str) -> ApiFormat {
    match provider {
        ProviderId::OpenAI => ApiFormat::OpenAI,
        _ => ApiFormat::Anthropic,
    }
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
    fn test_detect_api_format_minimax_provider() {
        assert!(matches!(
            detect_api_format(ProviderId::MiniMax, "MiniMax-M2.1"),
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
            detect_api_format(ProviderId::ZAi, "GLM-4.7"),
            ApiFormat::Anthropic
        ));
    }
}
