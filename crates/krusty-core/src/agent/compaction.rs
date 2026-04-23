//! Context-pressure policy for long-running agent loops.
//!
//! Krusty now responds to context pressure by pinching into a linked
//! continuation session instead of rewriting the active conversation in place.
//! This module therefore only owns the model-specific trigger budget and a
//! lightweight token estimate used to detect when a pinch should happen.

use crate::ai::model_profile::ModelProfile;
use crate::ai::models::ApiFormat;
use crate::ai::providers::ProviderId;
use crate::ai::types::{Content, ModelMessage};
use crate::constants;

const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

#[derive(Debug, Clone)]
pub(crate) struct CompactionManager {
    trigger_tokens: usize,
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
    pub(crate) fn for_model(
        provider: ProviderId,
        api_format: ApiFormat,
        model_id: &str,
        context_window: usize,
    ) -> Self {
        let budgets = ModelProfile::resolve(provider, api_format, model_id)
            .compaction_budgets(context_window);

        Self {
            trigger_tokens: budgets.trigger_tokens,
        }
    }

    pub(crate) fn should_compact(&self, estimated_tokens: usize) -> bool {
        estimated_tokens >= self.trigger_tokens
    }
}

#[cfg(test)]
impl CompactionManager {
    fn with_trigger_tokens(trigger_tokens: usize) -> Self {
        Self { trigger_tokens }
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
        let manager = CompactionManager::with_trigger_tokens(1_000);

        assert!(!manager.should_compact(999));
        assert!(manager.should_compact(1_000));
        assert!(manager.should_compact(1_100));
    }
}
