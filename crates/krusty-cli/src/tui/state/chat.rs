//! Chat state management
//!
//! Groups conversation and streaming-related state.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::agent::loop_events::LoopEvent;
use crate::ai::model_profile::StreamDrainPolicy;
use crate::ai::types::ModelMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamDrainMode {
    #[default]
    Smooth,
    CatchUp,
}

#[derive(Debug)]
struct QueuedLoopEvent {
    event: LoopEvent,
    queued_at: Instant,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StreamDrainTelemetry {
    pub enqueued_events: u64,
    pub coalesced_events: u64,
    pub dropped_events: u64,
    pub dequeued_events: u64,
    pub mode_switches: u64,
    pub peak_pending: usize,
    pub peak_oldest_age: Duration,
}

#[derive(Debug, Default)]
pub struct StreamDrainState {
    pending: VecDeque<QueuedLoopEvent>,
    oldest_queued_at: Option<Instant>,
    mode: StreamDrainMode,
    policy: StreamDrainPolicy,
    telemetry: StreamDrainTelemetry,
}

impl StreamDrainState {
    pub fn reset(&mut self) {
        self.pending.clear();
        self.oldest_queued_at = None;
        self.mode = StreamDrainMode::Smooth;
        self.telemetry = StreamDrainTelemetry::default();
    }

    pub fn set_policy(&mut self, policy: StreamDrainPolicy) {
        self.policy = policy;
    }

    pub fn enqueue(&mut self, event: LoopEvent) {
        let now = Instant::now();
        self.telemetry.enqueued_events += 1;

        if let Some(last) = self.pending.back_mut() {
            if coalesce_pending_event(&mut last.event, &event) {
                self.telemetry.coalesced_events += 1;
                return;
            }
        }

        if self.pending.len() >= self.policy.hard_queue_limit
            && !self.drop_low_priority_pending()
            && is_low_priority_event(&event)
        {
            self.telemetry.dropped_events += 1;
            return;
        }

        if self.pending.is_empty() {
            self.oldest_queued_at = Some(now);
        }

        self.pending.push_back(QueuedLoopEvent {
            event,
            queued_at: now,
        });
        self.record_backlog_metrics();
    }

    pub fn next_batch_limit(&mut self) -> usize {
        let pending = self.pending.len();
        let oldest_age = self.oldest_age();
        let previous_mode = self.mode;

        self.mode = if pending >= self.policy.catch_up_backlog_threshold
            || oldest_age >= self.policy.catch_up_backlog_age
        {
            StreamDrainMode::CatchUp
        } else {
            StreamDrainMode::Smooth
        };
        if self.mode != previous_mode {
            self.telemetry.mode_switches += 1;
        }
        self.record_backlog_metrics();

        match self.mode {
            StreamDrainMode::CatchUp => self.policy.catch_up_batch_limit,
            StreamDrainMode::Smooth
                if pending >= self.policy.moderate_backlog_threshold
                    || oldest_age >= self.policy.moderate_backlog_age =>
            {
                self.policy.moderate_batch_limit
            }
            StreamDrainMode::Smooth => self.policy.smooth_batch_limit,
        }
    }

    pub fn pop_next(&mut self) -> Option<LoopEvent> {
        let next = self.pending.pop_front().map(|queued| {
            self.telemetry.dequeued_events += 1;
            queued.event
        });
        self.oldest_queued_at = self.pending.front().map(|queued| queued.queued_at);
        if self.pending.is_empty() {
            self.mode = StreamDrainMode::Smooth;
        }
        self.record_backlog_metrics();
        next
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn oldest_age(&self) -> Duration {
        self.oldest_queued_at
            .map(|queued_at| queued_at.elapsed())
            .unwrap_or_default()
    }

    pub fn mode(&self) -> StreamDrainMode {
        self.mode
    }

    pub fn telemetry(&self) -> StreamDrainTelemetry {
        self.telemetry
    }

    fn drop_low_priority_pending(&mut self) -> bool {
        let Some(index) = self
            .pending
            .iter()
            .position(|queued| is_low_priority_event(&queued.event))
        else {
            return false;
        };

        self.pending.remove(index);
        self.oldest_queued_at = self.pending.front().map(|queued| queued.queued_at);
        self.telemetry.dropped_events += 1;
        true
    }

    fn record_backlog_metrics(&mut self) {
        self.telemetry.peak_pending = self.telemetry.peak_pending.max(self.pending.len());
        self.telemetry.peak_oldest_age = self.telemetry.peak_oldest_age.max(self.oldest_age());
    }
}

fn coalesce_pending_event(existing: &mut LoopEvent, next: &LoopEvent) -> bool {
    match (existing, next) {
        (LoopEvent::TextDelta { delta: existing }, LoopEvent::TextDelta { delta }) => {
            existing.push_str(delta);
            true
        }
        (
            LoopEvent::ThinkingDelta { thinking: existing },
            LoopEvent::ThinkingDelta { thinking },
        ) => {
            existing.push_str(thinking);
            true
        }
        (
            LoopEvent::ToolOutputDelta {
                id: existing_id,
                delta: existing_delta,
            },
            LoopEvent::ToolOutputDelta { id, delta },
        ) if existing_id == id => {
            existing_delta.push_str(delta);
            true
        }
        _ => false,
    }
}

fn is_low_priority_event(event: &LoopEvent) -> bool {
    matches!(
        event,
        LoopEvent::TextDelta { .. }
            | LoopEvent::ThinkingDelta { .. }
            | LoopEvent::ToolOutputDelta { .. }
            | LoopEvent::ToolExecuting { .. }
    )
}

/// Chat and conversation state
///
/// Groups fields related to the conversation, messages, and streaming status.
#[derive(Default)]
pub struct ChatState {
    /// Display messages (role, content) for rendering
    pub messages: Vec<(String, String)>,
    /// Full conversation history for API
    pub conversation: Vec<ModelMessage>,
    /// True while streaming response from AI API
    pub is_streaming: bool,
    /// True while tools are executing
    pub is_executing_tools: bool,
    /// Current activity description for status display
    pub current_activity: Option<String>,
    /// Cache for streaming assistant message index (avoids O(n) scan per delta)
    pub streaming_assistant_idx: Option<usize>,
    /// Adaptive draining state for queued LoopEvents under bursty streams
    pub stream_drain: StreamDrainState,
}

impl ChatState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start streaming from AI - sets is_streaming flag and clears caches
    pub fn start_streaming_with_policy(&mut self, policy: StreamDrainPolicy) {
        self.is_streaming = true;
        self.current_activity = Some("thinking".to_string());
        self.streaming_assistant_idx = None;
        self.stream_drain.set_policy(policy);
        self.stream_drain.reset();
    }

    /// Stop streaming from AI - clears is_streaming flag and related caches
    pub fn stop_streaming(&mut self) {
        self.is_streaming = false;
        self.current_activity = None;
        self.streaming_assistant_idx = None;
    }

    /// Start tool execution - sets is_executing_tools flag
    pub fn start_tool_execution(&mut self) {
        self.is_executing_tools = true;
    }

    /// Stop tool execution - clears is_executing_tools flag
    pub fn stop_tool_execution(&mut self) {
        self.is_executing_tools = false;
        self.current_activity = None;
    }

    /// Check if busy (streaming OR executing tools)
    pub fn is_busy(&self) -> bool {
        self.is_streaming || self.is_executing_tools
    }

    pub fn has_stream_backlog(&self) -> bool {
        self.stream_drain.has_pending()
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatState, StreamDrainMode, StreamDrainState};
    use crate::agent::loop_events::LoopEvent;
    use crate::ai::model_profile::StreamDrainPolicy;
    use std::time::{Duration, Instant};

    #[test]
    fn coalesces_adjacent_text_deltas() {
        let mut drain = StreamDrainState::default();
        drain.enqueue(LoopEvent::TextDelta {
            delta: "hello".to_string(),
        });
        drain.enqueue(LoopEvent::TextDelta {
            delta: " world".to_string(),
        });

        assert_eq!(drain.pending_len(), 1);
        match drain.pop_next() {
            Some(LoopEvent::TextDelta { delta }) => assert_eq!(delta, "hello world"),
            other => panic!("expected text delta, got {:?}", other),
        }
    }

    #[test]
    fn switches_to_catch_up_mode_for_old_backlog() {
        let mut drain = StreamDrainState::default();
        drain.enqueue(LoopEvent::TextDelta {
            delta: "hello".to_string(),
        });
        drain.oldest_queued_at = Some(Instant::now() - Duration::from_millis(150));

        let batch_limit = drain.next_batch_limit();

        assert_eq!(drain.mode(), StreamDrainMode::CatchUp);
        assert!(batch_limit >= 96);
    }

    #[test]
    fn chat_state_resets_drain_on_new_stream() {
        let mut chat = ChatState::new();
        chat.stream_drain.enqueue(LoopEvent::TextDelta {
            delta: "stale".to_string(),
        });

        chat.start_streaming_with_policy(StreamDrainPolicy::default());

        assert!(!chat.has_stream_backlog());
        assert!(chat.is_streaming);
    }

    #[test]
    fn drops_low_priority_events_when_queue_overflows() {
        let mut drain = StreamDrainState::default();
        drain.set_policy(StreamDrainPolicy {
            hard_queue_limit: 2,
            ..Default::default()
        });

        drain.enqueue(LoopEvent::ToolExecuting {
            id: "call_1".to_string(),
            name: "read".to_string(),
        });
        drain.enqueue(LoopEvent::ThinkingDelta {
            thinking: "plan".to_string(),
        });
        drain.enqueue(LoopEvent::ToolOutputDelta {
            id: "call_1".to_string(),
            delta: "stdout".to_string(),
        });

        assert!(drain.pending_len() <= 2);
        assert!(drain.telemetry().dropped_events >= 1);
    }
}
