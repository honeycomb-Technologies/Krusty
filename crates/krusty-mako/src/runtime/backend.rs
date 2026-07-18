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
    /// Raw event delivered only to authenticated live subscribers. The durable
    /// scheduler must never write this value to SQLite.
    pub payload: Value,
    /// Redacted replay representation. `None` marks a live-only event and must
    /// be published with no durable sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durable_payload: Option<Value>,
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
            event.payload = oversized_payload_summary(&event.event_type, &event.payload, actual);
        }
        if let Some(durable_payload) = event.durable_payload.as_ref() {
            let actual = serde_json::to_vec(durable_payload)
                .map_err(|error| ExecutionEventSendError::Encoding(error.to_string()))?
                .len();
            if actual > self.max_payload_bytes {
                let summary = oversized_payload_summary(&event.event_type, durable_payload, actual);
                event.durable_payload = Some(summary);
            }
        }
        self.sender
            .send(event)
            .await
            .map_err(|_| ExecutionEventSendError::Closed)
    }

    pub async fn agentic(&self, payload: Value) -> Result<(), ExecutionEventSendError> {
        let durable_payload = durable_agentic_payload(&payload);
        let live_payload = live_agentic_payload(payload);
        self.emit(ExecutionEvent {
            event_type: "agentic_event".to_string(),
            payload: live_payload,
            durable_payload,
        })
        .await
    }
}

fn oversized_payload_summary(event_type: &str, payload: &Value, original_bytes: usize) -> Value {
    let payload_kind = match payload {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    let agentic_type = (event_type == "agentic_event")
        .then(|| payload.get("type").and_then(Value::as_str))
        .flatten()
        .filter(|value| value.len() <= MAX_EXECUTION_EVENT_TYPE_BYTES);
    serde_json::json!({
        "type": agentic_type.unwrap_or("oversized_event"),
        "payload_kind": payload_kind,
        "original_bytes": original_bytes,
        "truncated": true,
        "redacted": true,
    })
}

/// Hidden reasoning is neither a durable diagnostic nor a client contract.
/// Suppress raw thinking and provider signatures even on the authenticated
/// live stream while retaining a small progress signal.
fn live_agentic_payload(payload: Value) -> Value {
    match agentic_type(&payload) {
        Some("thinking_delta" | "thinking_complete") => serde_json::json!({
            "type": agentic_type(&payload),
            "chars": payload
                .get("thinking")
                .and_then(Value::as_str)
                .map(str::chars)
                .map(Iterator::count)
                .unwrap_or_default(),
            "redacted": true,
        }),
        _ => payload,
    }
}

/// Produce the allow-listed replay form for one agent event. Free-form model,
/// tool, web, and user content is live-only; lifecycle identities and counters
/// remain replayable. Unknown event kinds persist only their type so future
/// fields cannot silently become a new secret-bearing journal surface.
fn durable_agentic_payload(payload: &Value) -> Option<Value> {
    let event_type = agentic_type(payload)?;
    let get = |key: &str| payload.get(key).cloned().unwrap_or(Value::Null);
    let chars = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::chars)
            .map(Iterator::count)
            .unwrap_or_default()
    };
    let durable = match event_type {
        "text_delta"
        | "text_delta_with_citations"
        | "thinking_delta"
        | "thinking_complete"
        | "tool_output_delta"
        | "web_search_results"
        | "web_fetch_result" => return None,
        "tool_call_start" | "tool_executing" | "server_tool_start" | "server_tool_complete" => {
            serde_json::json!({"type": event_type, "id": get("id"), "name": get("name")})
        }
        "tool_call_complete" | "tool_approval_required" => serde_json::json!({
            "type": event_type,
            "id": get("id"),
            "name": get("name"),
            "arguments": summarize_json_shape(payload.get("arguments")),
            "arguments_redacted": true,
        }),
        "tool_result" => serde_json::json!({
            "type": event_type,
            "id": get("id"),
            "is_error": get("is_error"),
            "output_chars": chars("output"),
            "output_redacted": true,
        }),
        "awaiting_input" => serde_json::json!({
            "type": event_type,
            "tool_call_id": get("tool_call_id"),
            "tool_name": get("tool_name"),
        }),
        "tool_approved" | "tool_denied" => {
            serde_json::json!({"type": event_type, "id": get("id")})
        }
        "steering_injected" => serde_json::json!({
            "type": event_type,
            "pending_id": get("pending_id"),
            "message_chars": chars("message"),
        }),
        "server_tool_error" => serde_json::json!({
            "type": event_type,
            "tool_use_id": get("tool_use_id"),
            "error_code": get("error_code"),
        }),
        "mode_change" => serde_json::json!({
            "type": event_type,
            "mode": get("mode"),
            "reason_chars": chars("reason"),
        }),
        "plan_update" => serde_json::json!({
            "type": event_type,
            "task_count": payload.get("tasks").and_then(Value::as_array).map(Vec::len).unwrap_or_default(),
        }),
        "plan_complete" => serde_json::json!({
            "type": event_type,
            "tool_call_id": get("tool_call_id"),
            "task_count": get("task_count"),
            "title_chars": chars("title"),
        }),
        "agent_sleeping" => serde_json::json!({
            "type": event_type,
            "duration_secs": get("duration_secs"),
            "reason_chars": chars("reason"),
        }),
        "turn_complete" => serde_json::json!({
            "type": event_type,
            "turn": get("turn"),
            "has_more": get("has_more"),
        }),
        "tick_injected" => {
            serde_json::json!({"type": event_type, "tick_number": get("tick_number")})
        }
        "usage" => serde_json::json!({
            "type": event_type,
            "prompt_tokens": get("prompt_tokens"),
            "input_tokens": get("input_tokens"),
            "completion_tokens": get("completion_tokens"),
            "reasoning_tokens": get("reasoning_tokens"),
            "cache_creation_input_tokens": get("cache_creation_input_tokens"),
            "cache_read_input_tokens": get("cache_read_input_tokens"),
            "total_tokens": get("total_tokens"),
        }),
        "session_pinched" => serde_json::json!({
            "type": event_type,
            "source_session_id": get("source_session_id"),
            "new_session_id": get("new_session_id"),
            "estimated_tokens_before": get("estimated_tokens_before"),
            "reason_chars": chars("reason"),
        }),
        "context_compaction_started" => serde_json::json!({
            "type": event_type,
            "reason_chars": chars("reason"),
        }),
        "context_compacted" => serde_json::json!({
            "type": event_type,
            "estimated_tokens_before": get("estimated_tokens_before"),
            "estimated_tokens_after": get("estimated_tokens_after"),
            "replaced_messages": get("replaced_messages"),
            "checkpoint_id": get("checkpoint_id"),
            "compaction_count": get("compaction_count"),
            "reason_chars": chars("reason"),
        }),
        "finish" => serde_json::json!({
            "type": event_type,
            "session_id": get("session_id"),
            "stop_reason": get("stop_reason"),
        }),
        "title_update" | "title_generated" => {
            serde_json::json!({"type": event_type, "title_chars": chars("title")})
        }
        "error" => serde_json::json!({
            "type": event_type,
            "error_chars": chars("error"),
            "error_redacted": true,
        }),
        "delegated_progress" => serde_json::json!({
            "type": event_type,
            "delegated_run_id": get("delegated_run_id"),
            "tool_call_id": get("tool_call_id"),
            "kind": get("kind"),
            "stage": get("stage"),
            "parent_session_id": get("parent_session_id"),
            "task_id": get("task_id"),
            "agent_name": get("agent_name"),
            "status": get("status"),
            "tool_count": get("tool_count"),
            "tokens": get("tokens"),
            "lines_added": get("lines_added"),
            "lines_removed": get("lines_removed"),
        }),
        "agent_background_started" => serde_json::json!({
            "type": event_type,
            "delegated_run_id": get("delegated_run_id"),
            "agent_type": get("agent_type"),
            "description_chars": chars("description"),
        }),
        "agent_background_completed" => serde_json::json!({
            "type": event_type,
            "delegated_run_id": get("delegated_run_id"),
            "agent_type": get("agent_type"),
            "success": get("success"),
            "summary_chars": chars("summary"),
        }),
        "user_message" => serde_json::json!({
            "type": event_type,
            "level": get("level"),
            "title_chars": chars("title"),
            "message_chars": chars("message"),
        }),
        "classifier_decision" => serde_json::json!({
            "type": event_type,
            "tool_name": get("tool_name"),
            "decision": get("decision"),
            "stage": get("stage"),
            "reason_chars": chars("reason"),
        }),
        "teammate_spawned" => serde_json::json!({
            "type": event_type,
            "name": get("name"),
            "role_chars": chars("role"),
        }),
        "teammate_task_completed" => serde_json::json!({
            "type": event_type,
            "name": get("name"),
            "task_id": get("task_id"),
            "result_chars": chars("result"),
        }),
        "teammate_task_failed" => serde_json::json!({
            "type": event_type,
            "name": get("name"),
            "task_id": get("task_id"),
            "error_chars": chars("error"),
        }),
        "teammate_cancelled" => {
            serde_json::json!({"type": event_type, "name": get("name")})
        }
        _ => serde_json::json!({"type": event_type, "redacted": true}),
    };
    Some(durable)
}

fn agentic_type(payload: &Value) -> Option<&str> {
    payload.get("type").and_then(Value::as_str)
}

fn summarize_json_shape(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Object(map)) => {
            serde_json::json!({"type": "object", "field_count": map.len()})
        }
        Some(Value::Array(items)) => serde_json::json!({"type": "array", "len": items.len()}),
        Some(Value::String(_)) => serde_json::json!({"type": "string"}),
        Some(Value::Number(_)) => serde_json::json!({"type": "number"}),
        Some(Value::Bool(_)) => serde_json::json!({"type": "bool"}),
        Some(Value::Null) | None => serde_json::json!({"type": "null"}),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionEventSendError {
    InvalidType,
    TypeTooLong { actual: usize, maximum: usize },
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
        /// Exact durable run that emitted the approval request.
        run_id: String,
        tool_call_id: String,
        approved: bool,
    },
    UserResponse {
        /// Exact durable run that emitted the question.
        run_id: String,
        /// Durable pending-message identity committed by the command handler.
        /// The execution host must preserve this when injecting the response
        /// so canonical promotion remains idempotent across the terminal race.
        pending_id: String,
        tool_call_id: String,
        response: String,
    },
    Cancel {
        reason: String,
    },
    /// Scheduler fencing cancellation for one exact claimed run. A stale
    /// cancellation must not abort a replacement run in the same session.
    CancelRun {
        run_id: String,
        reason: String,
    },
    /// Immediate scheduler-owned abort for one exact run after cooperative
    /// cancellation exceeded its durable grace budget. Backends must not
    /// broaden this to another run in the same session.
    AbortRun {
        run_id: String,
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

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::ExecutionEventSink;

    #[tokio::test]
    async fn oversized_live_payload_is_summarized_without_failing_the_run() {
        let (sender, mut receiver) = mpsc::channel(1);
        let sink = ExecutionEventSink::new(sender, 512);

        sink.agentic(serde_json::json!({
            "type": "tool_result",
            "id": "tool-1",
            "output": "x".repeat(4_096),
            "is_error": false,
        }))
        .await
        .expect("oversized raw payload should be summarized, not rejected");

        let event = receiver.recv().await.expect("summarized event");
        assert_eq!(event.payload["type"], "tool_result");
        assert_eq!(event.payload["truncated"], true);
        assert!(event.payload["original_bytes"].as_u64().unwrap() > 512);
        let durable = event
            .durable_payload
            .expect("tool-result replay summary should remain durable");
        assert_eq!(durable["type"], "tool_result");
        assert_eq!(durable["output_redacted"], true);
        assert_eq!(durable["output_chars"], 4_096);
        assert!(serde_json::to_vec(&event.payload).unwrap().len() < 512);
        assert!(serde_json::to_vec(&durable).unwrap().len() < 512);
    }
}
