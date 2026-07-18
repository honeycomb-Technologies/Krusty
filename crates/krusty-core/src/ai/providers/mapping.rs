use std::sync::LazyLock;

use super::config::ProviderId;
use super::registry::get_provider;

/// Canonical model families that exist across providers.
/// Maps to provider-specific IDs for seamless switching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    ClaudeFable5,
    ClaudeOpus4_8,
    ClaudeSonnet5,
    ClaudeOpus4_6,
    ClaudeOpus4_5,
    ClaudeSonnet4_5,
    ClaudeSonnet4,
    ClaudeHaiku4_5,
    ClaudeOpus4,
    Gpt5_6Sol,
    Gpt5_6Terra,
    Gpt5_6Luna,
    Gpt5_5,
    Gpt5_5Pro,
    Gpt5_4,
    Gpt5_4Pro,
    Gpt5_4Mini,
    Gpt5_4Nano,
    MiniMaxM3,
    Glm5_2,
}

/// Model ID mapping entry: (canonical_family, provider, provider_specific_id).
static MODEL_MAPPINGS: LazyLock<Vec<(ModelFamily, ProviderId, &'static str)>> =
    LazyLock::new(|| {
        vec![
            (
                ModelFamily::ClaudeFable5,
                ProviderId::Anthropic,
                "claude-fable-5",
            ),
            (
                ModelFamily::ClaudeFable5,
                ProviderId::OpenRouter,
                "anthropic/claude-fable-5",
            ),
            (
                ModelFamily::ClaudeOpus4_8,
                ProviderId::Anthropic,
                "claude-opus-4-8",
            ),
            (
                ModelFamily::ClaudeOpus4_8,
                ProviderId::OpenRouter,
                "anthropic/claude-opus-4.8",
            ),
            (
                ModelFamily::ClaudeSonnet5,
                ProviderId::Anthropic,
                "claude-sonnet-5",
            ),
            (
                ModelFamily::ClaudeSonnet5,
                ProviderId::OpenRouter,
                "anthropic/claude-sonnet-5",
            ),
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
                ProviderId::Anthropic,
                "claude-haiku-4-5-20251001",
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
            (ModelFamily::Gpt5_6Sol, ProviderId::OpenAI, "gpt-5.6-sol"),
            (
                ModelFamily::Gpt5_6Sol,
                ProviderId::OpenRouter,
                "openai/gpt-5.6-sol",
            ),
            (
                ModelFamily::Gpt5_6Terra,
                ProviderId::OpenAI,
                "gpt-5.6-terra",
            ),
            (
                ModelFamily::Gpt5_6Terra,
                ProviderId::OpenRouter,
                "openai/gpt-5.6-terra",
            ),
            (ModelFamily::Gpt5_6Luna, ProviderId::OpenAI, "gpt-5.6-luna"),
            (
                ModelFamily::Gpt5_6Luna,
                ProviderId::OpenRouter,
                "openai/gpt-5.6-luna",
            ),
            (ModelFamily::Gpt5_5, ProviderId::OpenAI, "gpt-5.5"),
            (
                ModelFamily::Gpt5_5,
                ProviderId::OpenRouter,
                "openai/gpt-5.5",
            ),
            (ModelFamily::Gpt5_5Pro, ProviderId::OpenAI, "gpt-5.5-pro"),
            (
                ModelFamily::Gpt5_5Pro,
                ProviderId::OpenRouter,
                "openai/gpt-5.5-pro",
            ),
            (ModelFamily::Gpt5_4, ProviderId::OpenAI, "gpt-5.4"),
            (
                ModelFamily::Gpt5_4,
                ProviderId::OpenRouter,
                "openai/gpt-5.4",
            ),
            (ModelFamily::Gpt5_4Pro, ProviderId::OpenAI, "gpt-5.4-pro"),
            (
                ModelFamily::Gpt5_4Pro,
                ProviderId::OpenRouter,
                "openai/gpt-5.4-pro",
            ),
            (ModelFamily::Gpt5_4Mini, ProviderId::OpenAI, "gpt-5.4-mini"),
            (
                ModelFamily::Gpt5_4Mini,
                ProviderId::OpenRouter,
                "openai/gpt-5.4-mini",
            ),
            (ModelFamily::Gpt5_4Nano, ProviderId::OpenAI, "gpt-5.4-nano"),
            (
                ModelFamily::Gpt5_4Nano,
                ProviderId::OpenRouter,
                "openai/gpt-5.4-nano",
            ),
            (ModelFamily::MiniMaxM3, ProviderId::MiniMax, "MiniMax-M3"),
            (
                ModelFamily::MiniMaxM3,
                ProviderId::OpenRouter,
                "minimax/minimax-m3",
            ),
            (ModelFamily::Glm5_2, ProviderId::ZAi, "glm-5.2"),
            (ModelFamily::Glm5_2, ProviderId::OpenRouter, "z-ai/glm-5.2"),
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

    let family = MODEL_MAPPINGS
        .iter()
        .find(|(_, provider, id)| *provider == from && *id == model_id)
        .map(|(family, _, _)| *family)?;

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
            .unwrap_or_else(|| "MiniMax-M3".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_family_detection() {
        assert_eq!(
            get_model_family("anthropic/claude-opus-4.8"),
            Some(ModelFamily::ClaudeOpus4_8)
        );
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
    fn test_current_cross_provider_translations() {
        assert_eq!(
            translate_model_id("gpt-5.6-sol", ProviderId::OpenAI, ProviderId::OpenRouter,),
            Some("openai/gpt-5.6-sol".to_string())
        );
        assert_eq!(
            translate_model_id(
                "anthropic/claude-fable-5",
                ProviderId::OpenRouter,
                ProviderId::Anthropic,
            ),
            Some("claude-fable-5".to_string())
        );
        assert_eq!(
            translate_model_id("MiniMax-M3", ProviderId::MiniMax, ProviderId::OpenRouter),
            Some("minimax/minimax-m3".to_string())
        );
        assert_eq!(
            translate_model_id("glm-5.2", ProviderId::ZAi, ProviderId::OpenRouter),
            Some("z-ai/glm-5.2".to_string())
        );
    }

    #[test]
    fn translation_respects_the_source_provider() {
        assert_eq!(
            translate_model_id("gpt-5.6-sol", ProviderId::Anthropic, ProviderId::OpenRouter),
            None
        );
    }

    #[test]
    fn test_model_translation_unknown_model() {
        let translated = translate_model_id("gpt-4", ProviderId::OpenRouter, ProviderId::MiniMax);
        assert_eq!(translated, None);
    }

    #[test]
    fn test_translate_model_or_default() {
        let result = translate_model_or_default("GLM-5", ProviderId::ZAi, ProviderId::MiniMax);
        assert_eq!(result, "MiniMax-M3");
    }
}
