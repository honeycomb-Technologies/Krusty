use super::super::providers::ProviderId;
use super::metadata::{ApiFormat, ModelMetadata};

const UNKNOWN_MODEL_CONTEXT_WINDOW: usize = 32_768;
const UNKNOWN_MODEL_MAX_OUTPUT: usize = 4_096;

/// Build conservative metadata for an arbitrary model ID.
///
/// Unknown IDs must not acquire tools, vision, reasoning controls, or a large
/// context window because their name resembles a known family. Users may add
/// explicit custom metadata when a provider does not expose a catalog row.
pub fn infer_model_metadata(
    provider: ProviderId,
    model_id: &str,
    api_format: ApiFormat,
) -> ModelMetadata {
    let mut metadata = ModelMetadata::new(model_id, model_id, provider)
        .with_context(UNKNOWN_MODEL_CONTEXT_WINDOW, UNKNOWN_MODEL_MAX_OUTPUT)
        .with_transport(api_format);
    metadata.supports_tools = false;
    metadata.supports_vision = false;
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
            metadata.supports_tools = model.supports_tools;
            metadata.reasoning_format = model.reasoning;
            metadata.supports_thinking = model.reasoning.is_some();
            metadata.supported_reasoning_levels = model.supported_reasoning_levels.clone();
            metadata.default_reasoning_level = model.default_reasoning_level;
            metadata.reasoning_is_mandatory = model.reasoning_is_mandatory;
            metadata.reasoning_control = model.reasoning_control;
            metadata.fast_mode = model.fast_mode;
            metadata.supports_vision = model.supports_vision;
            metadata.api_format = model.api_format.unwrap_or(api_format);
            return metadata;
        }
    }

    infer_model_metadata(provider, model_id, api_format)
}

/// Resolve the best-known context window for a model.
///
/// Order of preference:
/// 1. exact built-in provider metadata
/// 2. conservative unknown-model fallback
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

    let _ = api_format;
    UNKNOWN_MODEL_CONTEXT_WINDOW
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::providers::{ReasoningControl, ReasoningFormat};

    #[test]
    fn unknown_model_metadata_is_conservative() {
        let metadata = infer_model_metadata(
            ProviderId::OpenAI,
            "gpt-99-vision",
            ApiFormat::OpenAIResponses,
        );
        assert_eq!(metadata.context_window, UNKNOWN_MODEL_CONTEXT_WINDOW);
        assert_eq!(metadata.max_output, UNKNOWN_MODEL_MAX_OUTPUT);
        assert!(!metadata.supports_tools);
        assert!(!metadata.supports_vision);
        assert!(!metadata.supports_thinking);
        assert_eq!(metadata.reasoning_format, None);
        assert_eq!(metadata.reasoning_control, None);
    }

    #[test]
    fn known_rows_use_explicit_provider_metadata() {
        let zai = resolve_model_metadata(ProviderId::ZAi, "glm-5.2", ApiFormat::OpenAI);
        assert_eq!(zai.reasoning_format, Some(ReasoningFormat::OpenAI));
        assert_eq!(zai.reasoning_control, Some(ReasoningControl::OpenAiEffort));

        let minimax =
            resolve_model_metadata(ProviderId::MiniMax, "MiniMax-M3", ApiFormat::Anthropic);
        assert_eq!(
            minimax.reasoning_control,
            Some(ReasoningControl::AnthropicAdaptive)
        );
        assert!(minimax.supports_vision);
    }
}
