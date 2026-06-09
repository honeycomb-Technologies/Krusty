/// Anthropic adaptive effort for Opus 4.6 thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicAdaptiveEffort {
    Low,
    Medium,
    High,
    Max,
}

impl AnthropicAdaptiveEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

/// Codex reasoning effort controls for OpenAI Responses API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

impl CodexReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }

    pub fn normalized_for_model(self, model_id: &str) -> Self {
        if matches!(self, Self::XHigh) && !supports_openai_xhigh_reasoning(model_id) {
            Self::High
        } else {
            self
        }
    }
}

pub fn supports_openai_xhigh_reasoning(model_id: &str) -> bool {
    let raw = model_id.trim();
    let lower = raw.to_ascii_lowercase();
    let normalized = lower
        .strip_prefix("openai/")
        .or_else(|| lower.strip_prefix("grok/"))
        .unwrap_or(&lower);

    normalized.contains("codex")
        || normalized == "grok-build"
        || normalized.starts_with("grok-composer-")
        || supports_gpt5_xhigh_reasoning(normalized)
}

fn supports_gpt5_xhigh_reasoning(model_id: &str) -> bool {
    let Some(version) = model_id.strip_prefix("gpt-5.") else {
        return false;
    };
    let Some(minor) = version
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|segment| !segment.is_empty())
        .and_then(|segment| segment.parse::<u32>().ok())
    else {
        return false;
    };

    minor >= 4
}

#[cfg(test)]
mod tests {
    use super::{supports_openai_xhigh_reasoning, CodexReasoningEffort};

    #[test]
    fn gpt_5_4_family_supports_xhigh_reasoning() {
        assert!(supports_openai_xhigh_reasoning("gpt-5.4"));
        assert!(supports_openai_xhigh_reasoning("gpt-5.4-mini"));
        assert!(supports_openai_xhigh_reasoning("openai/gpt-5.4-mini"));
        assert!(supports_openai_xhigh_reasoning("gpt-5.3-codex"));
    }

    #[test]
    fn gpt_5_5_family_supports_xhigh_reasoning() {
        assert!(supports_openai_xhigh_reasoning("gpt-5.5"));
        assert!(supports_openai_xhigh_reasoning("gpt-5.5-mini"));
        assert!(supports_openai_xhigh_reasoning("openai/gpt-5.5"));
        assert!(supports_openai_xhigh_reasoning("openai/gpt-5.5-mini"));
    }

    #[test]
    fn older_openai_models_do_not_assume_xhigh_reasoning() {
        assert!(!supports_openai_xhigh_reasoning("gpt-5"));
        assert!(!supports_openai_xhigh_reasoning("gpt-5.2"));
        assert!(!supports_openai_xhigh_reasoning("gpt-4.1"));
    }

    #[test]
    fn grok_build_models_support_xhigh_reasoning() {
        assert!(supports_openai_xhigh_reasoning("grok-build"));
        assert!(supports_openai_xhigh_reasoning("grok-composer-2.5-fast"));
    }

    #[test]
    fn xhigh_effort_clamps_to_high_when_model_does_not_support_it() {
        assert_eq!(
            CodexReasoningEffort::XHigh.normalized_for_model("gpt-5.2"),
            CodexReasoningEffort::High
        );
        assert_eq!(
            CodexReasoningEffort::XHigh.normalized_for_model("gpt-5.4"),
            CodexReasoningEffort::XHigh
        );
        assert_eq!(
            CodexReasoningEffort::XHigh.normalized_for_model("gpt-5.5"),
            CodexReasoningEffort::XHigh
        );
    }
}
