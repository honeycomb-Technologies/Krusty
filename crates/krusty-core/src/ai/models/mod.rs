//! Model metadata and registry
//!
//! Central management for AI models from all providers.
//! Supports static models (built-in) and dynamic models (fetched from APIs).

mod inference;
mod metadata;
mod registry;

use std::collections::HashMap;

pub use inference::{infer_model_metadata, resolve_context_window, resolve_model_metadata};
pub use metadata::{
    dynamic_model_cache_ttl, model_catalog_fingerprint, ApiFormat, DynamicModelCacheMetadata,
    ModelMetadata,
};
pub use registry::{create_model_registry, ModelRegistry, SharedModelRegistry};

use super::providers::ProviderId;

pub type ModelsByProvider = HashMap<ProviderId, Vec<ModelMetadata>>;
pub type OrganizedModels = (Vec<ModelMetadata>, ModelsByProvider);

#[cfg(test)]
mod tests {
    use super::{infer_model_metadata, resolve_context_window, resolve_model_metadata, ApiFormat};
    use crate::ai::providers::{ProviderId, ReasoningFormat};

    #[test]
    fn resolves_exact_static_context_window() {
        assert_eq!(
            resolve_context_window(
                ProviderId::Anthropic,
                "claude-opus-4-6",
                ApiFormat::Anthropic
            ),
            200_000
        );
    }

    #[test]
    fn infers_future_openai_reasoning_model_window() {
        assert_eq!(
            resolve_context_window(
                ProviderId::OpenAI,
                "gpt-6.4-codex",
                ApiFormat::OpenAIResponses
            ),
            400_000
        );
    }

    #[test]
    fn infers_gemini_window_for_dynamic_model_ids() {
        assert_eq!(
            resolve_context_window(
                ProviderId::OpenRouter,
                "google/gemini-2.5-pro",
                ApiFormat::Anthropic,
            ),
            1_000_000
        );
    }

    #[test]
    fn infers_custom_openai_metadata_for_manual_model_ids() {
        let metadata =
            infer_model_metadata(ProviderId::OpenAI, "gpt-6.4", ApiFormat::OpenAIResponses);

        assert_eq!(metadata.id, "gpt-6.4");
        assert_eq!(metadata.api_format, ApiFormat::OpenAIResponses);
        assert_eq!(metadata.context_window, 400_000);
        assert!(metadata.supports_thinking);
    }

    #[test]
    fn resolves_builtin_reasoning_metadata_before_fallback() {
        let metadata =
            resolve_model_metadata(ProviderId::MiniMax, "MiniMax-M2.5", ApiFormat::Anthropic);

        assert_eq!(metadata.reasoning_format, Some(ReasoningFormat::Anthropic));
        assert!(metadata.supports_thinking);
        assert_eq!(metadata.context_window, 204_800);
    }

    #[test]
    fn resolves_grok_build_metadata_as_responses_reasoning() {
        let metadata =
            resolve_model_metadata(ProviderId::Grok, "grok-build", ApiFormat::OpenAIResponses);

        assert_eq!(metadata.api_format, ApiFormat::OpenAIResponses);
        assert_eq!(metadata.reasoning_format, Some(ReasoningFormat::OpenAI));
        assert!(metadata.supports_thinking);
        assert_eq!(metadata.context_window, 512_000);
    }
}
