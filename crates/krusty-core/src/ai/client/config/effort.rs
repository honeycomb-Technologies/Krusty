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
}
