use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::mapping::{
    failure_category_for_event, loop_event_type, stop_reason_for_event, summarize_loop_event,
};
use crate::agent::loop_events::{LoopEvent, LoopStopReason};

/// Canonical failure taxonomy for agent runtime traces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TraceFailureCategory {
    AgentError,
    ProviderError,
    BudgetExhausted,
    LoopGuardTriggered,
    StreamIdleTimeout,
    PinchFailed,
    UserAbort,
    ToolExecutionError,
    ServerToolError,
    ToolDenied,
}

impl TraceFailureCategory {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::AgentError => "agent_error",
            Self::ProviderError => "provider_error",
            Self::BudgetExhausted => "budget_exhausted",
            Self::LoopGuardTriggered => "loop_guard_triggered",
            Self::StreamIdleTimeout => "stream_idle_timeout",
            Self::PinchFailed => "pinch_failed",
            Self::UserAbort => "user_abort",
            Self::ToolExecutionError => "tool_execution_error",
            Self::ServerToolError => "server_tool_error",
            Self::ToolDenied => "tool_denied",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value {
            "agent_error" => Some(Self::AgentError),
            "provider_error" => Some(Self::ProviderError),
            "budget_exhausted" => Some(Self::BudgetExhausted),
            "loop_guard_triggered" => Some(Self::LoopGuardTriggered),
            "stream_idle_timeout" => Some(Self::StreamIdleTimeout),
            "pinch_failed" | "context_compaction_failed" => Some(Self::PinchFailed),
            "user_abort" => Some(Self::UserAbort),
            "tool_execution_error" => Some(Self::ToolExecutionError),
            "server_tool_error" => Some(Self::ServerToolError),
            "tool_denied" => Some(Self::ToolDenied),
            _ => None,
        }
    }
}

/// Compact persisted trace event derived from a canonical `LoopEvent`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTraceEvent {
    pub run_id: String,
    pub sequence: i64,
    pub turn: usize,
    pub event_type: String,
    pub payload: Value,
    pub failure_category: Option<TraceFailureCategory>,
    pub stop_reason: Option<LoopStopReason>,
    pub created_at: String,
}

impl RuntimeTraceEvent {
    pub fn from_loop_event(
        run_id: impl Into<String>,
        sequence: i64,
        turn: usize,
        event: &LoopEvent,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            sequence,
            turn,
            event_type: loop_event_type(event).to_string(),
            payload: summarize_loop_event(event),
            failure_category: failure_category_for_event(event),
            stop_reason: stop_reason_for_event(event),
            created_at: Utc::now().to_rfc3339(),
        }
    }
}
