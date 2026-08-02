use serde::{Deserialize, Serialize};

use crate::ai::reasoning::DEFAULT_THINKING_BUDGET;

/// Usage information with cache metrics.
///
/// `prompt_tokens` is the uncached input bucket. Cache reads and writes stay
/// separate so callers can compute both the real context size and provider
/// cost without double-counting.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: usize,
    /// Generated output, including any provider-reported reasoning tokens.
    pub completion_tokens: usize,
    /// Reasoning/thinking tokens contained within `completion_tokens`.
    ///
    /// This is an observability bucket, not an additional contribution to
    /// `total_tokens`.
    #[serde(default)]
    pub reasoning_tokens: usize,
    pub total_tokens: usize,
    /// Tokens written to cache (25% extra cost)
    #[serde(default)]
    pub cache_creation_input_tokens: usize,
    /// Tokens read from cache (10% cost vs 100%)
    #[serde(default)]
    pub cache_read_input_tokens: usize,
}

impl Usage {
    /// Logical input represented by this usage snapshot, including cached
    /// reads and writes. `prompt_tokens` alone is only the uncached bucket.
    pub fn input_tokens(&self) -> usize {
        self.prompt_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }

    /// Logical input plus generated output, preserving a provider total when
    /// it is larger than the represented public buckets.
    ///
    /// This is the cross-provider total used by context gauges and durable
    /// session token counts. It is derived from the normalized buckets rather
    /// than trusting a provider's differently-defined `total_tokens` field.
    pub fn logical_total_tokens(&self) -> usize {
        self.total_tokens
            .max(self.input_tokens().saturating_add(self.completion_tokens))
    }

    /// Merge cumulative provider snapshots from a single streamed response.
    ///
    /// Providers such as Anthropic report input at message start and output at
    /// message delta. Each bucket is cumulative when present, so taking the
    /// maximum preserves the complete turn without adding repeated snapshots.
    pub fn merge_snapshot(&mut self, snapshot: &Self) {
        self.prompt_tokens = self.prompt_tokens.max(snapshot.prompt_tokens);
        self.completion_tokens = self.completion_tokens.max(snapshot.completion_tokens);
        self.reasoning_tokens = self
            .reasoning_tokens
            .max(snapshot.reasoning_tokens)
            .min(self.completion_tokens);
        self.cache_creation_input_tokens = self
            .cache_creation_input_tokens
            .max(snapshot.cache_creation_input_tokens);
        self.cache_read_input_tokens = self
            .cache_read_input_tokens
            .max(snapshot.cache_read_input_tokens);

        self.total_tokens = self
            .total_tokens
            .max(snapshot.total_tokens)
            .max(self.input_tokens().saturating_add(self.completion_tokens));
    }
}

#[cfg(test)]
mod tests {
    use super::Usage;

    #[test]
    fn logical_totals_include_each_cache_bucket_once() {
        let usage = Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            reasoning_tokens: 0,
            total_tokens: 999,
            cache_creation_input_tokens: 200,
            cache_read_input_tokens: 700,
        };

        assert_eq!(usage.input_tokens(), 1_000);
        assert_eq!(usage.logical_total_tokens(), 1_050);
    }

    #[test]
    fn merging_snapshots_normalizes_provider_total_semantics() {
        let mut usage = Usage::default();
        usage.merge_snapshot(&Usage {
            prompt_tokens: 100,
            completion_tokens: 0,
            reasoning_tokens: 0,
            total_tokens: 100,
            cache_creation_input_tokens: 200,
            cache_read_input_tokens: 700,
        });
        usage.merge_snapshot(&Usage {
            prompt_tokens: 0,
            completion_tokens: 50,
            reasoning_tokens: 0,
            total_tokens: 50,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        });

        assert_eq!(usage.total_tokens, 1_050);
    }

    #[test]
    fn reasoning_tokens_are_observable_without_double_counting_totals() {
        let mut usage = Usage::default();
        usage.merge_snapshot(&Usage {
            prompt_tokens: 1_000,
            completion_tokens: 550,
            reasoning_tokens: 500,
            total_tokens: 1_550,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        });

        assert_eq!(usage.input_tokens(), 1_000);
        assert_eq!(usage.completion_tokens, 550);
        assert_eq!(usage.reasoning_tokens, 500);
        assert_eq!(usage.logical_total_tokens(), 1_550);
        assert_eq!(usage.total_tokens, 1_550);
    }

    #[test]
    fn merge_clamps_reasoning_to_the_completion_bucket() {
        let mut usage = Usage::default();
        usage.merge_snapshot(&Usage {
            prompt_tokens: 0,
            completion_tokens: 50,
            reasoning_tokens: 500,
            total_tokens: 50,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        });

        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.reasoning_tokens, 50);
        assert_eq!(usage.logical_total_tokens(), 50);
    }
}

/// Context management configuration for automatic context editing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManagement {
    pub edits: Vec<ContextEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContextEdit {
    #[serde(rename = "clear_tool_uses_20250919")]
    ClearToolUses {
        #[serde(skip_serializing_if = "Option::is_none")]
        trigger: Option<ContextTrigger>,
        #[serde(skip_serializing_if = "Option::is_none")]
        keep: Option<KeepConfig>,
    },
    #[serde(rename = "clear_thinking_20251015")]
    ClearThinking {
        #[serde(skip_serializing_if = "Option::is_none")]
        keep: Option<KeepConfig>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContextTrigger {
    #[serde(rename = "input_tokens")]
    InputTokens { value: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum KeepConfig {
    #[serde(rename = "tool_uses")]
    ToolUses { value: usize },
    #[serde(rename = "thinking_turns")]
    ThinkingTurns { value: usize },
}

/// Metrics from context editing operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextEditingMetrics {
    pub cleared_tool_uses: usize,
    pub cleared_thinking_turns: usize,
    pub cleared_input_tokens: usize,
}

impl ContextManagement {
    /// Default for extended thinking + tools (Mitsuro's main use case)
    /// Note: clear_thinking must come before clear_tool_uses per API requirement
    pub fn default_for_thinking_and_tools() -> Self {
        Self {
            edits: vec![
                // Thinking clearing - keep last 2 turns
                ContextEdit::ClearThinking {
                    keep: Some(KeepConfig::ThinkingTurns { value: 2 }),
                },
                // Tool clearing - trigger at 100k tokens, keep last 5
                ContextEdit::ClearToolUses {
                    trigger: Some(ContextTrigger::InputTokens { value: 100_000 }),
                    keep: Some(KeepConfig::ToolUses { value: 5 }),
                },
            ],
        }
    }

    /// For tools without thinking
    pub fn default_tools_only() -> Self {
        Self {
            edits: vec![ContextEdit::ClearToolUses {
                trigger: Some(ContextTrigger::InputTokens { value: 100_000 }),
                keep: Some(KeepConfig::ToolUses { value: 5 }),
            }],
        }
    }
}

/// Extended thinking configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    pub budget_tokens: u32,
}

impl Default for ThinkingConfig {
    fn default() -> Self {
        Self {
            budget_tokens: DEFAULT_THINKING_BUDGET,
        }
    }
}
