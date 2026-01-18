//! OpenCode Zen API integration
//!
//! Fetches available models from OpenCode Zen's API.
//! Routes models to correct endpoints based on API format per official docs.
//!
//! Endpoint routing (from https://opencode.ai/docs/zen):
//! - `/v1/messages` (Anthropic): Claude models, MiniMax M2.1
//! - `/v1/responses` (OpenAI Responses): GPT 5.x models
//! - `/v1/models/{model}` (Google): Gemini models
//! - `/v1/chat/completions` (OpenAI): GLM, Kimi, Qwen, Grok, Big Pickle

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, error, info};

use super::models::{ApiFormat, ModelMetadata};
use super::providers::{ProviderId, ReasoningFormat};

const OPENCODEZEN_MODELS_URL: &str = "https://opencode.ai/zen/v1/models";

/// Response from OpenCode Zen models endpoint
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<OpenCodeZenModel>,
}

/// Single model from OpenCode Zen API
#[derive(Debug, Deserialize)]
struct OpenCodeZenModel {
    id: String,
    #[serde(default)]
    owned_by: String,
}

/// Fetch all available models from OpenCode Zen
pub async fn fetch_models(api_key: &str) -> Result<Vec<ModelMetadata>> {
    let client = Client::new();

    info!("Fetching models from OpenCode Zen...");

    let response = client
        .get(OPENCODEZEN_MODELS_URL)
        .header("x-api-key", api_key)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        error!("OpenCode Zen API error: {} - {}", status, error_text);
        return Err(anyhow::anyhow!(
            "OpenCode Zen API error: {} - {}",
            status,
            error_text
        ));
    }

    let data: ModelsResponse = response.json().await?;
    info!("OpenCode Zen returned {} models", data.data.len());

    // Convert to our format
    let models: Vec<ModelMetadata> = data.data.into_iter().map(parse_model).collect();

    debug!("Parsed {} OpenCode Zen models", models.len());
    Ok(models)
}

/// Determine API format for OpenCode Zen model
///
/// Based on OpenCode Zen official documentation:
/// - Claude models + MiniMax M2.1 → Anthropic format (/v1/messages)
/// - GPT-5 models → OpenAI Responses format (/v1/responses)
/// - Gemini models → Google format (/v1/models/{model})
/// - GLM, Kimi, Qwen, Grok, Big Pickle → OpenAI-compatible (/v1/chat/completions)
fn detect_api_format(model_id: &str) -> ApiFormat {
    let id = model_id.to_lowercase();

    // Anthropic format: Claude models AND MiniMax M2.1
    // From docs: minimax-m2.1-free uses /v1/messages with @ai-sdk/anthropic
    if id.starts_with("claude") || id.starts_with("minimax") {
        return ApiFormat::Anthropic;
    }

    // GPT-5 uses OpenAI Responses format (/v1/responses)
    if id.starts_with("gpt-5") {
        return ApiFormat::OpenAIResponses;
    }

    // Gemini uses Google format (/v1/models/{model})
    if id.starts_with("gemini") {
        return ApiFormat::Google;
    }

    // Everything else uses OpenAI chat/completions:
    // GLM, Kimi, Qwen, Grok, Big Pickle
    ApiFormat::OpenAI
}

/// Parse OpenCode Zen model into our format
fn parse_model(raw: OpenCodeZenModel) -> ModelMetadata {
    let id = &raw.id;

    // Determine capabilities and specs based on model ID
    // Values from OpenCode Zen docs and model specs
    let (context_window, max_output, supports_thinking, reasoning_format) = get_model_specs(id);

    // Generate display name from ID
    let display_name = generate_display_name(id);

    // Check if free model (from docs)
    let is_free = is_free_model(id);

    // Determine API format for routing
    let api_format = detect_api_format(id);

    ModelMetadata {
        id: raw.id,
        display_name,
        provider: ProviderId::OpenCodeZen,
        context_window,
        max_output,
        supports_thinking,
        reasoning_format,
        supports_tools: true, // All OpenCode Zen models support tools
        supports_vision: false,
        input_price: None,
        output_price: None,
        sub_provider: Some(raw.owned_by),
        is_free,
        api_format,
    }
}

/// Get model specifications based on model ID
/// Returns: (context_window, max_output, supports_thinking, reasoning_format)
fn get_model_specs(id: &str) -> (usize, usize, bool, Option<ReasoningFormat>) {
    match id {
        // Claude Opus models - thinking enabled with Anthropic format
        "claude-opus-4-5" => (200_000, 16_384, true, Some(ReasoningFormat::Anthropic)),
        "claude-opus-4-1" => (200_000, 16_384, true, Some(ReasoningFormat::Anthropic)),

        // Claude Sonnet models
        "claude-sonnet-4-5" => (200_000, 16_384, true, Some(ReasoningFormat::Anthropic)),
        "claude-sonnet-4" => (200_000, 16_384, false, None),

        // Claude Haiku models
        "claude-haiku-4-5" => (200_000, 16_384, false, None),
        "claude-3-5-haiku" => (200_000, 16_384, false, None),

        // GPT-5 models - no reasoning support via Zen currently
        "gpt-5.2" => (200_000, 32_768, false, None),
        "gpt-5.1" | "gpt-5.1-codex" | "gpt-5.1-codex-max" => (200_000, 32_768, false, None),
        "gpt-5.1-codex-mini" => (200_000, 16_384, false, None),
        "gpt-5" | "gpt-5-codex" => (200_000, 32_768, false, None),
        "gpt-5-nano" => (128_000, 16_384, false, None),

        // Gemini models
        "gemini-3-pro" => (1_000_000, 65_536, false, None),
        "gemini-3-flash" => (1_000_000, 65_536, false, None),

        // MiniMax - uses Anthropic format, supports interleaved thinking
        "minimax-m2.1-free" => (200_000, 64_000, true, Some(ReasoningFormat::Anthropic)),

        // GLM models - use chat_template_args (handled separately via transform)
        "glm-4.6" => (128_000, 16_384, false, None),
        "glm-4.7-free" => (128_000, 16_384, false, None),

        // Kimi models
        "kimi-k2" => (256_000, 16_384, false, None),
        "kimi-k2-thinking" => (256_000, 16_384, true, Some(ReasoningFormat::Anthropic)),

        // Qwen
        "qwen3-coder" => (128_000, 32_768, false, None),

        // Grok
        "grok-code" => (128_000, 16_384, false, None),

        // Big Pickle (stealth model)
        "big-pickle" => (128_000, 16_384, false, None),

        // Fallback for unknown models
        _ => {
            // Try to infer from prefix
            let id_lower = id.to_lowercase();
            if id_lower.starts_with("claude-opus") || id_lower.starts_with("claude-sonnet-4-5") {
                (200_000, 16_384, true, Some(ReasoningFormat::Anthropic))
            } else if id_lower.starts_with("claude") {
                (200_000, 16_384, false, None)
            } else if id_lower.starts_with("gpt-5") {
                (200_000, 32_768, false, None)
            } else if id_lower.starts_with("gemini") {
                (1_000_000, 65_536, false, None)
            } else if id_lower.starts_with("minimax") {
                (200_000, 64_000, true, Some(ReasoningFormat::Anthropic))
            } else if id_lower.starts_with("kimi") && id_lower.contains("thinking") {
                (256_000, 16_384, true, Some(ReasoningFormat::Anthropic))
            } else if id_lower.starts_with("kimi") {
                (256_000, 16_384, false, None)
            } else if id_lower.starts_with("glm") {
                (128_000, 16_384, false, None)
            } else if id_lower.starts_with("qwen") {
                (128_000, 32_768, false, None)
            } else {
                (128_000, 16_384, false, None)
            }
        }
    }
}

/// Check if model is free (from OpenCode Zen docs)
fn is_free_model(id: &str) -> bool {
    matches!(
        id,
        "big-pickle" | "grok-code" | "minimax-m2.1-free" | "glm-4.7-free" | "gpt-5-nano"
    ) || id.ends_with("-free")
}

/// Generate a human-readable display name from model ID
fn generate_display_name(id: &str) -> String {
    // Special cases with proper formatting
    match id {
        "big-pickle" => return "Big Pickle (Free)".to_string(),
        "minimax-m2.1-free" => return "MiniMax M2.1 (Free)".to_string(),
        "glm-4.7-free" => return "GLM 4.7 (Free)".to_string(),
        "glm-4.6" => return "GLM 4.6".to_string(),
        "gpt-5-nano" => return "GPT-5 Nano (Free)".to_string(),
        "gpt-5.2" => return "GPT-5.2".to_string(),
        "gpt-5.1" => return "GPT-5.1".to_string(),
        "gpt-5.1-codex" => return "GPT-5.1 Codex".to_string(),
        "gpt-5.1-codex-max" => return "GPT-5.1 Codex Max".to_string(),
        "gpt-5.1-codex-mini" => return "GPT-5.1 Codex Mini".to_string(),
        "gpt-5" => return "GPT-5".to_string(),
        "gpt-5-codex" => return "GPT-5 Codex".to_string(),
        "grok-code" => return "Grok Code (Free)".to_string(),
        "kimi-k2" => return "Kimi K2".to_string(),
        "kimi-k2-thinking" => return "Kimi K2 Thinking".to_string(),
        "qwen3-coder" => return "Qwen3 Coder 480B".to_string(),
        "gemini-3-pro" => return "Gemini 3 Pro".to_string(),
        "gemini-3-flash" => return "Gemini 3 Flash".to_string(),
        "claude-opus-4-5" => return "Claude Opus 4.5".to_string(),
        "claude-opus-4-1" => return "Claude Opus 4.1".to_string(),
        "claude-sonnet-4-5" => return "Claude Sonnet 4.5".to_string(),
        "claude-sonnet-4" => return "Claude Sonnet 4".to_string(),
        "claude-haiku-4-5" => return "Claude Haiku 4.5".to_string(),
        "claude-3-5-haiku" => return "Claude 3.5 Haiku".to_string(),
        _ => {}
    }

    // Fallback: Convert kebab-case to Title Case
    let words: Vec<String> = id
        .split('-')
        .map(|word| {
            // Handle version numbers and special cases
            if word.chars().all(|c| c.is_ascii_digit() || c == '.') {
                word.to_string()
            } else if word.eq_ignore_ascii_case("gpt") {
                "GPT".to_string()
            } else if word.eq_ignore_ascii_case("glm") {
                "GLM".to_string()
            } else {
                // Capitalize first letter
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                    None => String::new(),
                }
            }
        })
        .collect();

    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_api_format() {
        // Anthropic format
        assert!(matches!(
            detect_api_format("claude-opus-4-5"),
            ApiFormat::Anthropic
        ));
        assert!(matches!(
            detect_api_format("claude-sonnet-4"),
            ApiFormat::Anthropic
        ));
        assert!(matches!(
            detect_api_format("minimax-m2.1-free"),
            ApiFormat::Anthropic
        ));

        // OpenAI Responses format
        assert!(matches!(
            detect_api_format("gpt-5.1-codex"),
            ApiFormat::OpenAIResponses
        ));
        assert!(matches!(
            detect_api_format("gpt-5-nano"),
            ApiFormat::OpenAIResponses
        ));

        // Google format
        assert!(matches!(
            detect_api_format("gemini-3-pro"),
            ApiFormat::Google
        ));

        // OpenAI chat format
        assert!(matches!(detect_api_format("glm-4.6"), ApiFormat::OpenAI));
        assert!(matches!(detect_api_format("kimi-k2"), ApiFormat::OpenAI));
        assert!(matches!(detect_api_format("grok-code"), ApiFormat::OpenAI));
        assert!(matches!(detect_api_format("big-pickle"), ApiFormat::OpenAI));
    }

    #[test]
    fn test_is_free_model() {
        assert!(is_free_model("big-pickle"));
        assert!(is_free_model("grok-code"));
        assert!(is_free_model("minimax-m2.1-free"));
        assert!(is_free_model("glm-4.7-free"));
        assert!(is_free_model("gpt-5-nano"));

        assert!(!is_free_model("claude-opus-4-5"));
        assert!(!is_free_model("glm-4.6"));
    }

    #[test]
    fn test_generate_display_name() {
        assert_eq!(generate_display_name("claude-opus-4-5"), "Claude Opus 4.5");
        assert_eq!(generate_display_name("gpt-5.1-codex"), "GPT-5.1 Codex");
        assert_eq!(generate_display_name("glm-4.6"), "GLM 4.6");
        assert_eq!(generate_display_name("big-pickle"), "Big Pickle (Free)");
        assert_eq!(
            generate_display_name("minimax-m2.1-free"),
            "MiniMax M2.1 (Free)"
        );
    }
}
