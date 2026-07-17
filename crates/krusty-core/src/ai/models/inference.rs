use crate::constants;

use super::super::providers::{ProviderId, ReasoningFormat};
use super::metadata::{ApiFormat, ModelMetadata};

/// Infer minimal metadata for an arbitrary model ID.
///
/// This is used when a user selects a model that is not present in the cached
/// catalog but should still be usable with the provider.
pub fn infer_model_metadata(
    provider: ProviderId,
    model_id: &str,
    api_format: ApiFormat,
) -> ModelMetadata {
    let normalized = model_id.trim().to_ascii_lowercase();
    let context_window = resolve_context_window(provider, model_id, api_format);
    let max_output = if context_window >= 1_000_000 {
        65_536
    } else if context_window >= 400_000 {
        128_000
    } else if context_window >= 200_000 {
        100_000
    } else {
        32_768
    };

    let mut metadata =
        ModelMetadata::new(model_id, model_id, provider).with_context(context_window, max_output);

    metadata.api_format = if provider == ProviderId::Grok
        || (provider == ProviderId::OpenAI
            && crate::ai::providers::ProviderConfig::openai_prefers_responses_api(model_id))
    {
        ApiFormat::OpenAIResponses
    } else {
        api_format
    };

    metadata.reasoning_format =
        if normalized.contains("claude") || normalized.starts_with("anthropic/") {
            Some(ReasoningFormat::Anthropic)
        } else if normalized.contains("deepseek") {
            Some(ReasoningFormat::DeepSeek)
        } else if provider == ProviderId::Grok
            || crate::ai::providers::ProviderConfig::openai_prefers_responses_api(model_id)
        {
            Some(ReasoningFormat::OpenAI)
        } else {
            None
        };
    metadata.supports_thinking = metadata.reasoning_format.is_some();
    metadata.reasoning_control = match metadata.reasoning_format {
        Some(ReasoningFormat::OpenAI) => {
            Some(super::super::providers::ReasoningControl::OpenAiEffort)
        }
        Some(ReasoningFormat::Anthropic) => {
            Some(super::super::providers::ReasoningControl::AnthropicBudget)
        }
        Some(ReasoningFormat::DeepSeek) => Some(super::super::providers::ReasoningControl::Boolean),
        None => None,
    };
    if provider == ProviderId::Grok && metadata.supports_thinking {
        metadata.reasoning_control = Some(super::super::providers::ReasoningControl::OutputOnly);
    }
    metadata.supports_tools = true;
    metadata.supports_vision = normalized.contains("gpt-4o")
        || normalized.contains("gpt-4.1")
        || normalized.contains("gpt-5")
        || normalized.contains("gpt-6")
        || normalized.contains("gemini")
        || normalized.contains("claude")
        || normalized.contains("grok");

    metadata
}

/// Resolve best-known metadata for a model by checking built-in provider catalog first,
/// then falling back to heuristic inference.
pub fn resolve_model_metadata(
    provider: ProviderId,
    model_id: &str,
    api_format: ApiFormat,
) -> ModelMetadata {
    if let Some(provider_config) = crate::ai::providers::get_provider(provider) {
        if let Some(model) = provider_config
            .models
            .iter()
            .find(|candidate| candidate.id.eq_ignore_ascii_case(model_id))
        {
            let mut metadata = ModelMetadata::new(&model.id, &model.display_name, provider)
                .with_context(model.context_window, model.max_output);
            metadata.supports_tools = true;
            metadata.reasoning_format = model.reasoning;
            metadata.supports_thinking = model.reasoning.is_some();
            metadata.supported_reasoning_levels = model.supported_reasoning_levels.clone();
            metadata.default_reasoning_level = model.default_reasoning_level;
            metadata.reasoning_is_mandatory = model.reasoning_is_mandatory;
            metadata.reasoning_control = model.reasoning_control;
            metadata.fast_mode = model.fast_mode;
            metadata.supports_vision =
                infer_model_metadata(provider, model_id, api_format).supports_vision;
            metadata.api_format = if provider == ProviderId::Grok
                || (provider == ProviderId::OpenAI
                    && crate::ai::providers::ProviderConfig::openai_prefers_responses_api(
                        &model.id,
                    )) {
                ApiFormat::OpenAIResponses
            } else {
                api_format
            };
            return metadata;
        }
    }

    infer_model_metadata(provider, model_id, api_format)
}

/// Resolve the best-known context window for a model.
///
/// Order of preference:
/// 1. exact built-in provider metadata
/// 2. heuristic inference from model ID / family
/// 3. global default fallback
pub fn resolve_context_window(
    provider: ProviderId,
    model_id: &str,
    api_format: ApiFormat,
) -> usize {
    if let Some(provider_config) = crate::ai::providers::get_provider(provider) {
        if let Some(model) = provider_config
            .models
            .iter()
            .find(|model| model.id.eq_ignore_ascii_case(model_id))
        {
            return model.context_window;
        }
    }

    infer_context_window(model_id, api_format).unwrap_or(constants::ai::CONTEXT_WINDOW_TOKENS)
}

fn infer_context_window(model_id: &str, api_format: ApiFormat) -> Option<usize> {
    let normalized = model_id.trim().to_ascii_lowercase();
    let id = normalized.strip_prefix("openai/").unwrap_or(&normalized);

    if normalized == "grok-build" {
        return Some(512_000);
    }

    if normalized.starts_with("grok-composer-") {
        return Some(200_000);
    }

    if normalized.contains("claude-sonnet-4.5") || normalized.contains("gemini-2.5") {
        return Some(1_000_000);
    }

    if normalized.contains("gemini")
        || normalized.contains("llama-4-maverick")
        || normalized.contains("gemini-2.0")
    {
        return Some(1_000_000);
    }

    if normalized.contains("llama-4-scout") {
        return Some(512_000);
    }

    if normalized.contains("codex") || gpt_major_version(id).is_some_and(|major| major >= 5) {
        return Some(400_000);
    }

    if id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || normalized.contains("gpt-4.1")
        || matches!(api_format, ApiFormat::OpenAIResponses)
    {
        return Some(200_000);
    }

    if normalized.contains("claude") || normalized.contains("glm-5") {
        return Some(200_000);
    }

    if normalized.contains("qwen") || normalized.contains("deepseek") {
        return Some(128_000);
    }

    if normalized.contains("gpt-4") || normalized.contains("gpt-3.5") || normalized.contains("grok")
    {
        return Some(128_000);
    }

    None
}

fn gpt_major_version(model_id: &str) -> Option<u32> {
    let suffix = model_id.strip_prefix("gpt-")?;
    let digits = suffix
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|segment| !segment.is_empty())?;
    digits.parse().ok()
}
