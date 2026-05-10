use crate::ai::models::{ApiFormat, ModelMetadata};
use crate::ai::providers::{ProviderConfig, ProviderId, ReasoningFormat};

use super::types::OpenAiModel;

const ALLOWED_OPENAI_MODELS: &[&str] = &["gpt-5.5", "gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex"];

pub(super) fn is_useful_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let normalized = id.strip_prefix("openai/").unwrap_or(&id);

    ALLOWED_OPENAI_MODELS.contains(&normalized)
}

pub(super) fn parse_model(raw: OpenAiModel) -> ModelMetadata {
    let id = raw.id;
    let id_lower = id.to_ascii_lowercase();
    let normalized = id_lower.strip_prefix("openai/").unwrap_or(&id_lower);
    let prefers_responses = ProviderConfig::openai_prefers_responses_api(&id);
    let reasoning_format = if prefers_responses {
        Some(ReasoningFormat::OpenAI)
    } else {
        None
    };

    let (context_window, max_output) = if normalized.contains("codex")
        || gpt_major_version(normalized).is_some_and(|major| major >= 5)
    {
        (400_000, 128_000)
    } else if is_openai_o_series(normalized) || normalized.contains("gpt-4.1") {
        (200_000, 100_000)
    } else {
        (128_000, 32_768)
    };

    let supports_vision = normalized.contains("gpt-4o")
        || normalized.contains("gpt-4.1")
        || gpt_major_version(normalized).is_some_and(|major| major >= 5);

    let mut metadata = ModelMetadata::new(&id, &display_name(&id), ProviderId::OpenAI)
        .with_context(context_window, max_output);
    if let Some(format) = reasoning_format {
        metadata = metadata.with_thinking(format);
    }
    metadata.supports_tools = true;
    metadata.supports_vision = supports_vision;
    metadata.api_format = if prefers_responses {
        ApiFormat::OpenAIResponses
    } else {
        ApiFormat::OpenAI
    };
    metadata
}

fn display_name(id: &str) -> String {
    id.split('-')
        .map(|part| match part {
            "gpt" => "GPT".to_string(),
            other if is_openai_o_series(other) => other.to_ascii_uppercase(),
            "codex" => "Codex".to_string(),
            other if other.chars().all(|c| c.is_ascii_digit()) => other.to_string(),
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_openai_o_series(model_id: &str) -> bool {
    model_id
        .strip_prefix('o')
        .and_then(|suffix| suffix.chars().next())
        .is_some_and(|ch| ch.is_ascii_digit())
}

fn gpt_major_version(model_id: &str) -> Option<u32> {
    let suffix = model_id.strip_prefix("gpt-")?;
    let digits = suffix
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|segment| !segment.is_empty())?;
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{display_name, is_useful_model, parse_model};
    use crate::ai::models::ApiFormat;
    use crate::ai::openai::types::OpenAiModel;

    #[test]
    fn keeps_curated_openai_models_only() {
        assert!(is_useful_model("gpt-5.5"));
        assert!(is_useful_model("gpt-5.3-codex"));
        assert!(is_useful_model("gpt-5.4"));
        assert!(is_useful_model("gpt-5.4-mini"));
        assert!(!is_useful_model("gpt-5.5-mini"));
        assert!(!is_useful_model("gpt-5.3-codex-spark"));
        assert!(!is_useful_model("gpt-5.2-pro"));
        assert!(!is_useful_model("whisper-1"));
    }

    #[test]
    fn builds_readable_display_name() {
        assert_eq!(display_name("gpt-5.3-codex"), "GPT 5.3 Codex");
        assert_eq!(display_name("gpt-5.5-mini"), "GPT 5.5 Mini");
        assert_eq!(display_name("gpt-5.4-mini"), "GPT 5.4 Mini");
    }

    #[test]
    fn parses_future_gpt_major_as_responses_with_large_context() {
        let metadata = parse_model(OpenAiModel {
            id: "gpt-6.4".to_string(),
        });

        assert_eq!(metadata.context_window, 400_000);
        assert_eq!(metadata.api_format, ApiFormat::OpenAIResponses);
        assert!(metadata.supports_vision);
    }
}
