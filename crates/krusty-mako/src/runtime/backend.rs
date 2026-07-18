use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use krusty_core::storage::ClaimedMakoRun;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

const MAX_EXECUTION_EVENT_TYPE_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct ExecutionEventSink {
    sender: mpsc::Sender<ExecutionEvent>,
    max_payload_bytes: usize,
}

impl ExecutionEventSink {
    pub(crate) fn new(sender: mpsc::Sender<ExecutionEvent>, max_payload_bytes: usize) -> Self {
        Self {
            sender,
            max_payload_bytes,
        }
    }

    pub async fn emit(&self, mut event: ExecutionEvent) -> Result<(), ExecutionEventSendError> {
        let event_type = event.event_type.trim().to_string();
        if event_type.is_empty() {
            return Err(ExecutionEventSendError::InvalidType);
        }
        if event_type.len() > MAX_EXECUTION_EVENT_TYPE_BYTES {
            return Err(ExecutionEventSendError::TypeTooLong {
                actual: event_type.len(),
                maximum: MAX_EXECUTION_EVENT_TYPE_BYTES,
            });
        }
        if event_type != event.event_type {
            event.event_type = event_type;
        }
        let actual = serde_json::to_vec(&event.payload)
            .map_err(|error| ExecutionEventSendError::Encoding(error.to_string()))?
            .len();
        if actual > self.max_payload_bytes {
            return Err(ExecutionEventSendError::PayloadTooLarge {
                actual,
                maximum: self.max_payload_bytes,
            });
        }
        self.sender
            .send(event)
            .await
            .map_err(|_| ExecutionEventSendError::Closed)
    }

    pub async fn agentic(&self, payload: Value) -> Result<(), ExecutionEventSendError> {
        self.emit(ExecutionEvent {
            event_type: "agentic_event".to_string(),
            payload,
        })
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionEventSendError {
    InvalidType,
    TypeTooLong { actual: usize, maximum: usize },
    PayloadTooLarge { actual: usize, maximum: usize },
    Encoding(String),
    Closed,
}

impl std::fmt::Display for ExecutionEventSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidType => formatter.write_str("execution event type is empty"),
            Self::TypeTooLong { actual, maximum } => write!(
                formatter,
                "execution event type is {actual} bytes; maximum is {maximum}"
            ),
            Self::PayloadTooLarge { actual, maximum } => write!(
                formatter,
                "execution event payload is {actual} bytes; maximum is {maximum}"
            ),
            Self::Encoding(error) => write!(formatter, "execution event encoding failed: {error}"),
            Self::Closed => formatter.write_str("execution event sink is closed"),
        }
    }
}

impl std::error::Error for ExecutionEventSendError {}

#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    pub claim: ClaimedMakoRun,
    pub daemon_instance_id: String,
    pub events: ExecutionEventSink,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ExecutionOutcome {
    Succeeded {
        output: Value,
    },
    Failed {
        error: String,
        retryable: bool,
        retry_after: Option<Duration>,
    },
    Sleeping {
        wake_at: DateTime<Utc>,
        reason: Option<String>,
    },
    AwaitingInput {
        details: Value,
    },
    RecoveryRequired {
        reason: String,
    },
    Cancelled {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "control", rename_all = "snake_case")]
pub enum ExecutionControl {
    Start,
    Message {
        message: String,
        /// The command handler already committed this canonical message and
        /// its episode index. Adapters must inject it into a live loop without
        /// saving a second copy.
        persisted: bool,
    },
    Steer {
        pending_id: Option<String>,
        content: Value,
    },
    ToolApproval {
        tool_call_id: String,
        approved: bool,
    },
    UserResponse {
        tool_call_id: String,
        response: String,
    },
    Cancel {
        reason: String,
    },
}

#[async_trait]
pub trait ExecutionBackend: Send + Sync + 'static {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome;

    async fn control(&self, session_id: &str, control: ExecutionControl) -> anyhow::Result<()>;
}

#[derive(Debug, Default)]
pub struct UnavailableExecutionBackend;

#[async_trait]
impl ExecutionBackend for UnavailableExecutionBackend {
    async fn execute(&self, _request: ExecutionRequest) -> ExecutionOutcome {
        ExecutionOutcome::Failed {
            error: "Mako execution backend is not installed".to_string(),
            retryable: false,
            retry_after: None,
        }
    }

    async fn control(&self, _session_id: &str, _control: ExecutionControl) -> anyhow::Result<()> {
        anyhow::bail!("Mako execution backend is not installed")
    }
}
