//! Agent events
//!
//! Events that occur during agent execution.

use crate::ai::types::{FinishReason, Usage};
use serde::Serialize;

/// Events during agent execution
#[derive(Debug, Clone, Serialize)]
pub enum AgentEvent {
    /// Turn started
    TurnStart { turn: usize, message_count: usize },
    /// Turn completed
    TurnComplete {
        turn: usize,
        duration_ms: u64,
        tokens: Usage,
    },
    /// Stream ended
    StreamEnd { reason: FinishReason },
    /// Stream error
    StreamError { error: String },
    /// Execution interrupted
    Interrupt {
        turn: usize,
        reason: InterruptReason,
    },
}

/// Reasons for interrupting execution
#[derive(Debug, Clone, Serialize)]
pub enum InterruptReason {
    UserRequested,
    MaxTurnsReached,
}
