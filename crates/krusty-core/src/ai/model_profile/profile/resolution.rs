use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;

use super::{ModelProfile, PromptFamily};

impl ModelProfile {
    pub fn resolve(provider: ProviderId, api_format: ApiFormat, model_id: &str) -> Self {
        let normalized = normalize_model_id(model_id);

        if normalized.contains("claude") || normalized.starts_with("anthropic/") {
            return Self {
                prompt_family: PromptFamily::AnthropicClaude,
            };
        }

        if matches!(api_format, ApiFormat::Google)
            || normalized.contains("gemini")
            || normalized.starts_with("google/")
        {
            return Self {
                prompt_family: PromptFamily::GoogleGemini,
            };
        }

        // Grok uses the OpenAI Responses wire format, but its behavioral
        // prompt must remain provider-specific. Resolve it before the generic
        // Responses family so transport compatibility does not erase model
        // steering that is required for reliable tool convergence.
        if matches!(provider, ProviderId::Grok)
            || normalized.starts_with("xai/")
            || normalized.contains("grok")
        {
            return Self {
                prompt_family: PromptFamily::Grok,
            };
        }

        if normalized.contains("codex") {
            return Self {
                prompt_family: PromptFamily::OpenAiCodex,
            };
        }

        if matches!(api_format, ApiFormat::OpenAIResponses)
            || uses_openai_responses_family(&normalized)
        {
            return Self {
                prompt_family: PromptFamily::OpenAiReasoning,
            };
        }

        if matches!(provider, ProviderId::OpenAI) || normalized.starts_with("openai/") {
            return Self {
                prompt_family: PromptFamily::OpenAiReasoning,
            };
        }

        Self {
            prompt_family: PromptFamily::GenericCoding,
        }
    }
}

fn normalize_model_id(model_id: &str) -> String {
    model_id.trim().to_ascii_lowercase()
}

fn uses_openai_responses_family(model_id: &str) -> bool {
    let normalized = model_id.strip_prefix("openai/").unwrap_or(model_id);

    normalized.contains("codex")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
        || gpt_major_version(normalized).is_some_and(|major| major >= 5)
}

fn gpt_major_version(model_id: &str) -> Option<u32> {
    let suffix = model_id.strip_prefix("gpt-")?;
    let digits = suffix
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|segment| !segment.is_empty())?;
    digits.parse().ok()
}
