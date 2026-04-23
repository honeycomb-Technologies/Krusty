use crate::agent::context_ledger::{ContextLedger, ContinuationDecision, NonResumableReason};
use crate::agent::stream;
use crate::storage::{
    PartialAssistantState, RecoveryDecision, RecoveryNonResumableReason, RecoveryStatus,
    RecoveryToolCall, SessionRecoveryState,
};

use super::super::loop_events::LoopStopReason;

pub(super) fn continuation_recovery_message(ledger: &ContextLedger) -> String {
    match ledger.continuation_decision() {
        ContinuationDecision::Resumable {
            latest_user_objective,
        } => format!(
            "Continuation contract: resumable. Preserved objective: {}",
            latest_user_objective
        ),
        ContinuationDecision::NonResumable { reason } => format!(
            "Continuation contract: non-resumable ({:?}). Start a fresh turn with explicit user intent.",
            reason
        ),
    }
}

pub(super) fn build_partial_assistant_state(
    checkpoint: &stream::StreamCheckpoint,
) -> PartialAssistantState {
    PartialAssistantState {
        text: checkpoint.text.clone(),
        thinking: checkpoint.thinking.clone(),
        tool_calls: checkpoint
            .tool_calls
            .iter()
            .map(|tool| RecoveryToolCall {
                id: tool.id.clone(),
                name: tool.name.clone(),
            })
            .collect(),
    }
}

pub(super) fn build_recovery_state(
    ledger: &ContextLedger,
    status: RecoveryStatus,
    stop_reason: Option<LoopStopReason>,
    last_error: Option<String>,
    partial_assistant: PartialAssistantState,
) -> SessionRecoveryState {
    SessionRecoveryState::new(
        status.clone(),
        stop_reason,
        last_error,
        partial_assistant.clone(),
        recovery_decision(ledger, &status, &partial_assistant),
    )
}

fn recovery_decision(
    ledger: &ContextLedger,
    status: &RecoveryStatus,
    partial_assistant: &PartialAssistantState,
) -> RecoveryDecision {
    let override_reason = match status {
        RecoveryStatus::ToolExecuting => Some(RecoveryNonResumableReason::ToolExecutionInProgress),
        RecoveryStatus::Streaming | RecoveryStatus::Interrupted
            if !partial_assistant.tool_calls.is_empty() =>
        {
            Some(RecoveryNonResumableReason::PendingToolCall)
        }
        _ => None,
    };

    if let Some(reason) = override_reason {
        return RecoveryDecision::NonResumable { reason };
    }

    match ledger.continuation_decision() {
        ContinuationDecision::Resumable {
            latest_user_objective,
        } => RecoveryDecision::Resumable {
            latest_user_objective,
        },
        ContinuationDecision::NonResumable { reason } => RecoveryDecision::NonResumable {
            reason: match reason {
                NonResumableReason::MissingUserObjective => {
                    RecoveryNonResumableReason::MissingUserObjective
                }
                NonResumableReason::EmptyConversation => {
                    RecoveryNonResumableReason::EmptyConversation
                }
            },
        },
    }
}
