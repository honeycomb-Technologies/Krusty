use crate::ai::models::{ApiFormat, ModelMetadata};
use crate::ai::providers::{ProviderId, ReasoningFormat};

use super::types::OpenRouterModel;

/// Check if a model is worth showing (filter out obscure/test models)
pub(super) fn is_useful_model(id: &str) -> bool {
    let good_prefixes = [
        "anthropic/",
        "openai/",
        "google/",
        "meta-llama/",
        "mistralai/",
        "qwen/",
        "deepseek/",
        "cohere/",
        "x-ai/",
        "nvidia/",
        "perplexity/",
        "databricks/",
    ];

    let bad_patterns = [":beta", "-preview", "-experimental", "-base"];

    let id_lower = id.to_lowercase();

    if !good_prefixes.iter().any(|p| id_lower.starts_with(p)) {
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

    let supports_thinking = raw
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

    ModelMetadata {
        id: raw.id,
        display_name,
        provider: ProviderId::OpenRouter,
        context_window,
        max_output,
        supports_thinking,
        reasoning_format,
        supports_tools,
        supports_vision,
        input_price,
        output_price,
        sub_provider,
        is_free,
        api_format: ApiFormat::Anthropic,
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
    use super::is_useful_model;

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

        assert!(!is_useful_model("openai/gpt-4-preview"));
        assert!(!is_useful_model("some-random/model"));
        assert!(!is_useful_model("meta-llama/llama-2-7b-base"));
    }
}
