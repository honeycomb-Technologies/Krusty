use std::sync::LazyLock;

use super::config::ProviderId;
use super::registry::get_provider;

/// Canonical model families that exist across providers.
/// Maps to provider-specific IDs for seamless switching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    ClaudeOpus4_6,
    ClaudeOpus4_5,
    ClaudeSonnet4_5,
    ClaudeSonnet4,
    ClaudeHaiku4_5,
    ClaudeOpus4,
}

/// Model ID mapping entry: (canonical_family, provider, provider_specific_id).
static MODEL_MAPPINGS: LazyLock<Vec<(ModelFamily, ProviderId, &'static str)>> =
    LazyLock::new(|| {
        vec![
            (
                ModelFamily::ClaudeOpus4_6,
                ProviderId::Anthropic,
                "claude-opus-4-6",
            ),
            (
                ModelFamily::ClaudeOpus4_6,
                ProviderId::OpenRouter,
                "anthropic/claude-opus-4.6",
            ),
            (
                ModelFamily::ClaudeOpus4_5,
                ProviderId::OpenRouter,
                "anthropic/claude-opus-4.5",
            ),
            (
                ModelFamily::ClaudeSonnet4_5,
                ProviderId::OpenRouter,
                "anthropic/claude-sonnet-4.5",
            ),
            (
                ModelFamily::ClaudeSonnet4,
                ProviderId::OpenRouter,
                "anthropic/claude-sonnet-4",
            ),
            (
                ModelFamily::ClaudeHaiku4_5,
                ProviderId::OpenRouter,
                "anthropic/claude-haiku-4.5",
            ),
            (
                ModelFamily::ClaudeOpus4,
                ProviderId::OpenRouter,
                "anthropic/claude-opus-4",
            ),
        ]
    });

/// Find the canonical model family for a provider-specific model ID.
pub fn get_model_family(model_id: &str) -> Option<ModelFamily> {
    MODEL_MAPPINGS
        .iter()
        .find(|(_, _, id)| *id == model_id)
        .map(|(family, _, _)| *family)
}

/// Translate a model ID from one provider to another.
/// Returns `None` if no mapping exists.
pub fn translate_model_id(model_id: &str, from: ProviderId, to: ProviderId) -> Option<String> {
    if from == to {
        return Some(model_id.to_string());
    }

    let family = get_model_family(model_id)?;

    MODEL_MAPPINGS
        .iter()
        .find(|(f, p, _)| *f == family && *p == to)
        .map(|(_, _, id)| id.to_string())
}

/// Get the equivalent model ID for a target provider, or the provider's default.
pub fn translate_model_or_default(model_id: &str, from: ProviderId, to: ProviderId) -> String {
    translate_model_id(model_id, from, to).unwrap_or_else(|| {
        get_provider(to)
            .map(|p| p.default_model().to_string())
            .unwrap_or_else(|| "MiniMax-M2.5".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_family_detection() {
        assert_eq!(
            get_model_family("anthropic/claude-opus-4.5"),
            Some(ModelFamily::ClaudeOpus4_5)
        );
        assert_eq!(
            get_model_family("anthropic/claude-sonnet-4"),
            Some(ModelFamily::ClaudeSonnet4)
        );
        assert_eq!(get_model_family("gpt-4"), None);
    }

    #[test]
    fn test_model_translation_same_provider() {
        let translated = translate_model_id(
            "anthropic/claude-opus-4.5",
            ProviderId::OpenRouter,
            ProviderId::OpenRouter,
        );
        assert_eq!(translated, Some("anthropic/claude-opus-4.5".to_string()));
    }

    #[test]
    fn test_model_translation_unknown_model() {
        let translated = translate_model_id("gpt-4", ProviderId::OpenRouter, ProviderId::MiniMax);
        assert_eq!(translated, None);
    }

    #[test]
    fn test_translate_model_or_default() {
        let result = translate_model_or_default("GLM-5", ProviderId::ZAi, ProviderId::MiniMax);
        assert_eq!(result, "MiniMax-M2.5");
    }
}
