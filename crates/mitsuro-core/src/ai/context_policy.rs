//! Context and compaction policy resolved independently of prompt style.

use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionBudgets {
    pub trigger_tokens: usize,
    pub target_tokens: usize,
    pub hard_failure_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextBudgetPolicy {
    trigger_ratio: f32,
    target_ratio: f32,
    hard_failure_ratio: f32,
}

impl ContextBudgetPolicy {
    /// Resolve from the wire API contract. The exact context-window size comes
    /// from `ResolvedModelRuntime`; model-name prompt heuristics never alter it.
    pub fn resolve(_provider: ProviderId, api_format: ApiFormat) -> Self {
        match api_format {
            ApiFormat::OpenAIResponses => Self {
                trigger_ratio: 0.80,
                target_ratio: 0.62,
                hard_failure_ratio: 0.90,
            },
            ApiFormat::Anthropic => Self {
                trigger_ratio: 0.82,
                target_ratio: 0.64,
                hard_failure_ratio: 0.90,
            },
            ApiFormat::Google => Self {
                trigger_ratio: 0.85,
                target_ratio: 0.68,
                hard_failure_ratio: 0.93,
            },
            ApiFormat::OpenAI => Self {
                trigger_ratio: 0.84,
                target_ratio: 0.64,
                hard_failure_ratio: 0.92,
            },
        }
    }

    pub fn compaction_budgets(self, context_window: usize) -> CompactionBudgets {
        let context_window = context_window.max(1);
        let trigger_tokens = ratio_to_tokens(context_window, self.trigger_ratio);
        let target_tokens = ratio_to_tokens(context_window, self.target_ratio)
            .min(trigger_tokens.saturating_sub(1).max(1));
        let hard_failure_tokens = ratio_to_tokens(context_window, self.hard_failure_ratio)
            .max(trigger_tokens.saturating_add(1));

        CompactionBudgets {
            trigger_tokens,
            target_tokens,
            hard_failure_tokens,
        }
    }
}

fn ratio_to_tokens(context_window: usize, ratio: f32) -> usize {
    ((context_window as f64 * ratio as f64).round() as usize)
        .max(1)
        .min(context_window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_are_monotonic_and_transport_scoped() {
        let responses =
            ContextBudgetPolicy::resolve(ProviderId::OpenAI, ApiFormat::OpenAIResponses)
                .compaction_budgets(500_000);
        let grok = ContextBudgetPolicy::resolve(ProviderId::Grok, ApiFormat::OpenAIResponses)
            .compaction_budgets(500_000);

        assert_eq!(responses, grok);
        assert!(responses.target_tokens < responses.trigger_tokens);
        assert!(responses.trigger_tokens < responses.hard_failure_tokens);
    }
}
