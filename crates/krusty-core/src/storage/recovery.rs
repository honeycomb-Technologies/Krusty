//! Session recovery state persisted separately from canonical conversation history.
//!
//! This captures interrupted in-flight work without mutating the durable thread.

use serde::{Deserialize, Serialize};

use crate::agent::loop_events::LoopStopReason;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStatus {
    Streaming,
    ToolExecuting,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryNonResumableReason {
    MissingUserObjective,
    EmptyConversation,
    PendingToolCall,
    ToolExecutionInProgress,
}

impl RecoveryNonResumableReason {
    pub fn message(&self) -> &'static str {
        match self {
            Self::MissingUserObjective => {
                "Krusty did not auto-resume because no stable user objective was preserved."
            }
            Self::EmptyConversation => {
                "Krusty did not auto-resume because the conversation has no recoverable context."
            }
            Self::PendingToolCall => {
                "Krusty did not auto-resume because a tool call was only partially received."
            }
            Self::ToolExecutionInProgress => {
                "Krusty did not auto-resume because tools may already have run."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RecoveryDecision {
    Resumable { latest_user_objective: String },
    NonResumable { reason: RecoveryNonResumableReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryToolCall {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PartialAssistantState {
    pub text: String,
    #[serde(default)]
    pub thinking: String,
    #[serde(default)]
    pub tool_calls: Vec<RecoveryToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecoveryState {
    pub schema_version: u8,
    pub status: RecoveryStatus,
    pub stop_reason: Option<LoopStopReason>,
    pub last_error: Option<String>,
    pub partial_assistant: PartialAssistantState,
    pub decision: RecoveryDecision,
}

impl SessionRecoveryState {
    pub fn new(
        status: RecoveryStatus,
        stop_reason: Option<LoopStopReason>,
        last_error: Option<String>,
        partial_assistant: PartialAssistantState,
        decision: RecoveryDecision,
    ) -> Self {
        Self {
            schema_version: 1,
            status,
            stop_reason,
            last_error,
            partial_assistant,
            decision,
        }
    }

    pub fn is_resumable(&self) -> bool {
        matches!(self.decision, RecoveryDecision::Resumable { .. })
    }

    pub fn latest_user_objective(&self) -> Option<&str> {
        match &self.decision {
            RecoveryDecision::Resumable {
                latest_user_objective,
            } => Some(latest_user_objective.as_str()),
            RecoveryDecision::NonResumable { .. } => None,
        }
    }

    pub fn pending_tool_calls(&self) -> &[RecoveryToolCall] {
        &self.partial_assistant.tool_calls
    }

    pub fn notice(&self) -> String {
        let headline = match (&self.status, &self.stop_reason) {
            (RecoveryStatus::Streaming, _) => {
                "Previous turn ended while the assistant was still streaming."
            }
            (RecoveryStatus::ToolExecuting, _) => {
                "Previous turn ended while tool execution was in progress."
            }
            (_, Some(LoopStopReason::StreamIdleTimeout)) => {
                "Previous turn stopped after the provider stream went idle."
            }
            (_, Some(LoopStopReason::ProviderError)) => {
                "Previous turn stopped after a provider error."
            }
            (_, Some(LoopStopReason::UserAbort)) => {
                "Previous turn was interrupted by user cancellation."
            }
            (_, _) => "Previous turn ended before Krusty could safely finalize it.",
        };

        let continuation = match &self.decision {
            RecoveryDecision::Resumable {
                latest_user_objective,
            } => format!("Safe manual resume target: {}.", latest_user_objective),
            RecoveryDecision::NonResumable { reason } => reason.message().to_string(),
        };

        let tool_detail = if self.partial_assistant.tool_calls.is_empty() {
            String::new()
        } else {
            let tools = self
                .partial_assistant
                .tool_calls
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(" Pending tool calls: {}.", tools)
        };

        format!("{headline} {continuation}{tool_detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resumable_notice_mentions_objective() {
        let state = SessionRecoveryState::new(
            RecoveryStatus::Interrupted,
            Some(LoopStopReason::StreamIdleTimeout),
            None,
            PartialAssistantState {
                text: "partial answer".to_string(),
                thinking: "reasoning".to_string(),
                tool_calls: Vec::new(),
            },
            RecoveryDecision::Resumable {
                latest_user_objective: "finish the parser fix".to_string(),
            },
        );

        let notice = state.notice();
        assert!(notice.contains("provider stream went idle"));
        assert!(notice.contains("finish the parser fix"));
    }

    #[test]
    fn non_resumable_notice_mentions_pending_tools() {
        let state = SessionRecoveryState::new(
            RecoveryStatus::Interrupted,
            Some(LoopStopReason::ProviderError),
            Some("provider failed".to_string()),
            PartialAssistantState {
                text: String::new(),
                thinking: String::new(),
                tool_calls: vec![RecoveryToolCall {
                    id: "tool-1".to_string(),
                    name: "bash".to_string(),
                }],
            },
            RecoveryDecision::NonResumable {
                reason: RecoveryNonResumableReason::PendingToolCall,
            },
        );

        let notice = state.notice();
        assert!(notice.contains("provider error"));
        assert!(notice.contains("Pending tool calls: bash."));
    }
}
