use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

use crate::agent::subagent::AgentCapability;
use crate::agent::{DelegatedRunStage, DelegatedToolKind};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedRunRole {
    Explore,
    Build,
    Planner,
    Verifier,
}

impl DelegatedRunRole {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Explore => "explore",
            Self::Build => "build",
            Self::Planner => "planner",
            Self::Verifier => "verifier",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value {
            "explore" => Some(Self::Explore),
            "build" => Some(Self::Build),
            "planner" => Some(Self::Planner),
            "verifier" => Some(Self::Verifier),
            _ => None,
        }
    }
}

impl From<DelegatedToolKind> for DelegatedRunRole {
    fn from(value: DelegatedToolKind) -> Self {
        match value {
            DelegatedToolKind::Explore => Self::Explore,
            DelegatedToolKind::Build => Self::Build,
            DelegatedToolKind::Plan => Self::Planner,
            DelegatedToolKind::Verify => Self::Verifier,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegatedRunScope {
    pub label: String,
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegatedRunAgentSnapshot {
    pub task_id: String,
    pub agent_name: String,
    pub status: String,
    pub tool_count: usize,
    pub tokens: usize,
    pub current_action: Option<String>,
    pub completion_summary: Option<String>,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub completed_plan_task: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegatedRunSnapshot {
    pub stage: DelegatedRunStage,
    #[serde(default)]
    pub agents: Vec<DelegatedRunAgentSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegatedRunRecord {
    pub delegated_run_id: String,
    pub parent_session_id: String,
    pub parent_tool_call_id: Option<String>,
    pub role: DelegatedRunRole,
    pub stage: DelegatedRunStage,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub resumable: bool,
    pub resumed_from_run_id: Option<String>,
    pub target_scope_key: String,
    pub target_scope: Vec<DelegatedRunScope>,
    pub snapshot: Option<DelegatedRunSnapshot>,
    pub artifact: Option<Value>,
    pub human_review: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Parent-chosen product identity for this child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_name: Option<String>,
    /// Exact durable capability contract. Empty means a pre-contract row and
    /// is resolved through the role fallback in `effective_capabilities`.
    #[serde(default)]
    pub capabilities: BTreeSet<AgentCapability>,
    /// Durable launch intent for server-hosted background children. This is
    /// persisted before execution so a restart can reconstruct a completion
    /// wake even when the process dies between terminal persistence and the
    /// pending-steering enqueue.
    #[serde(default)]
    pub wake_parent: bool,
}

/// Lightweight durable projection used to reconcile a hydrated transcript.
///
/// Full delegated artifacts can be large, so session state returns only a
/// small recent window of full records and this compact index for older tool
/// calls. There is at most one summary per parent tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegatedRunSummary {
    pub delegated_run_id: String,
    pub parent_session_id: String,
    pub parent_tool_call_id: String,
    pub role: DelegatedRunRole,
    pub stage: DelegatedRunStage,
    pub child_name: Option<String>,
    #[serde(default)]
    pub capabilities: BTreeSet<AgentCapability>,
    pub updated_at: DateTime<Utc>,
}

impl DelegatedRunSummary {
    pub fn effective_capabilities(&self) -> BTreeSet<AgentCapability> {
        if !self.capabilities.is_empty() {
            return self.capabilities.clone();
        }

        match self.role {
            DelegatedRunRole::Build => BTreeSet::from([
                AgentCapability::Read,
                AgentCapability::Write,
                AgentCapability::Execute,
            ]),
            DelegatedRunRole::Verifier => {
                BTreeSet::from([AgentCapability::Read, AgentCapability::Execute])
            }
            DelegatedRunRole::Explore | DelegatedRunRole::Planner => {
                BTreeSet::from([AgentCapability::Read])
            }
        }
    }
}

impl DelegatedRunRecord {
    pub fn effective_capabilities(&self) -> BTreeSet<AgentCapability> {
        if !self.capabilities.is_empty() {
            return self.capabilities.clone();
        }

        match self.role {
            DelegatedRunRole::Build => BTreeSet::from([
                AgentCapability::Read,
                AgentCapability::Write,
                AgentCapability::Execute,
            ]),
            DelegatedRunRole::Verifier => {
                BTreeSet::from([AgentCapability::Read, AgentCapability::Execute])
            }
            DelegatedRunRole::Explore | DelegatedRunRole::Planner => {
                BTreeSet::from([AgentCapability::Read])
            }
        }
    }

    /// Whether this durable terminal row is allowed to wake its parent.
    ///
    /// Normal user-requested cancellation intentionally stays quiet. The
    /// lease's `caller_aborted_before_terminal` outcome is different: the
    /// background worker vanished without a truthful terminal handoff, so the
    /// parent must be told that side effects may have occurred.
    pub fn should_wake_parent(&self) -> bool {
        if !self.wake_parent {
            return false;
        }

        match self.stage {
            DelegatedRunStage::Complete
            | DelegatedRunStage::Degraded
            | DelegatedRunStage::Failed => true,
            DelegatedRunStage::Cancelled => {
                matches!(
                    self.artifact
                        .as_ref()
                        .and_then(|artifact| artifact.get("outcome_reason"))
                        .and_then(Value::as_str),
                    Some("caller_aborted_before_terminal" | "background_host_lease_expired")
                )
            }
            DelegatedRunStage::Created
            | DelegatedRunStage::Running
            | DelegatedRunStage::Synthesizing => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DelegatedRunStartInput {
    pub delegated_run_id: String,
    pub parent_session_id: String,
    pub parent_tool_call_id: Option<String>,
    pub role: DelegatedRunRole,
    pub stage: DelegatedRunStage,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub resumable: bool,
    pub resumed_from_run_id: Option<String>,
    pub target_scope: Vec<DelegatedRunScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegatedRunCreateOutcome {
    Created,
    ExistingContinuation {
        delegated_run_id: String,
        resumed_from_run_id: String,
    },
}

pub fn normalize_scope_key(target_scope: &[DelegatedRunScope]) -> String {
    let mut entries = target_scope
        .iter()
        .map(|scope| {
            format!(
                "{}|{}|{}",
                scope.kind.trim(),
                scope.label.trim(),
                scope.path.trim()
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries.join("\n")
}
