//! OpenRouter API integration
//!
//! Fetches available models from OpenRouter's API.

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, error, info};

use super::models::ModelMetadata;
use super::providers::{ProviderId, ReasoningFormat};

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// Response from OpenRouter models endpoint
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<OpenRouterModel>,
}

/// Single model from OpenRouter API
#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    name: String,
    #[serde(default)]
    context_length: Option<usize>,
    #[serde(default)]
    top_provider: Option<TopProvider>,
    #[serde(default)]
    pricing: Option<Pricing>,
    #[serde(default)]
    supported_parameters: Vec<String>,
    #[serde(default)]
    architecture: Option<Architecture>,
}

#[derive(Debug, Deserialize)]
struct TopProvider {
    max_completion_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct Pricing {
    prompt: Option<String>,
    completion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Architecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

/// Fetch all available models from OpenRouter
pub async fn fetch_models(api_key: &str) -> Result<Vec<ModelMetadata>> {
    let client = Client::new();

    info!("Fetching models from OpenRouter...");

    let response = client
        .get(OPENROUTER_MODELS_URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("HTTP-Referer", "https://github.com/anthropics/claude-code")
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        error!("OpenRouter API error: {} - {}", status, error_text);
        return Err(anyhow::anyhow!(
            "OpenRouter API error: {} - {}",
            status,
            error_text
        ));
    }

    let data: ModelsResponse = response.json().await?;
    info!("OpenRouter returned {} models", data.data.len());

    // Convert to our format and filter to useful models
    let models: Vec<ModelMetadata> = data
        .data
        .into_iter()
        .filter(|m| is_useful_model(&m.id))
        .map(parse_model)
        .filter(|m| m.supports_tools) // Only include models that support tool calling
        .collect();

    debug!("Filtered to {} models with tool support", models.len());
    Ok(models)
}

/// Check if a model is worth showing (filter out obscure/test models)
fn is_useful_model(id: &str) -> bool {
    // Include models from major providers
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

    // Exclude test/deprecated models (but keep :free and instruct - we want those!)
    let bad_patterns = [
        ":beta",
        "-preview",
        "-experimental",
        "-base", // Base models without fine-tuning
    ];

    let id_lower = id.to_lowercase();

    // Must start with a good prefix
    if !good_prefixes.iter().any(|p| id_lower.starts_with(p)) {
        return false;
    }

    // Must not contain bad patterns
    !bad_patterns.iter().any(|p| id_lower.contains(p))
}

/// Parse OpenRouter model into our format
fn parse_model(raw: OpenRouterModel) -> ModelMetadata {
    let context_window = raw.context_length.unwrap_or(128_000);
    let max_output = raw
        .top_provider
        .and_then(|t| t.max_completion_tokens)
        .unwrap_or(4096);

    // Detect reasoning capabilities and determine format based on model ID
    let supports_thinking = raw
        .supported_parameters
        .iter()
        .any(|p| p == "reasoning" || p == "include_reasoning" || p == "reasoning_effort");

    // Determine reasoning format based on model provider/type
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

    // Parse pricing (convert from per-token to per-million)
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

    // Clean up display name (remove "Provider: " prefix if present)
    let display_name = raw.name.split(": ").last().unwrap_or(&raw.name).to_string();

    // Extract sub-provider from model ID (e.g., "anthropic/claude-3" -> "anthropic")
    let sub_provider = raw.id.split('/').next().map(|s| s.to_string());

    // Check if this is a free model (ends with :free)
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
        api_format: super::models::ApiFormat::Anthropic, // OpenRouter uses Anthropic skin
    }
}

/// Determine the correct reasoning format based on model ID
fn determine_reasoning_format(model_id: &str) -> ReasoningFormat {
    let id_lower = model_id.to_lowercase();

    // Anthropic Claude models use Anthropic format
    if id_lower.starts_with("anthropic/") {
        return ReasoningFormat::Anthropic;
    }

    // OpenAI o-series and GPT-5 models use OpenAI format
    if id_lower.starts_with("openai/")
        && (id_lower.contains("o1")
            || id_lower.contains("o3")
            || id_lower.contains("o4")
            || id_lower.contains("gpt-5"))
    {
        return ReasoningFormat::OpenAI;
    }

    // DeepSeek R1 and reasoning models use DeepSeek format
    if id_lower.starts_with("deepseek/")
        && (id_lower.contains("-r1") || id_lower.contains("reasoner"))
    {
        return ReasoningFormat::DeepSeek;
    }

    // Default to Anthropic format for other models with reasoning support
    ReasoningFormat::Anthropic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_useful_model() {
        // Should include
        assert!(is_useful_model("anthropic/claude-3-opus"));
        assert!(is_useful_model("openai/gpt-4o"));
        assert!(is_useful_model("google/gemini-2.0-flash"));
        assert!(is_useful_model("meta-llama/llama-4-scout"));
        assert!(is_useful_model("deepseek/deepseek-chat-v3"));
        assert!(is_useful_model("anthropic/claude-3-opus:free")); // Free models included
        assert!(is_useful_model("meta-llama/llama-3.2-3b-instruct:free")); // Instruct + free
        assert!(is_useful_model("mistralai/mistral-7b-instruct")); // Instruct models included

        // Should exclude
        assert!(!is_useful_model("openai/gpt-4-preview"));
        assert!(!is_useful_model("some-random/model"));
        assert!(!is_useful_model("meta-llama/llama-2-7b-base")); // Base models excluded
    }
}
