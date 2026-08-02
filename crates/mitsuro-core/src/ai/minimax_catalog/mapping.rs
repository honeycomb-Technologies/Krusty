use crate::ai::models::{ApiFormat, ModelMetadata};
use crate::ai::providers::{
    FastMode, ProviderId, ReasoningControl, ReasoningEffort, ReasoningFormat,
};

use super::types::MiniMaxModel;

const M3_REASONING_LEVELS: &[ReasoningEffort] = &[ReasoningEffort::None, ReasoningEffort::High];

struct CuratedCapabilities {
    display_name: &'static str,
    context_window: usize,
    max_output: usize,
    reasoning_levels: &'static [ReasoningEffort],
    default_reasoning_level: Option<ReasoningEffort>,
    reasoning_is_mandatory: bool,
    reasoning_control: ReasoningControl,
    supports_vision: bool,
    fast_mode: Option<FastMode>,
}

pub(super) fn parse_model(raw: MiniMaxModel) -> Option<ModelMetadata> {
    let id = raw.id.trim().to_string();
    if id.is_empty() {
        return None;
    }

    let curated = curated_capabilities(&id);
    let api_display_name = non_empty(raw.display_name).filter(|name| name != &id);
    let display_name = api_display_name
        .or_else(|| curated.as_ref().map(|item| item.display_name.to_string()))
        .unwrap_or_else(|| id.clone());

    let mut metadata = ModelMetadata::new(&id, &display_name, ProviderId::MiniMax);
    metadata.api_format = ApiFormat::Anthropic;
    metadata.supports_tools = true;
    metadata.supports_vision = false;

    if let Some(curated) = curated {
        metadata.context_window = curated.context_window;
        metadata.max_output = curated.max_output;
        metadata.supports_thinking = true;
        metadata.reasoning_format = Some(ReasoningFormat::Anthropic);
        metadata.supported_reasoning_levels = curated.reasoning_levels.to_vec();
        metadata.default_reasoning_level = curated.default_reasoning_level;
        metadata.reasoning_is_mandatory = curated.reasoning_is_mandatory;
        metadata.reasoning_control = Some(curated.reasoning_control);
        metadata.supports_vision = curated.supports_vision;
        metadata.fast_mode = curated.fast_mode;
    }

    Some(metadata)
}

fn curated_capabilities(id: &str) -> Option<CuratedCapabilities> {
    match id.to_ascii_lowercase().as_str() {
        "minimax-m3" => Some(CuratedCapabilities {
            display_name: "MiniMax M3",
            context_window: 1_000_000,
            max_output: 131_072,
            reasoning_levels: M3_REASONING_LEVELS,
            default_reasoning_level: Some(ReasoningEffort::High),
            reasoning_is_mandatory: false,
            reasoning_control: ReasoningControl::AnthropicAdaptive,
            supports_vision: true,
            fast_mode: Some(FastMode::Priority),
        }),
        "minimax-m2.7" => Some(older_model("MiniMax M2.7")),
        "minimax-m2.7-highspeed" => Some(older_model("MiniMax M2.7 Highspeed")),
        "minimax-m2.5" => Some(older_model("MiniMax M2.5")),
        "minimax-m2.5-highspeed" => Some(older_model("MiniMax M2.5 Highspeed")),
        _ => None,
    }
}

fn older_model(display_name: &'static str) -> CuratedCapabilities {
    CuratedCapabilities {
        display_name,
        context_window: 204_800,
        max_output: 131_072,
        reasoning_levels: &[ReasoningEffort::High],
        default_reasoning_level: Some(ReasoningEffort::High),
        reasoning_is_mandatory: true,
        reasoning_control: ReasoningControl::Boolean,
        supports_vision: false,
        fast_mode: None,
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::parse_model;
    use crate::ai::minimax_catalog::types::ModelsResponse;
    use crate::ai::providers::{FastMode, ReasoningControl, ReasoningEffort};

    fn fixture_models() -> Vec<crate::ai::models::ModelMetadata> {
        serde_json::from_str::<ModelsResponse>(include_str!("fixtures/models.json"))
            .expect("valid MiniMax fixture")
            .data
            .into_iter()
            .filter_map(parse_model)
            .collect()
    }

    #[test]
    fn overlays_m3_adaptive_reasoning_capabilities() {
        let models = fixture_models();
        let m3 = models
            .iter()
            .find(|model| model.id == "MiniMax-M3")
            .expect("M3 fixture model");

        assert_eq!(m3.display_name, "MiniMax M3");
        assert_eq!(m3.context_window, 1_000_000);
        assert_eq!(m3.max_output, 131_072);
        assert_eq!(
            m3.supported_reasoning_levels,
            vec![ReasoningEffort::None, ReasoningEffort::High]
        );
        assert_eq!(m3.default_reasoning_level, Some(ReasoningEffort::High));
        assert_eq!(
            m3.reasoning_control,
            Some(ReasoningControl::AnthropicAdaptive)
        );
        assert!(m3.supports_vision);
        assert_eq!(m3.fast_mode, Some(FastMode::Priority));
    }

    #[test]
    fn keeps_highspeed_as_a_distinct_model_not_fast_mode() {
        let models = fixture_models();
        let highspeed = models
            .iter()
            .find(|model| model.id == "MiniMax-M2.7-highspeed")
            .expect("highspeed fixture model");

        assert_eq!(highspeed.id, "MiniMax-M2.7-highspeed");
        assert_eq!(highspeed.context_window, 204_800);
        assert_eq!(highspeed.max_output, 131_072);
        assert!(highspeed.fast_mode.is_none());
        assert_eq!(
            highspeed.supported_reasoning_levels,
            vec![ReasoningEffort::High]
        );
        assert!(highspeed.reasoning_is_mandatory);
        assert_eq!(highspeed.reasoning_control, Some(ReasoningControl::Boolean));
    }

    #[test]
    fn overlays_m2_5_and_leaves_unknown_models_conservative() {
        let models = fixture_models();
        let m2_5 = models
            .iter()
            .find(|model| model.id == "MiniMax-M2.5")
            .expect("M2.5 fixture model");
        let unknown = models
            .iter()
            .find(|model| model.id == "MiniMax-Future")
            .expect("unknown fixture model");

        assert!(m2_5.supports_thinking);
        assert_eq!(m2_5.context_window, 204_800);
        assert!(!unknown.supports_thinking);
        assert_eq!(unknown.context_window, 128_000);
        assert_eq!(unknown.max_output, 4096);
    }

    #[test]
    fn ignores_catalog_rows_without_an_id() {
        assert_eq!(fixture_models().len(), 4);
    }
}
