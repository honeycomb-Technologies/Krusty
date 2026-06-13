//! Token budgeting and compaction trigger thresholds.

use crate::ai::model_profile::ModelProfile;
use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::types::{Content, ModelMessage};
use crate::constants;

use super::{DEFAULT_KEEP_RECENT_TOKENS, DEFAULT_RESERVE_TOKENS};

const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

#[derive(Debug, Clone, Copy)]
pub struct CompactionManager {
    trigger_tokens: usize,
    target_tokens: usize,
    hard_failure_tokens: usize,
}

impl Default for CompactionManager {
    fn default() -> Self {
        Self::for_model(
            ProviderId::MiniMax,
            ApiFormat::Anthropic,
            constants::ai::DEFAULT_MODEL,
            constants::ai::CONTEXT_WINDOW_TOKENS,
        )
    }
}

impl CompactionManager {
    pub fn for_model(
        provider: ProviderId,
        api_format: ApiFormat,
        model_id: &str,
        context_window: usize,
    ) -> Self {
        let budgets = ModelProfile::resolve(provider, api_format, model_id)
            .compaction_budgets(context_window);

        Self {
            trigger_tokens: budgets.trigger_tokens,
            target_tokens: budgets.target_tokens,
            hard_failure_tokens: budgets.hard_failure_tokens,
        }
    }

    pub fn should_compact(&self, estimated_tokens: usize) -> bool {
        estimated_tokens >= self.trigger_tokens
    }

    pub(crate) fn keep_recent_tokens_for_attempt(
        &self,
        estimated_request_tokens: usize,
        raw_conversation_tokens: usize,
        attempt: u32,
    ) -> usize {
        let context_overhead = estimated_request_tokens.saturating_sub(raw_conversation_tokens);
        let target_history_budget = self
            .target_tokens
            .saturating_sub(context_overhead)
            .saturating_sub(DEFAULT_RESERVE_TOKENS);
        let raw_tail_cap = raw_conversation_tokens.saturating_sub(1).max(1);
        let default_tail = DEFAULT_KEEP_RECENT_TOKENS.min(raw_tail_cap).max(1);
        let base_tail = target_history_budget.max(default_tail).min(raw_tail_cap);
        let hard_pressure = estimated_request_tokens >= self.hard_failure_tokens;

        let requested = match (hard_pressure, attempt) {
            (true, 0) => default_tail,
            (true, 1) => default_tail.saturating_mul(3) / 4,
            (true, 2) => default_tail / 2,
            (true, _) => default_tail / 4,
            (false, 0) => base_tail,
            (false, 1) => base_tail.saturating_mul(3) / 4,
            (false, 2) => base_tail / 2,
            (false, _) => default_tail,
        };

        requested.max(1).min(raw_tail_cap)
    }
}

#[cfg(test)]
impl CompactionManager {
    fn with_budgets(
        trigger_tokens: usize,
        target_tokens: usize,
        hard_failure_tokens: usize,
    ) -> Self {
        Self {
            trigger_tokens,
            target_tokens,
            hard_failure_tokens,
        }
    }
}

pub(crate) fn estimate_tokens(messages: &[ModelMessage]) -> usize {
    let total_chars: usize = messages
        .iter()
        .flat_map(|message| &message.content)
        .map(content_char_len)
        .sum();

    total_chars / CHARS_PER_TOKEN_ESTIMATE
}

pub fn estimate_with_usage(
    messages: &[ModelMessage],
    last_usage_prompt_tokens: Option<usize>,
    messages_after_usage: usize,
) -> usize {
    if let Some(usage_tokens) = last_usage_prompt_tokens {
        let tail_start = messages.len().saturating_sub(messages_after_usage);
        let tail_estimate = estimate_tokens(&messages[tail_start..]);
        return usage_tokens.saturating_add(tail_estimate);
    }
    estimate_tokens(messages)
}

fn content_char_len(content: &Content) -> usize {
    match content {
        Content::Text { text } => text.len(),
        Content::ToolUse { name, input, .. } => name.len() + input.to_string().len(),
        Content::ToolResult { output, .. } => output.to_string().len(),
        Content::Image { .. } => 1_000,
        Content::Document { .. } => 5_000,
        Content::Thinking { thinking, .. } => thinking.len(),
        Content::RedactedThinking { .. } => 100,
    }
}

#[cfg(test)]
mod tests {
    use super::{estimate_tokens, CompactionManager};
    use crate::ai::types::{Content, DocumentSource, ModelMessage, Role};

    #[test]
    fn estimate_tokens_counts_mixed_content() {
        let messages = vec![
            ModelMessage {
                role: Role::User,
                content: vec![Content::Text {
                    text: "abcdabcd".to_string(),
                }],
            },
            ModelMessage {
                role: Role::Assistant,
                content: vec![Content::Document {
                    source: DocumentSource {
                        source_type: "base64".to_string(),
                        media_type: "application/pdf".to_string(),
                        data: Some("ignored".to_string()),
                        url: None,
                    },
                }],
            },
        ];

        assert_eq!(estimate_tokens(&messages), (8 + 5_000) / 4);
    }

    #[test]
    fn should_compact_at_trigger_threshold() {
        let manager = CompactionManager::with_budgets(1_000, 800, 1_200);

        assert!(!manager.should_compact(999));
        assert!(manager.should_compact(1_000));
        assert!(manager.should_compact(1_100));
    }
}
