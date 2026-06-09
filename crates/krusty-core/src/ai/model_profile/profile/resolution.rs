use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;

use super::{ModelProfile, PromptFamily};

impl ModelProfile {
    pub fn resolve(provider: ProviderId, api_format: ApiFormat, model_id: &str) -> Self {
        let normalized = normalize_model_id(model_id);

        if normalized.contains("claude") || normalized.starts_with("anthropic/") {
            return Self {
                prompt_family: PromptFamily::AnthropicClaude,
                usable_context_ratio: 0.74,
                auto_compact_threshold_ratio: 0.82,
                compaction_target_ratio: 0.64,
                hard_failure_ratio: 0.90,
                stream_idle_timeout_secs: 900,
                supports_reasoning_summary: false,
                prefer_parallel_tool_calls: true,
            };
        }

        if matches!(api_format, ApiFormat::Google)
            || normalized.contains("gemini")
            || normalized.starts_with("google/")
        {
            return Self {
                prompt_family: PromptFamily::GoogleGemini,
                usable_context_ratio: 0.78,
                auto_compact_threshold_ratio: 0.85,
                compaction_target_ratio: 0.68,
                hard_failure_ratio: 0.93,
                stream_idle_timeout_secs: 900,
                supports_reasoning_summary: false,
                prefer_parallel_tool_calls: true,
            };
        }

        if normalized.contains("codex") {
            return Self {
                prompt_family: PromptFamily::OpenAiCodex,
                usable_context_ratio: 0.7,
                auto_compact_threshold_ratio: 0.78,
                compaction_target_ratio: 0.60,
                hard_failure_ratio: 0.88,
                stream_idle_timeout_secs: 1_200,
                supports_reasoning_summary: true,
                prefer_parallel_tool_calls: true,
            };
        }

        if matches!(api_format, ApiFormat::OpenAIResponses)
            || uses_openai_responses_family(&normalized)
        {
            return Self {
                prompt_family: PromptFamily::OpenAiReasoning,
                usable_context_ratio: 0.72,
                auto_compact_threshold_ratio: 0.8,
                compaction_target_ratio: 0.62,
                hard_failure_ratio: 0.90,
                stream_idle_timeout_secs: 1_200,
                supports_reasoning_summary: true,
                prefer_parallel_tool_calls: true,
            };
        }

        if matches!(provider, ProviderId::OpenAI | ProviderId::Grok)
            || normalized.starts_with("openai/")
            || normalized.contains("grok")
        {
            return Self {
                prompt_family: PromptFamily::OpenAiReasoning,
                usable_context_ratio: 0.75,
                auto_compact_threshold_ratio: 0.82,
                compaction_target_ratio: 0.64,
                hard_failure_ratio: 0.90,
                stream_idle_timeout_secs: 900,
                supports_reasoning_summary: false,
                prefer_parallel_tool_calls: true,
            };
        }

        Self {
            prompt_family: PromptFamily::GenericCoding,
            usable_context_ratio: 0.75,
            auto_compact_threshold_ratio: 0.84,
            compaction_target_ratio: 0.64,
            hard_failure_ratio: 0.92,
            stream_idle_timeout_secs: 900,
            supports_reasoning_summary: false,
            prefer_parallel_tool_calls: true,
        }
    }
}

fn normalize_model_id(model_id: &str) -> String {
    model_id.trim().to_ascii_lowercase()
}

fn uses_openai_responses_family(model_id: &str) -> bool {
    let normalized = model_id.strip_prefix("openai/").unwrap_or(model_id);

    normalized.contains("codex")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
        || gpt_major_version(normalized).is_some_and(|major| major >= 5)
}

fn gpt_major_version(model_id: &str) -> Option<u32> {
    let suffix = model_id.strip_prefix("gpt-")?;
    let digits = suffix
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|segment| !segment.is_empty())?;
    digits.parse().ok()
}
