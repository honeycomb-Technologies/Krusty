//! Centralized reasoning/thinking configuration
//!
//! Handles provider-specific reasoning parameters in one place.

use serde_json::{json, Value};

use crate::ai::providers::ReasoningFormat;

pub const DEFAULT_THINKING_BUDGET: u32 = 32000;

/// Centralized reasoning configuration builder
pub struct ReasoningConfig;

impl ReasoningConfig {
    /// Build provider-specific reasoning parameters based on format and settings
    ///
    /// Returns the JSON value to merge into the request body for the given format.
    /// For Anthropic, this is `thinking: {...}`. For OpenAI, `reasoning_effort: "high"`.
    /// For DeepSeek, `reasoning: { enabled: true }`.
    pub fn build(
        format: Option<ReasoningFormat>,
        enabled: bool,
        budget_tokens: Option<u32>,
        effort: Option<&str>,
    ) -> Option<Value> {
        if !enabled {
            return None;
        }

        match format {
            Some(ReasoningFormat::Anthropic) => {
                // budget_tokens + optional effort for Opus
                Some(json!({
                    "type": "enabled",
                    "budget_tokens": budget_tokens.unwrap_or(DEFAULT_THINKING_BUDGET)
                }))
            }
            Some(ReasoningFormat::OpenAI) => {
                // reasoning_effort: "low" | "medium" | "high"
                Some(json!({
                    "reasoning_effort": effort.unwrap_or("high")
                }))
            }
            Some(ReasoningFormat::DeepSeek) => {
                // reasoning.enabled: true
                Some(json!({
                    "reasoning": { "enabled": true }
                }))
            }
            None => None,
        }
    }

    /// Build Opus 4.5 effort config (output_config.effort)
    pub fn build_opus_effort(model_id: &str, enabled: bool) -> Option<Value> {
        if enabled && model_id.contains("opus-4-5") {
            Some(json!({
                "effort": "high"
            }))
        } else {
            None
        }
    }

    /// Get the appropriate max_tokens for a reasoning format
    ///
    /// Anthropic thinking requires max_tokens > budget_tokens, so we use 64k.
    /// Other formats don't reduce output quota.
    pub fn max_tokens_for_format(
        format: Option<ReasoningFormat>,
        fallback: u32,
        legacy_thinking_enabled: bool,
    ) -> u32 {
        match format {
            Some(ReasoningFormat::Anthropic) => 64000,
            Some(ReasoningFormat::OpenAI | ReasoningFormat::DeepSeek) => fallback,
            None => {
                if legacy_thinking_enabled {
                    64000
                } else {
                    fallback
                }
            }
        }
    }

    /// Check if a model supports reasoning and warn if toggle is on but unsupported
    pub fn validate(model_id: &str, format: Option<ReasoningFormat>, enabled: bool) {
        if enabled && format.is_none() {
            tracing::warn!(
                model = model_id,
                "Thinking requested but model does not support reasoning"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_anthropic() {
        let result = ReasoningConfig::build(Some(ReasoningFormat::Anthropic), true, None, None);
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["type"], "enabled");
        assert_eq!(val["budget_tokens"], DEFAULT_THINKING_BUDGET);
    }

    #[test]
    fn test_build_anthropic_custom_budget() {
        let result =
            ReasoningConfig::build(Some(ReasoningFormat::Anthropic), true, Some(10000), None);
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["budget_tokens"], 10000);
    }

    #[test]
    fn test_build_openai() {
        let result = ReasoningConfig::build(Some(ReasoningFormat::OpenAI), true, None, None);
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["reasoning_effort"], "high");
    }

    #[test]
    fn test_build_openai_custom_effort() {
        let result =
            ReasoningConfig::build(Some(ReasoningFormat::OpenAI), true, None, Some("medium"));
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["reasoning_effort"], "medium");
    }

    #[test]
    fn test_build_deepseek() {
        let result = ReasoningConfig::build(Some(ReasoningFormat::DeepSeek), true, None, None);
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(val["reasoning"]["enabled"], true);
    }

    #[test]
    fn test_build_disabled() {
        let result = ReasoningConfig::build(Some(ReasoningFormat::Anthropic), false, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_build_no_format() {
        let result = ReasoningConfig::build(None, true, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_opus_effort() {
        let result = ReasoningConfig::build_opus_effort("claude-opus-4-5-20251101", true);
        assert!(result.is_some());
        assert_eq!(result.unwrap()["effort"], "high");

        let result = ReasoningConfig::build_opus_effort("claude-sonnet-4", true);
        assert!(result.is_none());

        let result = ReasoningConfig::build_opus_effort("claude-opus-4-5-20251101", false);
        assert!(result.is_none());
    }

    #[test]
    fn test_max_tokens_anthropic() {
        let tokens =
            ReasoningConfig::max_tokens_for_format(Some(ReasoningFormat::Anthropic), 16384, false);
        assert_eq!(tokens, 64000);
    }

    #[test]
    fn test_max_tokens_openai() {
        let tokens =
            ReasoningConfig::max_tokens_for_format(Some(ReasoningFormat::OpenAI), 16384, false);
        assert_eq!(tokens, 16384);
    }

    #[test]
    fn test_max_tokens_legacy() {
        let tokens = ReasoningConfig::max_tokens_for_format(None, 16384, true);
        assert_eq!(tokens, 64000);

        let tokens = ReasoningConfig::max_tokens_for_format(None, 16384, false);
        assert_eq!(tokens, 16384);
    }
}
