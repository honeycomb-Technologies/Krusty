use crate::ai::models::{ApiFormat, ModelMetadata};
use crate::ai::providers::{
    FastMode, ProviderConfig, ProviderId, ReasoningControl, ReasoningEffort, ReasoningFormat,
};

use super::types::{ChatGptModel, OpenAiModel};

const GPT_56_LEVELS: &[ReasoningEffort] = &[
    ReasoningEffort::None,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::XHigh,
    ReasoningEffort::Max,
];
const GPT_55_LEVELS: &[ReasoningEffort] = &[
    ReasoningEffort::None,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::XHigh,
];
const GPT_54_LEVELS: &[ReasoningEffort] = &[
    ReasoningEffort::None,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::XHigh,
];
const GPT_55_PRO_LEVELS: &[ReasoningEffort] = &[
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::XHigh,
];
const GPT_PRO_LEVELS: &[ReasoningEffort] = &[
    ReasoningEffort::Minimal,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::XHigh,
];
const GPT_CODEX_LEVELS: &[ReasoningEffort] = &[
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::XHigh,
];
const GPT_GENERIC_LEVELS: &[ReasoningEffort] = &[
    ReasoningEffort::Minimal,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
];
const O_SERIES_LEVELS: &[ReasoningEffort] = &[
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
];

#[derive(Clone, Copy)]
struct CapabilityProfile {
    context_window: usize,
    max_output: usize,
    reasoning_levels: &'static [ReasoningEffort],
    default_reasoning: Option<ReasoningEffort>,
    reasoning_is_mandatory: bool,
    fast_mode: Option<FastMode>,
    supports_vision: bool,
}

pub(super) fn is_useful_model(id: &str) -> bool {
    let id = id.trim().to_ascii_lowercase();
    let normalized = id.strip_prefix("openai/").unwrap_or(&id);

    if normalized.is_empty()
        || is_snapshot_id(normalized)
        || contains_non_chat_capability(normalized)
    {
        return false;
    }

    gpt_major_version(normalized).is_some_and(|major| major >= 4)
        || is_openai_o_series(normalized)
        || normalized.starts_with("chatgpt-")
        || normalized.starts_with("codex-")
}

pub(super) fn parse_model(raw: OpenAiModel) -> ModelMetadata {
    let id = raw.id;
    let id_lower = id.to_ascii_lowercase();
    let normalized = id_lower.strip_prefix("openai/").unwrap_or(&id_lower);
    let profile = capability_profile(normalized);
    let prefers_responses = ProviderConfig::openai_prefers_responses_api(&id);

    let mut metadata = ModelMetadata::new(&id, &display_name(&id), ProviderId::OpenAI)
        .with_context(profile.context_window, profile.max_output);
    metadata.supports_tools = true;
    metadata.supports_vision = profile.supports_vision;
    metadata.api_format = if prefers_responses {
        ApiFormat::OpenAIResponses
    } else {
        ApiFormat::OpenAI
    };

    if !profile.reasoning_levels.is_empty() {
        metadata = metadata
            .with_thinking(ReasoningFormat::OpenAI)
            .with_reasoning_levels(
                profile.reasoning_levels.to_vec(),
                profile.default_reasoning,
                profile.reasoning_is_mandatory,
            )
            .with_reasoning_control(ReasoningControl::OpenAiEffort);
    }
    if let Some(fast_mode) = profile.fast_mode {
        metadata = metadata.with_fast_mode(fast_mode);
    }

    metadata
}

/// Convert an entitlement-specific ChatGPT Codex catalog entry. Hidden entries
/// are intentionally omitted; `supported_in_api` is not used because a model
/// such as Codex Spark may be valid through ChatGPT while unavailable to API-key
/// accounts.
pub(super) fn parse_chatgpt_model(raw: ChatGptModel) -> Option<ModelMetadata> {
    if !raw.visibility.eq_ignore_ascii_case("list") {
        return None;
    }

    let mut metadata = parse_model(OpenAiModel {
        id: raw.slug.clone(),
    });
    if !raw.display_name.trim().is_empty() {
        metadata.display_name = raw.display_name;
    }
    metadata.context_window = raw
        .context_window
        .or(raw.max_context_window)
        .unwrap_or(metadata.context_window);
    metadata.max_output = raw.max_output_tokens.unwrap_or(metadata.max_output);
    metadata.api_format = ApiFormat::OpenAIResponses;

    let mut levels = Vec::new();
    for preset in raw.supported_reasoning_levels {
        let Some(level) = parse_reasoning_effort(&preset.effort) else {
            continue;
        };
        if !levels.contains(&level) {
            levels.push(level);
        }
    }
    metadata.supports_thinking = !levels.is_empty();
    metadata.reasoning_format = (!levels.is_empty()).then_some(ReasoningFormat::OpenAI);
    metadata.reasoning_control = (!levels.is_empty()).then_some(ReasoningControl::OpenAiEffort);
    metadata.default_reasoning_level = raw
        .default_reasoning_level
        .as_deref()
        .and_then(parse_reasoning_effort)
        .filter(|default| levels.contains(default));
    metadata.reasoning_is_mandatory = !levels.is_empty()
        && raw
            .reasoning_is_mandatory
            .unwrap_or_else(|| !levels.contains(&ReasoningEffort::None));
    metadata.supported_reasoning_levels = levels;

    // The rich catalog is authoritative for service-tier eligibility. Standard
    // mode is represented by omitting a tier; only an advertised `priority`
    // tier enables Krusty's Fast control.
    metadata.fast_mode = raw
        .service_tiers
        .iter()
        .any(|tier| tier.id.eq_ignore_ascii_case("priority"))
        .then_some(FastMode::Priority);

    if !raw.input_modalities.is_empty() {
        metadata.supports_vision = raw
            .input_modalities
            .iter()
            .any(|modality| modality.eq_ignore_ascii_case("image"));
    }

    let catalog_describes_tools = raw.supports_tools.is_some()
        || raw.shell_type.is_some()
        || raw.supports_parallel_tool_calls
        || !raw.experimental_supported_tools.is_empty();
    if catalog_describes_tools {
        let has_shell_tool = raw.shell_type.as_deref().is_some_and(|shell_type| {
            !matches!(
                shell_type.to_ascii_lowercase().as_str(),
                "none" | "disabled"
            )
        });
        metadata.supports_tools = raw.supports_tools.unwrap_or(false)
            || raw.supports_parallel_tool_calls
            || has_shell_tool
            || !raw.experimental_supported_tools.is_empty();
    }

    Some(metadata)
}

fn capability_profile(model_id: &str) -> CapabilityProfile {
    match model_id {
        "gpt-5.6" | "gpt-5.6-sol" => CapabilityProfile {
            context_window: 1_050_000,
            max_output: 128_000,
            reasoning_levels: GPT_56_LEVELS,
            default_reasoning: Some(ReasoningEffort::Low),
            reasoning_is_mandatory: false,
            fast_mode: Some(FastMode::Priority),
            supports_vision: true,
        },
        "gpt-5.6-luna" | "gpt-5.6-terra" => CapabilityProfile {
            context_window: 1_050_000,
            max_output: 128_000,
            reasoning_levels: GPT_56_LEVELS,
            default_reasoning: Some(ReasoningEffort::Medium),
            reasoning_is_mandatory: false,
            fast_mode: Some(FastMode::Priority),
            supports_vision: true,
        },
        "gpt-5.5" => CapabilityProfile {
            context_window: 272_000,
            max_output: 128_000,
            reasoning_levels: GPT_55_LEVELS,
            default_reasoning: Some(ReasoningEffort::Medium),
            reasoning_is_mandatory: false,
            fast_mode: Some(FastMode::Priority),
            supports_vision: true,
        },
        "gpt-5.5-pro" => CapabilityProfile {
            context_window: 1_050_000,
            max_output: 128_000,
            reasoning_levels: GPT_55_PRO_LEVELS,
            default_reasoning: Some(ReasoningEffort::High),
            reasoning_is_mandatory: true,
            fast_mode: None,
            supports_vision: true,
        },
        "gpt-5.4" => CapabilityProfile {
            context_window: 1_050_000,
            max_output: 128_000,
            reasoning_levels: GPT_54_LEVELS,
            default_reasoning: Some(ReasoningEffort::Medium),
            reasoning_is_mandatory: false,
            fast_mode: Some(FastMode::Priority),
            supports_vision: true,
        },
        "gpt-5.4-mini" | "gpt-5.4-nano" => CapabilityProfile {
            context_window: 400_000,
            max_output: 128_000,
            reasoning_levels: GPT_54_LEVELS,
            default_reasoning: Some(ReasoningEffort::Medium),
            reasoning_is_mandatory: false,
            fast_mode: None,
            supports_vision: true,
        },
        "gpt-5.4-pro" => CapabilityProfile {
            context_window: 1_050_000,
            max_output: 128_000,
            reasoning_levels: GPT_PRO_LEVELS,
            default_reasoning: Some(ReasoningEffort::High),
            reasoning_is_mandatory: true,
            fast_mode: None,
            supports_vision: true,
        },
        "gpt-5.3-codex" => CapabilityProfile {
            context_window: 400_000,
            max_output: 128_000,
            reasoning_levels: GPT_CODEX_LEVELS,
            default_reasoning: Some(ReasoningEffort::Medium),
            reasoning_is_mandatory: true,
            fast_mode: None,
            supports_vision: true,
        },
        "gpt-5.3-codex-spark" => CapabilityProfile {
            context_window: 128_000,
            max_output: 32_000,
            reasoning_levels: GPT_CODEX_LEVELS,
            default_reasoning: Some(ReasoningEffort::High),
            reasoning_is_mandatory: true,
            fast_mode: None,
            supports_vision: true,
        },
        "gpt-5.3-chat-latest" => CapabilityProfile {
            context_window: 128_000,
            max_output: 16_384,
            reasoning_levels: &[],
            default_reasoning: None,
            reasoning_is_mandatory: false,
            fast_mode: None,
            supports_vision: true,
        },
        _ => generic_capability_profile(model_id),
    }
}

fn generic_capability_profile(model_id: &str) -> CapabilityProfile {
    if is_openai_o_series(model_id) {
        return CapabilityProfile {
            context_window: 200_000,
            max_output: 100_000,
            reasoning_levels: O_SERIES_LEVELS,
            default_reasoning: Some(ReasoningEffort::Medium),
            reasoning_is_mandatory: true,
            fast_mode: None,
            supports_vision: model_id != "o3-mini",
        };
    }

    if model_id.starts_with("codex-") || model_id.contains("codex") {
        return CapabilityProfile {
            context_window: 400_000,
            max_output: 128_000,
            reasoning_levels: GPT_GENERIC_LEVELS,
            default_reasoning: Some(ReasoningEffort::Medium),
            reasoning_is_mandatory: true,
            fast_mode: None,
            supports_vision: true,
        };
    }

    match gpt_major_version(model_id) {
        Some(major) if major >= 5 && !model_id.contains("chat-latest") => CapabilityProfile {
            context_window: 400_000,
            max_output: 128_000,
            reasoning_levels: GPT_GENERIC_LEVELS,
            default_reasoning: Some(ReasoningEffort::Medium),
            reasoning_is_mandatory: true,
            fast_mode: None,
            supports_vision: true,
        },
        Some(4) if model_id.starts_with("gpt-4.1") => CapabilityProfile {
            context_window: 1_047_576,
            max_output: 32_768,
            reasoning_levels: &[],
            default_reasoning: None,
            reasoning_is_mandatory: false,
            fast_mode: None,
            supports_vision: true,
        },
        _ => CapabilityProfile {
            context_window: 128_000,
            max_output: 16_384,
            reasoning_levels: &[],
            default_reasoning: None,
            reasoning_is_mandatory: false,
            fast_mode: None,
            supports_vision: model_id.contains("gpt-4o") || model_id.starts_with("chatgpt-"),
        },
    }
}

fn parse_reasoning_effort(effort: &str) -> Option<ReasoningEffort> {
    match effort.trim().to_ascii_lowercase().as_str() {
        "none" | "off" => Some(ReasoningEffort::None),
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" | "x-high" => Some(ReasoningEffort::XHigh),
        "max" => Some(ReasoningEffort::Max),
        "ultra" => Some(ReasoningEffort::Ultra),
        _ => None,
    }
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

fn contains_non_chat_capability(model_id: &str) -> bool {
    const EXCLUDED_MARKERS: &[&str] = &[
        "audio",
        "computer-use",
        "dall-e",
        "deep-research",
        "embedding",
        "image",
        "instruct",
        "moderation",
        "realtime",
        "search",
        "sora",
        "transcribe",
        "transcription",
        "tts",
        "whisper",
    ];
    EXCLUDED_MARKERS
        .iter()
        .any(|marker| model_id.contains(marker))
}

fn is_snapshot_id(model_id: &str) -> bool {
    let parts = model_id.split('-').collect::<Vec<_>>();
    if parts
        .iter()
        .any(|part| is_digits_of_len(part, 4) || is_digits_of_len(part, 8))
    {
        return true;
    }
    if parts.len() >= 3 {
        let tail = &parts[parts.len() - 3..];
        if is_digits_of_len(tail[0], 4)
            && is_digits_of_len(tail[1], 2)
            && is_digits_of_len(tail[2], 2)
        {
            return true;
        }
    }

    parts.last().is_some_and(|tail| {
        (tail.len() == 4 || tail.len() == 8) && tail.chars().all(|ch| ch.is_ascii_digit())
    })
}

fn is_digits_of_len(value: &str, len: usize) -> bool {
    value.len() == len && value.chars().all(|ch| ch.is_ascii_digit())
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
    use super::{display_name, is_useful_model, parse_chatgpt_model, parse_model};
    use crate::ai::models::ApiFormat;
    use crate::ai::openai::types::{ChatGptModelsResponse, OpenAiModel};
    use crate::ai::providers::{FastMode, ReasoningControl, ReasoningEffort};

    #[test]
    fn keeps_future_chat_and_reasoning_families_without_exact_allowlist() {
        for model in [
            "gpt-5.6-sol",
            "gpt-6.4",
            "o5",
            "chatgpt-4o-latest",
            "codex-mini-latest",
            "openai/gpt-5.5-mini",
        ] {
            assert!(is_useful_model(model), "expected {model} to be retained");
        }

        for model in [
            "gpt-5.4-2026-04-01",
            "gpt-4o-2024-11-20",
            "o1-2024-12-17",
            "gpt-4-0613",
            "gpt-4-1106-preview",
            "gpt-audio-2",
            "text-embedding-4-large",
            "gpt-image-2",
            "gpt-realtime-2.1",
            "whisper-1",
            "omni-moderation-latest",
        ] {
            assert!(!is_useful_model(model), "expected {model} to be filtered");
        }
    }

    #[test]
    fn builds_readable_display_name() {
        assert_eq!(display_name("gpt-5.3-codex"), "GPT 5.3 Codex");
        assert_eq!(display_name("gpt-5.6-sol"), "GPT 5.6 Sol");
        assert_eq!(display_name("gpt-5.4-mini"), "GPT 5.4 Mini");
    }

    #[test]
    fn enriches_current_api_models_with_reasoning_and_fast_capabilities() {
        let metadata = parse_model(OpenAiModel {
            id: "gpt-5.6-sol".to_string(),
        });

        assert_eq!(metadata.context_window, 1_050_000);
        assert_eq!(metadata.max_output, 128_000);
        assert_eq!(metadata.api_format, ApiFormat::OpenAIResponses);
        assert_eq!(metadata.fast_mode, Some(FastMode::Priority));
        assert_eq!(
            metadata.reasoning_control,
            Some(ReasoningControl::OpenAiEffort)
        );
        assert_eq!(
            metadata.supported_reasoning_levels,
            vec![
                ReasoningEffort::None,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
                ReasoningEffort::Max,
            ]
        );
        assert!(!metadata.reasoning_is_mandatory);
        assert!(metadata.supports_vision);
        assert!(metadata.supports_tools);
    }

    #[test]
    fn distinguishes_current_openai_model_capabilities() {
        let gpt_55 = parse_model(OpenAiModel {
            id: "gpt-5.5".to_string(),
        });
        assert_eq!(gpt_55.context_window, 272_000);
        assert_eq!(gpt_55.fast_mode, Some(FastMode::Priority));
        assert!(!gpt_55
            .supported_reasoning_levels
            .contains(&ReasoningEffort::Minimal));
        assert!(!gpt_55
            .supported_reasoning_levels
            .contains(&ReasoningEffort::Max));

        let gpt_54_mini = parse_model(OpenAiModel {
            id: "gpt-5.4-mini".to_string(),
        });
        assert_eq!(gpt_54_mini.context_window, 400_000);
        assert_eq!(gpt_54_mini.fast_mode, None);

        let gpt_54 = parse_model(OpenAiModel {
            id: "gpt-5.4".to_string(),
        });
        assert_eq!(gpt_54.context_window, 1_050_000);

        let spark = parse_model(OpenAiModel {
            id: "gpt-5.3-codex-spark".to_string(),
        });
        assert_eq!(spark.max_output, 32_000);
        assert_eq!(spark.default_reasoning_level, Some(ReasoningEffort::High));
        assert_eq!(
            spark.supported_reasoning_levels,
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
            ]
        );
        assert!(spark.reasoning_is_mandatory);
    }

    #[test]
    fn parses_chatgpt_fixture_as_entitlement_specific_capabilities() {
        let fixture = r#"
        {
          "models": [
            {
              "slug": "gpt-5.6-sol",
              "display_name": "GPT-5.6-Sol",
              "visibility": "list",
              "context_window": 272000,
              "max_context_window": 272000,
              "max_output_tokens": 128000,
              "default_reasoning_level": "low",
              "supported_reasoning_levels": [
                {"effort": "low", "description": "Fast"},
                {"effort": "medium", "description": "Balanced"},
                {"effort": "high", "description": "Deep"},
                {"effort": "xhigh", "description": "Deeper"},
                {"effort": "max", "description": "Maximum"},
                {"effort": "ultra", "description": "Delegates"}
              ],
              "service_tiers": [
                {"id": "priority", "name": "Fast", "description": "1.5x speed"}
              ],
              "input_modalities": ["text", "image"],
              "shell_type": "shell_command",
              "supports_parallel_tool_calls": true,
              "experimental_supported_tools": []
            },
            {
              "slug": "codex-auto-review",
              "display_name": "Auto review",
              "visibility": "hide",
              "supported_reasoning_levels": [],
              "service_tiers": [],
              "input_modalities": ["text"]
            }
          ]
        }
        "#;
        let response: ChatGptModelsResponse = serde_json::from_str(fixture).unwrap();
        let models = response
            .models
            .into_iter()
            .filter_map(parse_chatgpt_model)
            .collect::<Vec<_>>();

        assert_eq!(models.len(), 1);
        let model = &models[0];
        assert_eq!(model.id, "gpt-5.6-sol");
        assert_eq!(model.display_name, "GPT-5.6-Sol");
        assert_eq!(model.context_window, 272_000);
        assert_eq!(model.max_output, 128_000);
        assert_eq!(model.default_reasoning_level, Some(ReasoningEffort::Low));
        assert_eq!(
            model.supported_reasoning_levels,
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
                ReasoningEffort::Max,
                ReasoningEffort::Ultra,
            ]
        );
        assert!(model.reasoning_is_mandatory);
        assert_eq!(model.fast_mode, Some(FastMode::Priority));
        assert_eq!(model.api_format, ApiFormat::OpenAIResponses);
        assert!(model.supports_vision);
        assert!(model.supports_tools);
    }
}
