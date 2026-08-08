//! Shared error, status, and turn-stream event types for agent backends.

use serde_json::Value;
use thiserror::Error;

use crate::approvals::PendingApproval;
use crate::process::ProcessOutputStream;

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
            | Self::FileChangePatchUpdated { thread_id, .. } => Some(thread_id.as_str()),
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
