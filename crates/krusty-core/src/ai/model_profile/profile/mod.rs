use serde::{Deserialize, Serialize};
use std::time::Duration;

mod prompting;
mod resolution;
mod runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptFamily {
    AnthropicClaude,
    OpenAiCodex,
    OpenAiReasoning,
    GoogleGemini,
    #[default]
    GenericCoding,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelProfile {
    pub prompt_family: PromptFamily,
    pub usable_context_ratio: f32,
    pub auto_compact_threshold_ratio: f32,
    pub compaction_target_ratio: f32,
    pub hard_failure_ratio: f32,
    pub stream_idle_timeout_secs: u64,
    pub supports_reasoning_summary: bool,
    pub prefer_parallel_tool_calls: bool,
}

pub struct CompactionBudgets {
    pub trigger_tokens: usize,
    pub target_tokens: usize,
    pub hard_failure_tokens: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamDrainPolicy {
    pub smooth_batch_limit: usize,
    pub moderate_batch_limit: usize,
    pub catch_up_batch_limit: usize,
    pub moderate_backlog_threshold: usize,
    pub catch_up_backlog_threshold: usize,
    pub moderate_backlog_age: Duration,
    pub catch_up_backlog_age: Duration,
    pub hard_queue_limit: usize,
}
