//! Context ledger for deterministic continuation semantics.
//!
//! Tracks high-level context state transitions (canonical/summarized/dropped/pinned/replayed)
//! so compaction and resume decisions can be explicit and auditable.

use crate::ai::types::{Content, ModelMessage, Role};
use serde::{Deserialize, Serialize};

use super::compaction::CompactionReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NonResumableReason {
    MissingUserObjective,
    EmptyConversation,
}

impl NonResumableReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingUserObjective => "missing_user_objective",
            Self::EmptyConversation => "empty_conversation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ContinuationDecision {
    Resumable { latest_user_objective: String },
    NonResumable { reason: NonResumableReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum ContinuationContractDecision {
    Resumable { latest_user_objective: String },
    NonResumable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContinuationContract {
    pub(crate) schema_version: u8,
    pub(crate) decision: ContinuationContractDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactionLedgerSnapshot {
    pub(crate) reason: CompactionReason,
    pub(crate) estimated_tokens_before: usize,
    pub(crate) estimated_tokens_after: usize,
    pub(crate) replaced_messages: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CompactionLedgerSnapshotRecord {
    pub(crate) reason: String,
    pub(crate) estimated_tokens_before: usize,
    pub(crate) estimated_tokens_after: usize,
    pub(crate) replaced_messages: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ContextLedgerRecord {
    pub(crate) schema_version: u8,
    pub(crate) canonical_messages: usize,
    pub(crate) summarized_messages: usize,
    pub(crate) dropped_messages: usize,
    pub(crate) pinned_messages: usize,
    pub(crate) replayed_messages: usize,
    pub(crate) latest_user_objective: Option<String>,
    pub(crate) last_compaction: Option<CompactionLedgerSnapshotRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ContextLedger {
    pub(crate) canonical_messages: usize,
    pub(crate) summarized_messages: usize,
    pub(crate) dropped_messages: usize,
    pub(crate) pinned_messages: usize,
    pub(crate) replayed_messages: usize,
    pub(crate) latest_user_objective: Option<String>,
    pub(crate) last_compaction: Option<CompactionLedgerSnapshot>,
}

impl ContextLedger {
    pub(crate) fn from_conversation(conversation: &[ModelMessage]) -> Self {
        let mut ledger = Self::default();
        ledger.recompute(conversation);
        ledger
    }

    pub(crate) fn update_from_conversation(&mut self, conversation: &[ModelMessage]) {
        self.recompute(conversation);
    }

    pub(crate) fn record_compaction(
        &mut self,
        reason: CompactionReason,
        estimated_tokens_before: usize,
        estimated_tokens_after: usize,
        replaced_messages: usize,
    ) {
        self.last_compaction = Some(CompactionLedgerSnapshot {
            reason,
            estimated_tokens_before,
            estimated_tokens_after,
            replaced_messages,
        });
    }

    pub(crate) fn continuation_decision(&self) -> ContinuationDecision {
        if self.canonical_messages == 0 {
            return ContinuationDecision::NonResumable {
                reason: NonResumableReason::EmptyConversation,
            };
        }

        match self.latest_user_objective.as_ref().map(|s| s.trim()) {
            Some(text) if !text.is_empty() => ContinuationDecision::Resumable {
                latest_user_objective: text.to_string(),
            },
            _ => ContinuationDecision::NonResumable {
                reason: NonResumableReason::MissingUserObjective,
            },
        }
    }

    pub(crate) fn persistence_record(&self) -> ContextLedgerRecord {
        ContextLedgerRecord {
            schema_version: 1,
            canonical_messages: self.canonical_messages,
            summarized_messages: self.summarized_messages,
            dropped_messages: self.dropped_messages,
            pinned_messages: self.pinned_messages,
            replayed_messages: self.replayed_messages,
            latest_user_objective: self.latest_user_objective.clone(),
            last_compaction: self
                .last_compaction
                .map(|snapshot| CompactionLedgerSnapshotRecord {
                    reason: snapshot.reason.as_str().to_string(),
                    estimated_tokens_before: snapshot.estimated_tokens_before,
                    estimated_tokens_after: snapshot.estimated_tokens_after,
                    replaced_messages: snapshot.replaced_messages,
                }),
        }
    }

    pub(crate) fn continuation_contract(&self) -> ContinuationContract {
        let decision = match self.continuation_decision() {
            ContinuationDecision::Resumable {
                latest_user_objective,
            } => ContinuationContractDecision::Resumable {
                latest_user_objective,
            },
            ContinuationDecision::NonResumable { reason } => {
                ContinuationContractDecision::NonResumable {
                    reason: reason.as_str().to_string(),
                }
            }
        };

        ContinuationContract {
            schema_version: 1,
            decision,
        }
    }

    fn recompute(&mut self, conversation: &[ModelMessage]) {
        self.canonical_messages = conversation.len();
        self.summarized_messages = 0;
        self.dropped_messages = 0;
        self.pinned_messages = 0;
        self.latest_user_objective = None;

        for message in conversation {
            if message.role == Role::System
                && first_text(&message.content)
                    .is_some_and(|text| text.starts_with("[PROJECT INSTRUCTIONS"))
            {
                self.pinned_messages += 1;
            }

            if message.role == Role::User {
                for content in &message.content {
                    if let Content::ToolResult { output, .. } = content {
                        if output
                            .get("retention")
                            .and_then(|value| value.as_str())
                            .is_some_and(|value| value == "summarize_after_turn")
                        {
                            self.summarized_messages += 1;
                        }
                        if output
                            .get("retention")
                            .and_then(|value| value.as_str())
                            .is_some_and(|value| value == "drop_after_compaction")
                        {
                            self.dropped_messages += 1;
                        }
                    }
                }
            }
        }

        self.latest_user_objective = extract_latest_user_objective(conversation);
    }
}

fn first_text(content: &[Content]) -> Option<&str> {
    content.iter().find_map(|c| {
        if let Content::Text { text } = c {
            Some(text.as_str())
        } else {
            None
        }
    })
}

fn extract_latest_user_objective(conversation: &[ModelMessage]) -> Option<String> {
    conversation.iter().rev().find_map(|message| {
        if message.role != Role::User {
            return None;
        }

        message.content.iter().find_map(|content| {
            if let Content::Text { text } = content {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            } else {
                None
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text_message(role: Role, text: &str) -> ModelMessage {
        ModelMessage {
            role,
            content: vec![Content::Text {
                text: text.to_string(),
            }],
        }
    }

    #[test]
    fn ledger_extracts_latest_user_objective() {
        let conversation = vec![
            text_message(Role::User, "first request"),
            text_message(Role::Assistant, "thinking"),
            text_message(Role::User, "final request"),
        ];

        let ledger = ContextLedger::from_conversation(&conversation);
        assert_eq!(
            ledger.continuation_decision(),
            ContinuationDecision::Resumable {
                latest_user_objective: "final request".to_string()
            }
        );
    }

    #[test]
    fn ledger_detects_missing_objective() {
        let conversation = vec![ModelMessage {
            role: Role::User,
            content: vec![Content::ToolResult {
                tool_use_id: "call_1".to_string(),
                output: json!({"retention":"summarize_after_turn"}),
                is_error: None,
            }],
        }];

        let ledger = ContextLedger::from_conversation(&conversation);
        assert_eq!(
            ledger.continuation_decision(),
            ContinuationDecision::NonResumable {
                reason: NonResumableReason::MissingUserObjective
            }
        );
    }

    #[test]
    fn ledger_tracks_retention_and_pinned_counts() {
        let conversation = vec![
            text_message(
                Role::System,
                "[PROJECT INSTRUCTIONS - /repo/AGENTS.md]\nUse Rust",
            ),
            ModelMessage {
                role: Role::User,
                content: vec![Content::ToolResult {
                    tool_use_id: "call_1".to_string(),
                    output: json!({"retention":"drop_after_compaction"}),
                    is_error: None,
                }],
            },
        ];

        let ledger = ContextLedger::from_conversation(&conversation);
        assert_eq!(ledger.pinned_messages, 1);
        assert_eq!(ledger.dropped_messages, 1);
    }

    #[test]
    fn ledger_serializes_persistence_contract() {
        let conversation = vec![text_message(Role::User, "stabilize streaming resumes")];
        let mut ledger = ContextLedger::from_conversation(&conversation);
        ledger.record_compaction(CompactionReason::ContextPressure, 5000, 1800, 6);

        let record = ledger.persistence_record();
        assert_eq!(record.schema_version, 1);
        assert_eq!(
            record.latest_user_objective.as_deref(),
            Some("stabilize streaming resumes")
        );
        assert_eq!(
            record
                .last_compaction
                .as_ref()
                .map(|snapshot| snapshot.reason.as_str()),
            Some("context_pressure")
        );

        let contract = ledger.continuation_contract();
        assert_eq!(contract.schema_version, 1);
        match contract.decision {
            ContinuationContractDecision::Resumable {
                latest_user_objective,
            } => assert_eq!(latest_user_objective, "stabilize streaming resumes"),
            ContinuationContractDecision::NonResumable { reason } => {
                panic!("unexpected non-resumable decision: {reason}")
            }
        }
    }
}
