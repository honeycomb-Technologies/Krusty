use std::time::Duration;

use super::{CompactionBudgets, ModelProfile, PromptFamily, StreamDrainPolicy};

impl ModelProfile {
    pub fn compaction_budgets(self, context_window: usize) -> CompactionBudgets {
        let context_window = context_window.max(1);
        let trigger_tokens = ratio_to_tokens(context_window, self.auto_compact_threshold_ratio);
        let target_tokens = ratio_to_tokens(context_window, self.compaction_target_ratio)
            .min(trigger_tokens.saturating_sub(1).max(1));
        let hard_failure_tokens = ratio_to_tokens(context_window, self.hard_failure_ratio)
            .max(trigger_tokens.saturating_add(1));

        CompactionBudgets {
            trigger_tokens,
            target_tokens,
            hard_failure_tokens,
        }
    }

    pub fn stream_drain_policy(self) -> StreamDrainPolicy {
        match self.prompt_family {
            PromptFamily::OpenAiCodex => StreamDrainPolicy {
                smooth_batch_limit: 18,
                moderate_batch_limit: 48,
                catch_up_batch_limit: 128,
                moderate_backlog_threshold: 28,
                catch_up_backlog_threshold: 96,
                moderate_backlog_age: Duration::from_millis(35),
                catch_up_backlog_age: Duration::from_millis(100),
                hard_queue_limit: 512,
            },
            PromptFamily::OpenAiReasoning => StreamDrainPolicy {
                smooth_batch_limit: 14,
                moderate_batch_limit: 40,
                catch_up_batch_limit: 112,
                moderate_backlog_threshold: 24,
                catch_up_backlog_threshold: 88,
                moderate_backlog_age: Duration::from_millis(40),
                catch_up_backlog_age: Duration::from_millis(110),
                hard_queue_limit: 448,
            },
            PromptFamily::AnthropicClaude | PromptFamily::GoogleGemini => StreamDrainPolicy {
                smooth_batch_limit: 12,
                moderate_batch_limit: 28,
                catch_up_batch_limit: 80,
                moderate_backlog_threshold: 20,
                catch_up_backlog_threshold: 72,
                moderate_backlog_age: Duration::from_millis(40),
                catch_up_backlog_age: Duration::from_millis(120),
                hard_queue_limit: 384,
            },
            PromptFamily::GenericCoding => StreamDrainPolicy::default(),
        }
    }
}

impl Default for StreamDrainPolicy {
    fn default() -> Self {
        Self {
            smooth_batch_limit: 12,
            moderate_batch_limit: 32,
            catch_up_batch_limit: 96,
            moderate_backlog_threshold: 24,
            catch_up_backlog_threshold: 80,
            moderate_backlog_age: Duration::from_millis(40),
            catch_up_backlog_age: Duration::from_millis(120),
            hard_queue_limit: 384,
        }
    }
}

fn ratio_to_tokens(context_window: usize, ratio: f32) -> usize {
    ((context_window as f64 * ratio as f64).round() as usize)
        .max(1)
        .min(context_window)
}
