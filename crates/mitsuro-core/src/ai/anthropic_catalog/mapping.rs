use crate::ai::models::{ApiFormat, ModelMetadata};
use crate::ai::providers::{
    FastMode, ProviderId, ReasoningControl, ReasoningEffort, ReasoningFormat,
};

use super::types::{AnthropicModel, Capabilities, EffortCapability, SupportFlag};

const ADAPTIVE_LEVELS: &[ReasoningEffort] = &[
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::XHigh,
    ReasoningEffort::Max,
];

struct CuratedCapabilities {
    context_window: usize,
    max_output: usize,
    supports_thinking: bool,
    reasoning_levels: &'static [ReasoningEffort],
    default_reasoning_level: Option<ReasoningEffort>,
    reasoning_is_mandatory: bool,
    reasoning_control: Option<ReasoningControl>,
    supports_vision: bool,
}

#[derive(Default)]
struct CatalogReasoning {
    explicit: bool,
    supported: bool,
    levels: Vec<ReasoningEffort>,
    default: Option<ReasoningEffort>,
    control: Option<ReasoningControl>,
}

pub(super) fn parse_model(raw: AnthropicModel) -> Option<ModelMetadata> {
    let id = raw.id.trim().to_string();
    if id.is_empty() {
        return None;
    }

    let curated = curated_capabilities(&id);
    let context_window = positive(raw.max_input_tokens)
        .or_else(|| curated.as_ref().map(|item| item.context_window))
        .unwrap_or(200_000);
    let max_output = positive(raw.max_tokens)
        .or_else(|| curated.as_ref().map(|item| item.max_output))
        .unwrap_or(64_000);
    let display_name = non_empty(raw.display_name).unwrap_or_else(|| humanize_model_id(&id));

    let advertised = catalog_reasoning(raw.capabilities.as_ref());
    let (supports_thinking, reasoning_levels, default_reasoning_level, reasoning_control) =
        if advertised.explicit {
            (
                advertised.supported,
                advertised.levels,
                advertised.default,
                advertised.control,
            )
        } else if let Some(curated) = curated.as_ref() {
            (
                curated.supports_thinking,
                curated.reasoning_levels.to_vec(),
                curated.default_reasoning_level,
                curated.reasoning_control,
            )
        } else {
            (false, Vec::new(), None, None)
        };

    let supports_vision = raw
        .capabilities
        .as_ref()
        .and_then(|capabilities| capabilities.image_input.as_ref())
        .and_then(|image| image.supported)
        .or_else(|| curated.as_ref().map(|item| item.supports_vision))
        .unwrap_or(false);
    let reasoning_is_mandatory = supports_thinking
        && curated
            .as_ref()
            .is_some_and(|item| item.reasoning_is_mandatory);

    let mut metadata = ModelMetadata::new(&id, &display_name, ProviderId::Anthropic)
        .with_context(context_window, max_output);
    metadata.api_format = ApiFormat::Anthropic;
    metadata.supports_tools = true;
    metadata.supports_vision = supports_vision;
    metadata.supports_thinking = supports_thinking;
    metadata.reasoning_format = supports_thinking.then_some(ReasoningFormat::Anthropic);
    metadata.supported_reasoning_levels = reasoning_levels;
    metadata.default_reasoning_level = default_reasoning_level;
    metadata.reasoning_is_mandatory = reasoning_is_mandatory;
    metadata.reasoning_control = reasoning_control;

    if is_opus_4_8(&id) {
        metadata.fast_mode = Some(FastMode::AnthropicFast);
    }

    Some(metadata)
}

fn catalog_reasoning(capabilities: Option<&Capabilities>) -> CatalogReasoning {
    let Some(capabilities) = capabilities else {
        return CatalogReasoning::default();
    };

    let effort = capabilities.effort.as_ref();
    let thinking = capabilities.thinking.as_ref();
    let explicit = effort.is_some_and(EffortCapability::has_explicit_value)
        || thinking.is_some_and(|value| value.has_explicit_value());
    if !explicit {
        return CatalogReasoning::default();
    }

    let mut levels = effort.map(supported_efforts).unwrap_or_default();
    let effort_supported = effort.is_some_and(|value| match value.supported {
        Some(supported) => supported,
        None => !levels.is_empty(),
    });
    if !effort_supported {
        levels.clear();
    }

    let thinking_supported = thinking.and_then(|value| value.supported).unwrap_or(false);
    let supported = thinking_supported || effort_supported;
    if !supported {
        return CatalogReasoning {
            explicit: true,
            ..CatalogReasoning::default()
        };
    }

    let adaptive = thinking
        .and_then(|value| value.types.as_ref())
        .and_then(|types| types.adaptive.as_ref())
        .is_some_and(SupportFlag::is_supported);
    let control = if adaptive || effort_supported {
        Some(ReasoningControl::AnthropicAdaptive)
    } else {
        Some(ReasoningControl::AnthropicBudget)
    };
    let default = default_effort(&levels);

    CatalogReasoning {
        explicit: true,
        supported: true,
        levels,
        default,
        control,
    }
}

fn supported_efforts(effort: &EffortCapability) -> Vec<ReasoningEffort> {
    [
        (ReasoningEffort::Low, effort.low.as_ref()),
        (ReasoningEffort::Medium, effort.medium.as_ref()),
        (ReasoningEffort::High, effort.high.as_ref()),
        (ReasoningEffort::XHigh, effort.xhigh.as_ref()),
        (ReasoningEffort::Max, effort.max.as_ref()),
    ]
    .into_iter()
    .filter_map(|(level, support)| {
        support
            .is_some_and(SupportFlag::is_supported)
            .then_some(level)
    })
    .collect()
}

fn default_effort(levels: &[ReasoningEffort]) -> Option<ReasoningEffort> {
    [
        ReasoningEffort::High,
        ReasoningEffort::Medium,
        ReasoningEffort::Low,
        ReasoningEffort::XHigh,
        ReasoningEffort::Max,
    ]
    .into_iter()
    .find(|candidate| levels.contains(candidate))
}

fn curated_capabilities(id: &str) -> Option<CuratedCapabilities> {
    let normalized = id.to_ascii_lowercase().replace('.', "-");
    if normalized.starts_with("claude-fable-5") {
        return Some(adaptive_capabilities(true));
    }
    if normalized.starts_with("claude-opus-4-8") || normalized.starts_with("claude-sonnet-5") {
        return Some(adaptive_capabilities(false));
    }
    if normalized.starts_with("claude-haiku-4-5") {
        return Some(CuratedCapabilities {
            context_window: 200_000,
            max_output: 64_000,
            supports_thinking: true,
            reasoning_levels: &[],
            default_reasoning_level: None,
            reasoning_is_mandatory: false,
            reasoning_control: Some(ReasoningControl::AnthropicBudget),
            supports_vision: true,
        });
    }
    None
}

fn adaptive_capabilities(reasoning_is_mandatory: bool) -> CuratedCapabilities {
    CuratedCapabilities {
        context_window: 1_000_000,
        max_output: 128_000,
        supports_thinking: true,
        reasoning_levels: ADAPTIVE_LEVELS,
        default_reasoning_level: Some(ReasoningEffort::High),
        reasoning_is_mandatory,
        reasoning_control: Some(ReasoningControl::AnthropicAdaptive),
        supports_vision: true,
    }
}

fn is_opus_4_8(id: &str) -> bool {
    id.to_ascii_lowercase()
        .replace('.', "-")
        .starts_with("claude-opus-4-8")
}

fn positive(value: Option<usize>) -> Option<usize> {
    value.filter(|value| *value > 0)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn humanize_model_id(id: &str) -> String {
    id.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::parse_model;
    use crate::ai::providers::{FastMode, ReasoningControl, ReasoningEffort};

    use crate::ai::anthropic_catalog::types::ModelsResponse;

    fn fixture_models() -> Vec<crate::ai::models::ModelMetadata> {
        serde_json::from_str::<ModelsResponse>(include_str!("fixtures/models.json"))
            .expect("valid Anthropic fixture")
            .data
            .into_iter()
            .filter_map(parse_model)
            .collect()
    }

    #[test]
    fn parses_rich_capabilities_and_fast_mode() {
        let models = fixture_models();
        let opus = models
            .iter()
            .find(|model| model.id == "claude-opus-4-8")
            .expect("Opus fixture model");

        assert_eq!(opus.context_window, 1_000_000);
        assert_eq!(opus.max_output, 128_000);
        assert!(opus.supports_vision);
        assert_eq!(
            opus.supported_reasoning_levels,
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
            ]
        );
        assert_eq!(opus.default_reasoning_level, Some(ReasoningEffort::High));
        assert_eq!(
            opus.reasoning_control,
            Some(ReasoningControl::AnthropicAdaptive)
        );
        assert_eq!(opus.fast_mode, Some(FastMode::AnthropicFast));
    }

    #[test]
    fn uses_curated_overlay_when_catalog_capabilities_are_absent() {
        let models = fixture_models();
        let fable = models
            .iter()
            .find(|model| model.id == "claude-fable-5")
            .expect("Fable fixture model");

        assert_eq!(fable.context_window, 1_000_000);
        assert_eq!(fable.max_output, 128_000);
        assert_eq!(fable.default_reasoning_level, Some(ReasoningEffort::High));
        assert!(fable.reasoning_is_mandatory);
        assert_eq!(
            fable.reasoning_control,
            Some(ReasoningControl::AnthropicAdaptive)
        );
    }

    #[test]
    fn preserves_budget_thinking_without_inventing_effort_levels() {
        let models = fixture_models();
        let haiku = models
            .iter()
            .find(|model| model.id == "claude-haiku-4-5-20251001")
            .expect("Haiku fixture model");

        assert!(haiku.supports_thinking);
        assert!(haiku.supported_reasoning_levels.is_empty());
        assert_eq!(haiku.default_reasoning_level, None);
        assert_eq!(
            haiku.reasoning_control,
            Some(ReasoningControl::AnthropicBudget)
        );
        assert_eq!(haiku.context_window, 200_000);
        assert_eq!(haiku.max_output, 64_000);
    }

    #[test]
    fn ignores_catalog_rows_without_an_id() {
        assert_eq!(fixture_models().len(), 3);
    }
}
