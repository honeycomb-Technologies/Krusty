/// Anthropic adaptive effort for Opus 4.6 thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicAdaptiveEffort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl AnthropicAdaptiveEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
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
    Max,
}

impl CodexReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn normalized_for_model(self, model_id: &str) -> Self {
        match self {
            Self::Max if !supports_openai_max_reasoning(model_id) => {
                if supports_openai_xhigh_reasoning(model_id) {
                    Self::XHigh
                } else {
                    Self::High
                }
            }
            Self::XHigh if !supports_openai_xhigh_reasoning(model_id) => Self::High,
            _ => self,
        }
    }
}

pub fn supports_openai_max_reasoning(model_id: &str) -> bool {
    let lower = model_id.trim().to_ascii_lowercase();
    let normalized = lower.strip_prefix("openai/").unwrap_or(&lower);
    normalized.starts_with("gpt-5.6")
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
        || normalized.contains("grok-4.6")
        || normalized.contains("grok-4-6")
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
    fn grok_46_supports_xhigh_and_grok_45_does_not() {
        assert!(supports_openai_xhigh_reasoning("grok-4.6"));
        assert!(supports_openai_xhigh_reasoning("grok-4-6"));
        assert!(supports_openai_xhigh_reasoning("xai/grok-4.6"));
        assert!(!supports_openai_xhigh_reasoning("grok-4.5"));
        assert_eq!(
            CodexReasoningEffort::XHigh.normalized_for_model("grok-4.6"),
            CodexReasoningEffort::XHigh
        );
        assert_eq!(
            CodexReasoningEffort::XHigh.normalized_for_model("grok-4.5"),
            CodexReasoningEffort::High
        );
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
