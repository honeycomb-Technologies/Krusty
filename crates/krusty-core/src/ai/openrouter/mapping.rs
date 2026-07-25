use crate::ai::models::{ApiFormat, ModelCatalogSource, ModelMetadata};
use crate::ai::providers::{
    FastMode, ProviderId, ReasoningControl, ReasoningEffort, ReasoningFormat,
};

use super::types::{OpenRouterModel, SupportedEfforts};

/// Check if a model is worth showing (filter out obscure/test models)
pub(super) fn is_useful_model(id: &str) -> bool {
    let bad_patterns = [":beta", "-experimental", "-base"];

    let id_lower = id.trim().to_ascii_lowercase();
    if id_lower.is_empty() || !id_lower.contains('/') {
        return false;
    }

    !bad_patterns.iter().any(|p| id_lower.contains(p))
}

/// Parse OpenRouter model into our format
pub(super) fn parse_model(raw: OpenRouterModel) -> ModelMetadata {
    let context_window = raw.context_length.unwrap_or(128_000);
    let max_output = raw
        .top_provider
        .and_then(|t| t.max_completion_tokens)
        .unwrap_or(4096);

    let supports_thinking = raw.reasoning.is_some()
        || raw
            .supported_parameters
            .iter()
            .any(|p| p == "reasoning" || p == "include_reasoning" || p == "reasoning_effort");

    let reasoning_format = if supports_thinking {
        Some(determine_reasoning_format(&raw.id))
    } else {
        None
    };

    let supports_tools = raw
        .supported_parameters
        .iter()
        .any(|p| p == "tools" || p == "tool_choice");

    let supports_vision = raw
        .architecture
        .map(|a| a.input_modalities.iter().any(|m| m == "image"))
        .unwrap_or(false);

    let input_price = raw
        .pricing
        .as_ref()
        .and_then(|p| p.prompt.as_ref())
        .and_then(|s| s.parse::<f64>().ok())
        .map(|p| p * 1_000_000.0);

    let output_price = raw
        .pricing
        .as_ref()
        .and_then(|p| p.completion.as_ref())
        .and_then(|s| s.parse::<f64>().ok())
        .map(|p| p * 1_000_000.0);

    let display_name = raw.name.split(": ").last().unwrap_or(&raw.name).to_string();
    let sub_provider = raw.id.split('/').next().map(|s| s.to_string());
    let is_free = raw.id.ends_with(":free");

    let (
        mut supported_reasoning_levels,
        default_reasoning_level,
        reasoning_is_mandatory,
        mut reasoning_control,
    ) = raw
        .reasoning
        .as_ref()
        .map(|reasoning| {
            let (mut levels, control) = match &reasoning.supported_efforts {
                SupportedEfforts::Listed(efforts) => {
                    let parsed = efforts
                        .iter()
                        .filter_map(|effort| parse_reasoning_effort(effort))
                        .collect::<Vec<_>>();
                    if parsed.is_empty() {
                        (vec![ReasoningEffort::High], ReasoningControl::Boolean)
                    } else {
                        (parsed, ReasoningControl::OpenAiEffort)
                    }
                }
                SupportedEfforts::All => (
                    vec![
                        ReasoningEffort::Minimal,
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::XHigh,
                        ReasoningEffort::Max,
                    ],
                    ReasoningControl::OpenAiEffort,
                ),
                SupportedEfforts::Missing => (
                    vec![ReasoningEffort::High],
                    if reasoning.supports_max_tokens {
                        ReasoningControl::AnthropicBudget
                    } else {
                        ReasoningControl::Boolean
                    },
                ),
            };
            levels.sort();
            levels.dedup();
            if !reasoning.mandatory && !levels.contains(&ReasoningEffort::None) {
                levels.insert(0, ReasoningEffort::None);
            }
            let default = if reasoning.default_enabled == Some(false) {
                levels
                    .contains(&ReasoningEffort::None)
                    .then_some(ReasoningEffort::None)
            } else {
                reasoning
                    .default_effort
                    .as_deref()
                    .and_then(parse_reasoning_effort)
                    .or_else(|| {
                        reasoning
                            .mandatory
                            .then(|| {
                                levels
                                    .iter()
                                    .copied()
                                    .find(|level| *level != ReasoningEffort::None)
                            })
                            .flatten()
                    })
            };
            (levels, default, reasoning.mandatory, Some(control))
        })
        .unwrap_or_default();
    if supports_thinking && reasoning_control.is_none() {
        supported_reasoning_levels = vec![ReasoningEffort::None, ReasoningEffort::High];
        reasoning_control = Some(ReasoningControl::Boolean);
    }

    // OpenRouter accepts provider-level priority routing with standard fallback;
    // it is not advertised per model in `supported_parameters`.
    let fast_mode = Some(FastMode::Priority);

    ModelMetadata {
        id: raw.id,
        display_name,
        provider: ProviderId::OpenRouter,
        auth_scope: None,
        context_window,
        max_output,
        supports_thinking,
        reasoning_format,
        supported_reasoning_levels,
        default_reasoning_level,
        reasoning_is_mandatory,
        // OpenRouter's Messages surface owns the request shape. The streaming
        // adapter maps this metadata to `thinking` + `output_config.effort`.
        reasoning_control,
        fast_mode,
        supports_tools,
        supports_vision,
        input_price,
        output_price,
        sub_provider,
        is_free,
        api_format: ApiFormat::Anthropic,
        catalog_source: ModelCatalogSource::Legacy,
        catalog_revision: None,
    }
}

fn parse_reasoning_effort(raw: &str) -> Option<ReasoningEffort> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "off" => Some(ReasoningEffort::None),
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" | "x-high" => Some(ReasoningEffort::XHigh),
        "max" => Some(ReasoningEffort::Max),
        _ => None,
    }
}

/// Determine the correct reasoning format based on model ID
fn determine_reasoning_format(model_id: &str) -> ReasoningFormat {
    let id_lower = model_id.to_lowercase();

    if id_lower.starts_with("anthropic/") {
        return ReasoningFormat::Anthropic;
    }

    if id_lower.starts_with("openai/")
        && (id_lower.contains("o1")
            || id_lower.contains("o3")
            || id_lower.contains("o4")
            || id_lower.contains("gpt-5"))
    {
        return ReasoningFormat::OpenAI;
    }

    if id_lower.starts_with("deepseek/")
        && (id_lower.contains("-r1") || id_lower.contains("reasoner"))
    {
        return ReasoningFormat::DeepSeek;
    }

    ReasoningFormat::Anthropic
}

#[cfg(test)]
mod tests {
    use super::{is_useful_model, parse_model};
    use crate::ai::openrouter::types::OpenRouterModel;
    use crate::ai::providers::{FastMode, ReasoningControl, ReasoningEffort};

    #[test]
    fn test_is_useful_model() {
        assert!(is_useful_model("anthropic/claude-3-opus"));
        assert!(is_useful_model("openai/gpt-4o"));
        assert!(is_useful_model("google/gemini-2.0-flash"));
        assert!(is_useful_model("meta-llama/llama-4-scout"));
        assert!(is_useful_model("deepseek/deepseek-chat-v3"));
        assert!(is_useful_model("anthropic/claude-3-opus:free"));
        assert!(is_useful_model("meta-llama/llama-3.2-3b-instruct:free"));
        assert!(is_useful_model("mistralai/mistral-7b-instruct"));
        assert!(is_useful_model("moonshotai/kimi-k2"));
        assert!(is_useful_model("minimax/minimax-m3"));
        assert!(is_useful_model("z-ai/glm-5.2"));

        // Preview is a valid lifecycle marker on OpenRouter; the live catalog
        // should decide availability rather than a blanket suffix filter.
        assert!(is_useful_model("openai/gpt-4-preview"));
        assert!(is_useful_model("some-random/model"));
        assert!(!is_useful_model("missing-vendor-prefix"));
        assert!(!is_useful_model("meta-llama/llama-2-7b-base"));
    }

    #[test]
    fn nullable_efforts_use_full_levels_and_priority_routing() {
        let raw: OpenRouterModel = serde_json::from_value(serde_json::json!({
            "id": "moonshotai/kimi-k2",
            "name": "Moonshot: Kimi K2",
            "reasoning": {
                "supported_efforts": null,
                "default_enabled": false
            }
        }))
        .expect("nullable effort schema");

        let model = parse_model(raw);
        assert_eq!(
            model.reasoning_control,
            Some(ReasoningControl::OpenAiEffort)
        );
        assert!(model
            .supported_reasoning_levels
            .contains(&ReasoningEffort::Max));
        assert_eq!(model.default_reasoning_level, Some(ReasoningEffort::None));
        assert_eq!(model.fast_mode, Some(FastMode::Priority));
    }

    #[test]
    fn missing_efforts_fall_back_without_dropping_reasoning() {
        let raw: OpenRouterModel = serde_json::from_value(serde_json::json!({
            "id": "future/model",
            "name": "Future Model",
            "reasoning": {"supports_max_tokens": true}
        }))
        .expect("partial reasoning schema");

        let model = parse_model(raw);
        assert!(model.supports_thinking);
        assert_eq!(
            model.reasoning_control,
            Some(ReasoningControl::AnthropicBudget)
        );
        assert_eq!(
            model.supported_reasoning_levels,
            vec![ReasoningEffort::None, ReasoningEffort::High]
        );
    }

    #[test]
    fn catalog_efforts_are_presented_in_canonical_order() {
        let raw: OpenRouterModel = serde_json::from_value(serde_json::json!({
            "id": "anthropic/claude-opus-4.8",
            "name": "Anthropic: Claude Opus 4.8",
            "reasoning": {
                "supported_efforts": ["max", "xhigh", "high", "medium", "low"],
                "default_effort": "medium"
            }
        }))
        .expect("descending effort schema");

        let model = parse_model(raw);
        assert_eq!(
            model.supported_reasoning_levels,
            vec![
                ReasoningEffort::None,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
                ReasoningEffort::Max,
            ]
        );
    }
}
