//! Shared error, status, and turn-stream event types for agent backends.

use serde_json::Value;
use thiserror::Error;

use crate::approvals::PendingApproval;
use crate::process::ProcessOutputStream;

/// Backend-neutral lifecycle of a durable delegation group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationGroupStatus {
    Created,
    Queued,
    Running,
    ReadyForParent,
    Synthesizing,
    Complete,
    Degraded,
    Failed,
    Cancelled,
}

impl DelegationGroupStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::ReadyForParent => "ready for parent",
            Self::Synthesizing => "synthesizing",
            Self::Complete => "complete",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Degraded | Self::Failed | Self::Cancelled
        )
    }
}

/// Backend-neutral lifecycle of one durable delegated task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationTaskStatus {
    Created,
    Queued,
    Leased,
    Running,
    Retrying,
    Complete,
    Degraded,
    Failed,
    Cancelled,
}

impl DelegationTaskStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::Retrying => "retrying",
            Self::Complete => "complete",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Degraded | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationExecution {
    Foreground,
    Detached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationParentContinuationStatus {
    NotRequested,
    Pending,
    Queued,
    Promoted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationRole {
    Unknown,
    Explore,
    Build,
    Planner,
    Verifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationKind {
    Explore,
    Plan,
    Verify,
    Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationRunStage {
    Created,
    Running,
    Synthesizing,
    Complete,
    Degraded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationTaskProjection {
    pub id: String,
    pub key: String,
    pub role: DelegationRole,
    pub status: DelegationTaskStatus,
    pub attempt_count: usize,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationGroupProjection {
    pub id: String,
    pub parent_tool_call_id: Option<String>,
    pub status: DelegationGroupStatus,
    pub execution: DelegationExecution,
    pub parent_continuation: DelegationParentContinuationStatus,
    pub tasks: Vec<DelegationTaskProjection>,
    pub updated_at: String,
}

/// Durable event kinds remain open so a newer server event is never erased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableDelegationEventKind {
    GroupCreated,
    GroupQueued,
    GroupStateChanged,
    TaskClaimed,
    TaskRunning,
    TaskStateChanged,
    ParentContinuationQueued,
    ParentContinuationPromoted,
    Other(String),
}

impl DurableDelegationEventKind {
    pub fn label(&self) -> &str {
        match self {
            Self::GroupCreated => "group created",
            Self::GroupQueued => "group queued",
            Self::GroupStateChanged => "group state changed",
            Self::TaskClaimed => "task claimed",
            Self::TaskRunning => "task running",
            Self::TaskStateChanged => "task state changed",
            Self::ParentContinuationQueued => "parent continuation queued",
            Self::ParentContinuationPromoted => "parent continuation promoted",
            Self::Other(value) => value.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableDelegationEvent {
    pub id: i64,
    pub parent_session_id: String,
    pub group_id: String,
    pub task_id: Option<String>,
    pub kind: DurableDelegationEventKind,
    /// Raw structured payload retained for replay and forward compatibility.
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatedProgressProjection {
    pub delegated_run_id: String,
    pub tool_call_id: String,
    pub kind: DelegationKind,
    pub stage: DelegationRunStage,
    pub parent_session_id: String,
    pub task_id: String,
    pub agent_name: String,
    pub status: DelegationTaskStatus,
    pub tool_count: usize,
    pub tokens: usize,
    pub current_action: Option<String>,
    pub completion_summary: Option<String>,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub completed_plan_task: Option<String>,
}

/// Reconnect/reload projection hydrated from the canonical session state route.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionDelegationProjection {
    pub groups: Vec<DelegationGroupProjection>,
    pub events: Vec<DurableDelegationEvent>,
    pub event_cursor: Option<i64>,
}

impl SessionDelegationProjection {
    const EVENT_HISTORY_LIMIT: usize = 256;

    pub fn active_counts(&self) -> (usize, usize) {
        let active_groups = self
            .groups
            .iter()
            .filter(|group| !group.status.is_terminal())
            .count();
        let active_tasks = self
            .groups
            .iter()
            .flat_map(|group| &group.tasks)
            .filter(|task| !task.status.is_terminal())
            .count();
        (active_groups, active_tasks)
    }

    pub fn latest_task(&self) -> Option<&DelegationTaskProjection> {
        self.groups
            .iter()
            .flat_map(|group| &group.tasks)
            .max_by(|left, right| left.updated_at.cmp(&right.updated_at))
    }

    /// Merge one canonical lifecycle event into the hydrated projection.
    /// Events are monotonic and open-ended: stale duplicates are ignored and
    /// unknown future kinds advance replay without mutating known group/task
    /// state. Existing siblings and terminal groups are never replaced.
    pub fn apply_event(&mut self, event: &DurableDelegationEvent) -> bool {
        if self.event_cursor.is_some_and(|cursor| event.id <= cursor) {
            return false;
        }

        match &event.kind {
            DurableDelegationEventKind::GroupCreated => {
                let execution = event
                    .payload
                    .get("execution_mode")
                    .and_then(Value::as_str)
                    .and_then(delegation_execution_from_str)
                    .unwrap_or(DelegationExecution::Foreground);
                let group =
                    self.group_mut_or_insert(event, DelegationGroupStatus::Created, execution);
                if let Some(tasks) = event.payload.get("tasks").and_then(Value::as_array) {
                    for task in tasks {
                        let Some(id) = task.get("delegation_task_id").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        let key = task.get("task_key").and_then(Value::as_str).unwrap_or(id);
                        if !group.tasks.iter().any(|existing| existing.id == id) {
                            group.tasks.push(DelegationTaskProjection {
                                id: id.to_owned(),
                                key: key.to_owned(),
                                role: DelegationRole::Unknown,
                                status: DelegationTaskStatus::Created,
                                attempt_count: 0,
                                updated_at: event.created_at.clone(),
                            });
                        }
                    }
                }
                group.updated_at.clone_from(&event.created_at);
            }
            DurableDelegationEventKind::GroupQueued => {
                let group = self.group_mut_or_insert(
                    event,
                    DelegationGroupStatus::Queued,
                    DelegationExecution::Foreground,
                );
                group.status = DelegationGroupStatus::Queued;
                group.updated_at.clone_from(&event.created_at);
            }
            DurableDelegationEventKind::GroupStateChanged => {
                if let Some(status) = event
                    .payload
                    .get("to")
                    .and_then(Value::as_str)
                    .and_then(delegation_group_status_from_str)
                {
                    let group =
                        self.group_mut_or_insert(event, status, DelegationExecution::Foreground);
                    group.status = status;
                    group.updated_at.clone_from(&event.created_at);
                }
            }
            DurableDelegationEventKind::TaskClaimed
            | DurableDelegationEventKind::TaskRunning
            | DurableDelegationEventKind::TaskStateChanged => {
                let Some(task_id) = event.task_id.as_deref() else {
                    self.record_event(event);
                    return true;
                };
                let status = match &event.kind {
                    DurableDelegationEventKind::TaskClaimed => Some(DelegationTaskStatus::Leased),
                    DurableDelegationEventKind::TaskRunning => Some(DelegationTaskStatus::Running),
                    DurableDelegationEventKind::TaskStateChanged => event
                        .payload
                        .get("state")
                        .or_else(|| event.payload.get("to"))
                        .and_then(Value::as_str)
                        .and_then(delegation_task_status_from_str),
                    _ => None,
                };
                let attempt_count = event
                    .payload
                    .get("attempt_number")
                    .or_else(|| event.payload.get("next_attempt_number"))
                    .and_then(Value::as_u64)
                    .and_then(|attempt| usize::try_from(attempt).ok());
                let group = self.group_mut_or_insert(
                    event,
                    DelegationGroupStatus::Created,
                    DelegationExecution::Foreground,
                );
                let task =
                    if let Some(index) = group.tasks.iter().position(|task| task.id == task_id) {
                        &mut group.tasks[index]
                    } else {
                        group.tasks.push(DelegationTaskProjection {
                            id: task_id.to_owned(),
                            key: task_id.to_owned(),
                            role: DelegationRole::Unknown,
                            status: DelegationTaskStatus::Created,
                            attempt_count: 0,
                            updated_at: event.created_at.clone(),
                        });
                        group.tasks.last_mut().expect("inserted delegation task")
                    };
                if let Some(status) = status {
                    task.status = status;
                }
                if let Some(attempt_count) = attempt_count {
                    task.attempt_count = attempt_count;
                }
                task.updated_at.clone_from(&event.created_at);
                group.updated_at.clone_from(&event.created_at);
            }
            DurableDelegationEventKind::ParentContinuationQueued
            | DurableDelegationEventKind::ParentContinuationPromoted => {
                let continuation = match &event.kind {
                    DurableDelegationEventKind::ParentContinuationQueued => {
                        DelegationParentContinuationStatus::Queued
                    }
                    _ => DelegationParentContinuationStatus::Promoted,
                };
                let group = self.group_mut_or_insert(
                    event,
                    DelegationGroupStatus::Created,
                    DelegationExecution::Foreground,
                );
                group.parent_continuation = continuation;
                group.updated_at.clone_from(&event.created_at);
            }
            DurableDelegationEventKind::Other(_) => {}
        }

        self.groups.sort_by(|left, right| {
            left.status
                .is_terminal()
                .cmp(&right.status.is_terminal())
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.record_event(event);
        true
    }

    fn group_mut_or_insert(
        &mut self,
        event: &DurableDelegationEvent,
        status: DelegationGroupStatus,
        execution: DelegationExecution,
    ) -> &mut DelegationGroupProjection {
        if let Some(index) = self
            .groups
            .iter()
            .position(|group| group.id == event.group_id)
        {
            return &mut self.groups[index];
        }
        self.groups.push(DelegationGroupProjection {
            id: event.group_id.clone(),
            parent_tool_call_id: None,
            status,
            execution,
            parent_continuation: DelegationParentContinuationStatus::NotRequested,
            tasks: Vec::new(),
            updated_at: event.created_at.clone(),
        });
        self.groups.last_mut().expect("inserted delegation group")
    }

    fn record_event(&mut self, event: &DurableDelegationEvent) {
        self.event_cursor = Some(event.id);
        self.events.push(event.clone());
        let excess = self.events.len().saturating_sub(Self::EVENT_HISTORY_LIMIT);
        if excess > 0 {
            self.events.drain(..excess);
        }
    }
}

fn delegation_execution_from_str(value: &str) -> Option<DelegationExecution> {
    match value {
        "foreground" => Some(DelegationExecution::Foreground),
        "detached" => Some(DelegationExecution::Detached),
        _ => None,
    }
}

fn delegation_group_status_from_str(value: &str) -> Option<DelegationGroupStatus> {
    match value {
        "created" => Some(DelegationGroupStatus::Created),
        "queued" => Some(DelegationGroupStatus::Queued),
        "running" => Some(DelegationGroupStatus::Running),
        "ready_for_parent" => Some(DelegationGroupStatus::ReadyForParent),
        "synthesizing" => Some(DelegationGroupStatus::Synthesizing),
        "complete" => Some(DelegationGroupStatus::Complete),
        "degraded" => Some(DelegationGroupStatus::Degraded),
        "failed" => Some(DelegationGroupStatus::Failed),
        "cancelled" => Some(DelegationGroupStatus::Cancelled),
        _ => None,
    }
}

fn delegation_task_status_from_str(value: &str) -> Option<DelegationTaskStatus> {
    match value {
        "created" => Some(DelegationTaskStatus::Created),
        "queued" => Some(DelegationTaskStatus::Queued),
        "leased" => Some(DelegationTaskStatus::Leased),
        "running" => Some(DelegationTaskStatus::Running),
        "retrying" => Some(DelegationTaskStatus::Retrying),
        "complete" => Some(DelegationTaskStatus::Complete),
        "degraded" => Some(DelegationTaskStatus::Degraded),
        "failed" => Some(DelegationTaskStatus::Failed),
        "cancelled" => Some(DelegationTaskStatus::Cancelled),
        _ => None,
    }
}

pub type Result<T> = std::result::Result<T, AgentError>;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("backend not connected")]
    NotConnected,

    #[error("backend is not ready: {0}")]
    NotReady(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("spawn failed: {0}")]
    Spawn(String),

    #[error("request timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("json-rpc error {code}: {message}")]
    Rpc { code: i64, message: String },

    #[error("channel closed")]
    ChannelClosed,

    #[error("protocol: {0}")]
    Protocol(String),

    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("{0}")]
    Other(String),
}

/// High-level connection lifecycle for UI indicators.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Ready,
    /// Offline fixture backend — no live models / no paid API.
    Fixture,
    Error(String),
}

impl ConnectionStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting",
            Self::Ready => "Ready",
            Self::Fixture => "Fixture",
            Self::Error(_) => "Error",
        }
    }

    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Ready | Self::Fixture)
    }
}

/// Coarse item kind extracted from ThreadItem `type` fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    UserMessage,
    AgentMessage,
    Reasoning,
    Plan,
    CommandExecution,
    FileChange,
    McpToolCall,
    DynamicToolCall,
    WebSearch,
    ImageGeneration,
    ImageView,
    CollabAgentToolCall,
    SubAgentActivity,
    ContextCompaction,
    EnteredReviewMode,
    ExitedReviewMode,
    HookPrompt,
    Sleep,
    Other(String),
}

impl ItemKind {
    pub fn from_type_str(s: &str) -> Self {
        match s {
            "userMessage" => Self::UserMessage,
            "agentMessage" => Self::AgentMessage,
            "reasoning" => Self::Reasoning,
            "plan" => Self::Plan,
            "commandExecution" => Self::CommandExecution,
            "fileChange" => Self::FileChange,
            "mcpToolCall" => Self::McpToolCall,
            "dynamicToolCall" => Self::DynamicToolCall,
            "webSearch" => Self::WebSearch,
            "imageGeneration" => Self::ImageGeneration,
            "imageView" => Self::ImageView,
            "collabAgentToolCall" => Self::CollabAgentToolCall,
            "subAgentActivity" => Self::SubAgentActivity,
            "contextCompaction" => Self::ContextCompaction,
            "enteredReviewMode" => Self::EnteredReviewMode,
            "exitedReviewMode" => Self::ExitedReviewMode,
            "hookPrompt" => Self::HookPrompt,
            "sleep" => Self::Sleep,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::UserMessage => "userMessage",
            Self::AgentMessage => "agentMessage",
            Self::Reasoning => "reasoning",
            Self::Plan => "plan",
            Self::CommandExecution => "commandExecution",
            Self::FileChange => "fileChange",
            Self::McpToolCall => "mcpToolCall",
            Self::DynamicToolCall => "dynamicToolCall",
            Self::WebSearch => "webSearch",
            Self::ImageGeneration => "imageGeneration",
            Self::ImageView => "imageView",
            Self::CollabAgentToolCall => "collabAgentToolCall",
            Self::SubAgentActivity => "subAgentActivity",
            Self::ContextCompaction => "contextCompaction",
            Self::EnteredReviewMode => "enteredReviewMode",
            Self::ExitedReviewMode => "exitedReviewMode",
            Self::HookPrompt => "hookPrompt",
            Self::Sleep => "sleep",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Typed turn / item stream events mapped from app-server notifications.
///
/// Used by both live `CodexAppServerBackend` notification dispatch and the
/// offline [`crate::fixture::FixtureBackend`] JSONL replay.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnStreamEvent {
    TurnStarted {
        thread_id: String,
        turn_id: String,
        turn: Option<Value>,
    },
    TurnCompleted {
        thread_id: String,
        turn_id: String,
        status: Option<String>,
        turn: Option<Value>,
    },
    ItemStarted {
        thread_id: String,
        turn_id: String,
        item_id: String,
        kind: ItemKind,
        item: Option<Value>,
    },
    ItemCompleted {
        thread_id: String,
        turn_id: String,
        item_id: String,
        kind: ItemKind,
        /// Best-effort final text for agentMessage / plan / reasoning.
        text: Option<String>,
        item: Option<Value>,
    },
    AgentMessageDelta {
        thread_id: String,
        turn_id: String,
        item_id: String,
        delta: String,
    },
    ReasoningTextDelta {
        thread_id: String,
        turn_id: String,
        item_id: String,
        content_index: Option<i64>,
        delta: String,
    },
    ReasoningSummaryDelta {
        thread_id: String,
        turn_id: String,
        item_id: String,
        summary_index: Option<i64>,
        delta: String,
    },
    PlanDelta {
        thread_id: String,
        turn_id: String,
        item_id: String,
        delta: String,
    },
    /// Streaming stdout/stderr chunk for a `commandExecution` item.
    CommandExecutionOutputDelta {
        thread_id: String,
        turn_id: String,
        item_id: String,
        delta: String,
    },
    /// Deprecated legacy `fileChange` textual output delta (still mapped for fixtures).
    FileChangeOutputDelta {
        thread_id: String,
        turn_id: String,
        item_id: String,
        delta: String,
    },
    /// Structured patch update for a `fileChange` item (`changes: FileUpdateChange[]`).
    FileChangePatchUpdated {
        thread_id: String,
        turn_id: String,
        item_id: String,
        /// Raw `changes` array (path / kind / diff per entry).
        changes: Value,
    },
    /// Server→client approval request (exec / patch). Stream may pause until answered.
    ApprovalRequested(PendingApproval),
    /// Streaming stdout/stderr chunk for a standalone `process/spawn` session.
    ProcessOutputDelta {
        process_handle: String,
        stream: ProcessOutputStream,
        /// Raw base64 payload from the wire.
        delta_base64: String,
        /// Lossy UTF-8 decode of `delta_base64` (UI convenience).
        delta: String,
        cap_reached: bool,
    },
    /// Final exit for a `process/spawn` session.
    ProcessExited {
        process_handle: String,
        exit_code: i32,
        stdout: String,
        stdout_cap_reached: bool,
        stderr: String,
        stderr_cap_reached: bool,
    },
    /// Ephemeral high-frequency task progress. Durable state is represented by
    /// `DelegationEvent` and the session projection returned on hydration.
    DelegatedProgress {
        thread_id: String,
        progress: DelegatedProgressProjection,
    },
    /// Canonical replayable delegation event emitted by the session coordinator.
    DelegationEvent {
        thread_id: String,
        event: DurableDelegationEvent,
    },
    /// Unmapped notification (kept for forward-compat / logging).
    Other {
        method: String,
        params: Option<Value>,
    },
}

impl TurnStreamEvent {
    pub fn method_name(&self) -> &str {
        match self {
            Self::TurnStarted { .. } => "turn/started",
            Self::TurnCompleted { .. } => "turn/completed",
            Self::ItemStarted { .. } => "item/started",
            Self::ItemCompleted { .. } => "item/completed",
            Self::AgentMessageDelta { .. } => "item/agentMessage/delta",
            Self::ReasoningTextDelta { .. } => "item/reasoning/textDelta",
            Self::ReasoningSummaryDelta { .. } => "item/reasoning/summaryTextDelta",
            Self::PlanDelta { .. } => "item/plan/delta",
            Self::CommandExecutionOutputDelta { .. } => "item/commandExecution/outputDelta",
            Self::FileChangeOutputDelta { .. } => "item/fileChange/outputDelta",
            Self::FileChangePatchUpdated { .. } => "item/fileChange/patchUpdated",
            Self::ApprovalRequested(p) => p.method.as_str(),
            Self::ProcessOutputDelta { .. } => "process/outputDelta",
            Self::ProcessExited { .. } => "process/exited",
            Self::DelegatedProgress { .. } => "mitsuro/delegated_progress",
            Self::DelegationEvent { .. } => "mitsuro/delegation_event",
            Self::Other { method, .. } => method.as_str(),
        }
    }

    pub fn thread_id(&self) -> Option<&str> {
        match self {
            Self::TurnStarted { thread_id, .. }
            | Self::TurnCompleted { thread_id, .. }
            | Self::ItemStarted { thread_id, .. }
            | Self::ItemCompleted { thread_id, .. }
            | Self::AgentMessageDelta { thread_id, .. }
            | Self::ReasoningTextDelta { thread_id, .. }
            | Self::ReasoningSummaryDelta { thread_id, .. }
            | Self::PlanDelta { thread_id, .. }
            | Self::CommandExecutionOutputDelta { thread_id, .. }
            | Self::FileChangeOutputDelta { thread_id, .. }
            | Self::FileChangePatchUpdated { thread_id, .. }
            | Self::DelegatedProgress { thread_id, .. }
            | Self::DelegationEvent { thread_id, .. } => Some(thread_id.as_str()),
            Self::ApprovalRequested(p) => p.thread_id.as_deref(),
            Self::ProcessOutputDelta { .. } | Self::ProcessExited { .. } | Self::Other { .. } => {
                None
            }
        }
    }

    /// Client-supplied `processHandle` when this is a process notification.
    pub fn process_handle(&self) -> Option<&str> {
        match self {
            Self::ProcessOutputDelta { process_handle, .. }
            | Self::ProcessExited { process_handle, .. } => Some(process_handle.as_str()),
            _ => None,
        }
    }

    pub fn is_approval(&self) -> bool {
        matches!(self, Self::ApprovalRequested(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegation_variants_keep_session_identity_and_exact_labels() {
        let progress = TurnStreamEvent::DelegatedProgress {
            thread_id: "session-1".to_owned(),
            progress: DelegatedProgressProjection {
                delegated_run_id: "run-1".to_owned(),
                tool_call_id: "tool-1".to_owned(),
                kind: DelegationKind::Build,
                stage: DelegationRunStage::Running,
                parent_session_id: "session-1".to_owned(),
                task_id: "task-1".to_owned(),
                agent_name: "Builder".to_owned(),
                status: DelegationTaskStatus::Retrying,
                tool_count: 2,
                tokens: 10,
                current_action: None,
                completion_summary: None,
                lines_added: 0,
                lines_removed: 0,
                completed_plan_task: None,
            },
        };
        assert_eq!(progress.thread_id(), Some("session-1"));
        assert_eq!(progress.method_name(), "mitsuro/delegated_progress");
        assert_eq!(DelegationTaskStatus::Retrying.label(), "retrying");
        assert_eq!(
            DurableDelegationEventKind::Other("future_event".to_owned()).label(),
            "future_event"
        );
    }

    #[test]
    fn session_projection_reports_active_counts_and_latest_exact_task() {
        let task = |id: &str, status, updated_at: &str| DelegationTaskProjection {
            id: id.to_owned(),
            key: id.to_owned(),
            role: DelegationRole::Explore,
            status,
            attempt_count: 1,
            updated_at: updated_at.to_owned(),
        };
        let projection = SessionDelegationProjection {
            groups: vec![DelegationGroupProjection {
                id: "group-1".to_owned(),
                parent_tool_call_id: None,
                status: DelegationGroupStatus::Running,
                execution: DelegationExecution::Detached,
                parent_continuation: DelegationParentContinuationStatus::Pending,
                tasks: vec![
                    task(
                        "older-complete",
                        DelegationTaskStatus::Complete,
                        "2026-08-08T00:00:01Z",
                    ),
                    task(
                        "latest-retry",
                        DelegationTaskStatus::Retrying,
                        "2026-08-08T00:00:02Z",
                    ),
                ],
                updated_at: "2026-08-08T00:00:02Z".to_owned(),
            }],
            ..Default::default()
        };

        assert_eq!(projection.active_counts(), (1, 1));
        let latest = projection.latest_task().expect("latest task");
        assert_eq!(latest.key, "latest-retry");
        assert_eq!(latest.status.label(), "retrying");
    }

    fn delegation_event(
        id: i64,
        group_id: &str,
        task_id: Option<&str>,
        kind: DurableDelegationEventKind,
        payload: Value,
    ) -> DurableDelegationEvent {
        DurableDelegationEvent {
            id,
            parent_session_id: "session-1".to_owned(),
            group_id: group_id.to_owned(),
            task_id: task_id.map(str::to_owned),
            kind,
            payload,
            created_at: format!("2026-08-08T00:00:{id:02}Z"),
        }
    }

    #[test]
    fn live_events_merge_one_task_without_losing_hydrated_siblings() {
        let task = |id: &str| DelegationTaskProjection {
            id: id.to_owned(),
            key: format!("key-{id}"),
            role: DelegationRole::Build,
            status: DelegationTaskStatus::Queued,
            attempt_count: 0,
            updated_at: "2026-08-08T00:00:00Z".to_owned(),
        };
        let mut projection = SessionDelegationProjection {
            groups: vec![DelegationGroupProjection {
                id: "group-1".to_owned(),
                parent_tool_call_id: Some("tool-1".to_owned()),
                status: DelegationGroupStatus::Running,
                execution: DelegationExecution::Detached,
                parent_continuation: DelegationParentContinuationStatus::Pending,
                tasks: vec![task("task-a"), task("task-b")],
                updated_at: "2026-08-08T00:00:00Z".to_owned(),
            }],
            event_cursor: Some(10),
            ..Default::default()
        };

        assert!(projection.apply_event(&delegation_event(
            11,
            "group-1",
            Some("task-b"),
            DurableDelegationEventKind::TaskRunning,
            serde_json::json!({"attempt_number": 2}),
        )));

        let tasks = &projection.groups[0].tasks;
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].key, "key-task-a");
        assert_eq!(tasks[0].status, DelegationTaskStatus::Queued);
        assert_eq!(tasks[1].key, "key-task-b");
        assert_eq!(tasks[1].status, DelegationTaskStatus::Running);
        assert_eq!(tasks[1].attempt_count, 2);
        assert_eq!(projection.event_cursor, Some(11));
    }

    #[test]
    fn group_created_event_materializes_all_task_keys_and_exact_lifecycle() {
        let mut projection = SessionDelegationProjection::default();
        assert!(projection.apply_event(&delegation_event(
            1,
            "group-live",
            None,
            DurableDelegationEventKind::GroupCreated,
            serde_json::json!({
                "execution_mode": "detached",
                "tasks": [
                    {"delegation_task_id": "task-a", "task_key": "inspect-api"},
                    {"delegation_task_id": "task-b", "task_key": "run-tests"}
                ]
            }),
        )));
        projection.apply_event(&delegation_event(
            2,
            "group-live",
            Some("task-a"),
            DurableDelegationEventKind::TaskClaimed,
            serde_json::json!({"next_attempt_number": 1}),
        ));
        projection.apply_event(&delegation_event(
            3,
            "group-live",
            Some("task-a"),
            DurableDelegationEventKind::TaskStateChanged,
            serde_json::json!({"state": "complete", "attempt_number": 1}),
        ));

        let group = &projection.groups[0];
        assert_eq!(group.execution, DelegationExecution::Detached);
        assert_eq!(group.tasks.len(), 2);
        assert_eq!(group.tasks[0].key, "inspect-api");
        assert_eq!(group.tasks[0].status, DelegationTaskStatus::Complete);
        assert_eq!(group.tasks[0].attempt_count, 1);
        assert_eq!(group.tasks[1].key, "run-tests");
        assert_eq!(group.tasks[1].status, DelegationTaskStatus::Created);
    }

    #[test]
    fn terminal_group_survives_unknown_and_stale_events_unchanged() {
        let mut projection = SessionDelegationProjection {
            groups: vec![DelegationGroupProjection {
                id: "group-terminal".to_owned(),
                parent_tool_call_id: None,
                status: DelegationGroupStatus::Complete,
                execution: DelegationExecution::Foreground,
                parent_continuation: DelegationParentContinuationStatus::Promoted,
                tasks: vec![DelegationTaskProjection {
                    id: "task-done".to_owned(),
                    key: "final-review".to_owned(),
                    role: DelegationRole::Verifier,
                    status: DelegationTaskStatus::Complete,
                    attempt_count: 1,
                    updated_at: "2026-08-08T00:00:20Z".to_owned(),
                }],
                updated_at: "2026-08-08T00:00:20Z".to_owned(),
            }],
            event_cursor: Some(20),
            ..Default::default()
        };
        let before = projection.groups.clone();

        assert!(projection.apply_event(&delegation_event(
            21,
            "group-terminal",
            None,
            DurableDelegationEventKind::Other("future_scheduler_event".to_owned()),
            serde_json::json!({"epoch": 2}),
        )));
        assert_eq!(projection.groups, before);
        assert!(!projection.apply_event(&delegation_event(
            20,
            "group-terminal",
            Some("task-done"),
            DurableDelegationEventKind::TaskRunning,
            serde_json::json!({"attempt_number": 9}),
        )));
        assert_eq!(projection.groups, before);
        assert_eq!(projection.event_cursor, Some(21));
        assert_eq!(projection.events.len(), 1);
    }
}
