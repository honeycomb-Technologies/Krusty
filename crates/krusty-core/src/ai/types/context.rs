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
    pub completion_tokens: usize,
    pub total_tokens: usize,
    /// Tokens written to cache (25% extra cost)
    #[serde(default)]
    pub cache_creation_input_tokens: usize,
    /// Tokens read from cache (10% cost vs 100%)
    #[serde(default)]
    pub cache_read_input_tokens: usize,
}

impl Usage {
    /// Total input represented by this usage snapshot, including cached input.
    pub fn input_tokens(&self) -> usize {
        self.prompt_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }

    /// Merge cumulative provider snapshots from a single streamed response.
    ///
    /// Providers such as Anthropic report input at message start and output at
    /// message delta. Each bucket is cumulative when present, so taking the
    /// maximum preserves the complete turn without adding repeated snapshots.
    pub fn merge_snapshot(&mut self, snapshot: &Self) {
        self.prompt_tokens = self.prompt_tokens.max(snapshot.prompt_tokens);
        self.completion_tokens = self.completion_tokens.max(snapshot.completion_tokens);
        self.cache_creation_input_tokens = self
            .cache_creation_input_tokens
            .max(snapshot.cache_creation_input_tokens);
        self.cache_read_input_tokens = self
            .cache_read_input_tokens
            .max(snapshot.cache_read_input_tokens);

        let represented_total = self.input_tokens().saturating_add(self.completion_tokens);
        self.total_tokens = self
            .total_tokens
            .max(snapshot.total_tokens)
            .max(represented_total);
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
    /// Default for extended thinking + tools (Krusty's main use case)
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
