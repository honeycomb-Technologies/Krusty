//! OpenAI-compatible model handler
//!
//! Provides simple ON/OFF reasoning toggle for OpenAI-compatible thinking models:
//! - GLM models (GLM-4.x via Z.ai)
//! - MiniMax M2 (MiniMax)
//!
//! Based on OpenCode's handling - no effort levels, just maxed thinking or off.
use serde::{Deserialize, Serialize};

/// Simple reasoning mode toggle for OpenAI-compatible thinking models
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningMode {
    Off,
    On,
}

impl From<bool> for ReasoningMode {
    fn from(enabled: bool) -> Self {
        if enabled {
            ReasoningMode::On
        } else {
            ReasoningMode::Off
        }
    }
}

/// Chat template arguments for GLM thinking models
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTemplateArgs {
    pub enable_thinking: bool,
}

/// Check if a model ID is an OpenAI-compatible thinking model
///
/// Note: GLM models via Z.ai's Anthropic endpoint use standard Anthropic format.
/// This function is for OpenAI-format handling (e.g., via OpenCode Zen).
pub fn is_openai_compatible_model(model_id: &str) -> bool {
    let id_lower = model_id.to_lowercase();
    id_lower.contains("glm") || id_lower.contains("minimax-m2")
}

/// Check if a model uses reasoning_content field (OpenAI format only)
///
/// Note: MiniMax via their Anthropic API uses standard thinking blocks.
/// This function applies to OpenAI-format responses (e.g., via OpenRouter).
pub fn uses_reasoning_content(model_id: &str) -> bool {
    let id_lower = model_id.to_lowercase();
    id_lower.contains("deepseek") || id_lower.contains("minimax-m2")
}

/// Check if a model uses chat_template_args (GLM)
pub fn uses_chat_template_args(model_id: &str) -> bool {
    let id_lower = model_id.to_lowercase();
    id_lower.contains("glm")
}

/// Get default temperature for OpenAI-compatible thinking models
///
/// These models typically work better with higher temperatures, especially in reasoning mode
pub fn get_default_temperature(model_id: &str) -> Option<f32> {
    let id_lower = model_id.to_lowercase();

    if !is_openai_compatible_model(model_id) {
        return None;
    }

    // GLM-4.6, GLM-4.7: prefer 1.0
    if id_lower.contains("glm-4.6") || id_lower.contains("glm-4.7") {
        return Some(1.0);
    }

    // GLM-4.5: 1.0
    if id_lower.contains("glm-4.5") {
        return Some(1.0);
    }

    // MiniMax M2: 1.0 (from OpenCode)
    if id_lower.contains("minimax-m2") {
        return Some(1.0);
    }

    // Default for other OpenAI-compatible models
    Some(0.8)
}

/// Get chat template args for OpenAI-compatible thinking models.
///
/// Returns `Some(value)` when reasoning is ON for GLM models:
/// - GLM models: `{ "enableThinking": true }`
///
/// Returns `None` when reasoning is OFF or not applicable.
pub fn get_chat_template_args(
    model_id: &str,
    reasoning_mode: ReasoningMode,
) -> Option<serde_json::Value> {
    let id_lower = model_id.to_lowercase();

    // Only apply to models that use chat_template_args
    let is_chat_template_model = id_lower.contains("glm");

    if !is_chat_template_model {
        return None;
    }

    // Enable thinking only when mode is On
    if reasoning_mode == ReasoningMode::On {
        let args = ChatTemplateArgs {
            enable_thinking: true,
        };
        serde_json::to_value(args).ok()
    } else {
        None
    }
}

/// Get provider options for OpenAI-compatible models with reasoning
///
/// Returns provider-specific JSON based on model type and reasoning mode:
/// - MiniMax/DeepSeek: { "reasoningContent": "..." } in openaiCompatible
/// - GLM: Handled via chat_template_args (separate field)
pub fn get_provider_options(
    model_id: &str,
    reasoning_text: Option<String>,
) -> Option<serde_json::Value> {
    // MiniMax M2 and DeepSeek use reasoning_content field
    if uses_reasoning_content(model_id) {
        if let Some(text) = reasoning_text {
            return Some(serde_json::json!({
                "openaiCompatible": {
                    "reasoning_content": text
                }
            }));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_openai_compatible_model() {
        assert!(is_openai_compatible_model("glm-4.6"));
        assert!(is_openai_compatible_model("GLM-4.7"));
        assert!(is_openai_compatible_model("minimax-m2.1"));
        assert!(is_openai_compatible_model("zai/glm-4.6"));
        assert!(!is_openai_compatible_model("claude-3.5"));
        assert!(!is_openai_compatible_model("gpt-4"));
    }

    #[test]
    fn test_uses_reasoning_content() {
        assert!(uses_reasoning_content("deepseek-r1"));
        assert!(uses_reasoning_content("minimax-m2.1"));
        assert!(uses_reasoning_content("minimax-m2"));
        assert!(!uses_reasoning_content("glm-4.6"));
    }

    #[test]
    fn test_uses_chat_template_args() {
        assert!(uses_chat_template_args("glm-4.6"));
        assert!(uses_chat_template_args("zai/glm-4.5"));
        assert!(!uses_chat_template_args("minimax-m2"));
        assert!(!uses_chat_template_args("claude-3.5"));
    }

    #[test]
    fn test_get_default_temperature() {
        assert_eq!(get_default_temperature("glm-4.6"), Some(1.0));
        assert_eq!(get_default_temperature("glm-4.7"), Some(1.0));
        assert_eq!(get_default_temperature("glm-4.5"), Some(1.0));
        assert_eq!(get_default_temperature("minimax-m2.1"), Some(1.0));
        assert_eq!(get_default_temperature("gpt-4"), None);
    }

    #[test]
    fn test_get_chat_template_args() {
        // GLM-4.6 with reasoning ON
        let args = get_chat_template_args("glm-4.6", ReasoningMode::On);
        assert!(args.is_some());
        let binding = args.unwrap();
        let obj = binding.as_object().unwrap();
        assert_eq!(obj.get("enableThinking").unwrap().as_bool(), Some(true));

        // GLM-4.6 with reasoning OFF
        let args = get_chat_template_args("glm-4.6", ReasoningMode::Off);
        assert!(args.is_none());

        // MiniMax M2 (doesn't use chat_template_args)
        let args = get_chat_template_args("minimax-m2.1", ReasoningMode::On);
        assert!(args.is_none());
    }

    #[test]
    fn test_get_provider_options() {
        // MiniMax M2 with reasoning text
        let opts = get_provider_options("minimax-m2.1", Some("thinking goes here".to_string()));
        assert!(opts.is_some());
        let binding = opts.unwrap();
        let obj = binding.as_object().unwrap();
        let openai = obj.get("openaiCompatible").unwrap().as_object().unwrap();
        assert_eq!(
            openai.get("reasoning_content").unwrap().as_str(),
            Some("thinking goes here")
        );

        // GLM (doesn't use reasoning_content)
        let opts = get_provider_options("glm-4.6", Some("thinking".to_string()));
        assert!(opts.is_none());
    }

    #[test]
    fn test_reasoning_mode_from_bool() {
        assert_eq!(ReasoningMode::from(true), ReasoningMode::On);
        assert_eq!(ReasoningMode::from(false), ReasoningMode::Off);
    }
}
