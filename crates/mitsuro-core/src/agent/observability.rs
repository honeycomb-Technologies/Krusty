//! Runtime trace forwarding at the canonical loop-event boundary.
//!
//! Presentation delivery must never wait on SQLite. Loop events are forwarded
//! immediately, while a dedicated blocking writer persists compact batches.
//! Provider-call accounting arrives on a private side channel so cumulative
//! live usage snapshots can remain responsive without being mistaken for
//! multiple billed calls.

use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::warn;

use crate::ai::client::SimpleCallResult;
use crate::ai::providers::{ProviderId, ReasoningEffort};
use crate::ai::types::Usage;
use crate::storage::{Database, RuntimeTraceEvent, RuntimeTraceStore};

use super::loop_events::LoopEvent;

const TRACE_WRITE_BATCH_SIZE: usize = 64;
const TRACE_WRITE_FLUSH_INTERVAL: Duration = Duration::from_millis(50);
const MAX_RUNTIME_TRACE_EVENTS_PER_SESSION: usize = 20_000;

/// One completed attempt to call the selected AI provider. This is deliberately
/// private to observability: `LoopEvent::Usage` remains a cumulative live UI
/// snapshot, while this record is the single accounting row used by parity
/// reports and durable telemetry.
#[derive(Debug, Clone)]
pub(crate) struct ProviderCallTrace {
    pub provider_call_id: String,
    pub turn: usize,
    pub call_kind: String,
    pub operation: String,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub outcome: String,
    pub usage: Option<Usage>,
    pub duration_ms: u64,
    pub delegated_run_id: Option<String>,
    pub delegated_task_id: Option<String>,
    pub delegated_turn: Option<usize>,
}

impl ProviderCallTrace {
    pub(crate) fn agent_loop(
        provider_call_id: String,
        turn: usize,
        provider: ProviderId,
        model: &str,
        reasoning_effort: Option<ReasoningEffort>,
        outcome: &str,
        usage: Option<Usage>,
        elapsed: Duration,
    ) -> Self {
        Self {
            provider_call_id,
            turn,
            call_kind: "agent_loop".to_string(),
            operation: "agent_turn".to_string(),
            provider: provider.storage_key().to_string(),
            model: model.to_string(),
            reasoning_effort,
            outcome: outcome.to_string(),
            usage,
            duration_ms: duration_millis(elapsed),
            delegated_run_id: None,
            delegated_task_id: None,
            delegated_turn: None,
        }
    }
}

#[derive(Debug, Clone)]
enum ProviderCallTraceTarget {
    Channel(mpsc::UnboundedSender<ProviderCallTrace>),
    Standalone {
        db_path: PathBuf,
        session_id: String,
        run_id: String,
    },
}

/// Session/run attribution for auxiliary provider requests.
///
/// Orchestrated calls share the run's non-blocking writer. Standalone flows
/// such as manual pinch use a one-shot writer and their own auxiliary run id.
#[derive(Debug, Clone)]
pub struct ProviderCallTraceContext {
    target: ProviderCallTraceTarget,
    turn: usize,
    provider_governor: Option<Arc<super::WorkerProviderCallGovernor>>,
}

/// Content-free terminal classification for provider attempts that do not use
/// the ordinary agent-loop trace channel. Keeping this typed prevents callers
/// from placing provider text or other unbounded detail in the outcome field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCallTraceOutcome {
    Completed,
    Error,
    Cancelled,
    SemanticInvalid,
    UnsafeOutput,
}

impl ProviderCallTraceOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
            Self::SemanticInvalid => "semantic_invalid",
            Self::UnsafeOutput => "unsafe_output",
        }
    }
}

impl ProviderCallTraceContext {
    pub(crate) fn for_run(tx: mpsc::UnboundedSender<ProviderCallTrace>, turn: usize) -> Self {
        Self {
            target: ProviderCallTraceTarget::Channel(tx),
            turn,
            provider_governor: None,
        }
    }

    pub fn standalone(
        db_path: impl Into<PathBuf>,
        session_id: impl Into<String>,
        turn: usize,
    ) -> Self {
        Self::standalone_with_run_id(
            db_path,
            session_id,
            format!("auxiliary-{}", uuid::Uuid::new_v4()),
            turn,
        )
    }

    /// Build a one-shot trace writer attributed to a stable caller-owned run.
    /// Hive uses this for provider calls that intentionally bypass the full
    /// agent loop but still belong to an exact durable run or review claim.
    pub fn standalone_with_run_id(
        db_path: impl Into<PathBuf>,
        session_id: impl Into<String>,
        run_id: impl Into<String>,
        turn: usize,
    ) -> Self {
        Self {
            target: ProviderCallTraceTarget::Standalone {
                db_path: db_path.into(),
                session_id: session_id.into(),
                run_id: run_id.into(),
            },
            turn,
            provider_governor: None,
        }
    }

    /// Attach the exact run-scoped Worker provider capability. Clones passed
    /// into compaction, classifiers, or delegated work inherit this same root
    /// origin and lease; they still need a distinct deterministic child slot.
    pub fn with_provider_governor(
        mut self,
        provider_governor: Option<Arc<super::WorkerProviderCallGovernor>>,
    ) -> Self {
        self.provider_governor = provider_governor;
        self
    }

    pub(crate) fn provider_governor(&self) -> Option<&Arc<super::WorkerProviderCallGovernor>> {
        self.provider_governor.as_ref()
    }

    pub(crate) const fn turn(&self) -> usize {
        self.turn
    }

    /// The stable runtime-trace run id when this context writes directly.
    pub fn run_id(&self) -> Option<&str> {
        match &self.target {
            ProviderCallTraceTarget::Standalone { run_id, .. } => Some(run_id),
            ProviderCallTraceTarget::Channel(_) => None,
        }
    }

    pub async fn record_simple_call(
        &self,
        operation: &str,
        provider: ProviderId,
        model: &str,
        started_at: Instant,
        result: &anyhow::Result<SimpleCallResult>,
    ) -> String {
        let provider_call_id = uuid::Uuid::new_v4().to_string();
        self.record_simple_call_with_id(
            provider_call_id,
            operation,
            provider,
            model,
            started_at,
            result,
        )
        .await
    }

    pub async fn record_simple_call_with_id(
        &self,
        provider_call_id: String,
        operation: &str,
        provider: ProviderId,
        model: &str,
        started_at: Instant,
        result: &anyhow::Result<SimpleCallResult>,
    ) -> String {
        let trace = ProviderCallTrace {
            provider_call_id: provider_call_id.clone(),
            turn: self.turn,
            call_kind: "auxiliary".to_string(),
            operation: operation.to_string(),
            provider: provider.storage_key().to_string(),
            model: model.to_string(),
            reasoning_effort: None,
            outcome: if result.is_ok() { "completed" } else { "error" }.to_string(),
            usage: result
                .as_ref()
                .ok()
                .and_then(|response| response.usage.clone()),
            duration_ms: duration_millis(started_at.elapsed()),
            delegated_run_id: None,
            delegated_task_id: None,
            delegated_turn: None,
        };
        self.emit(trace).await;
        provider_call_id
    }

    /// Record a bounded, content-free streaming or semantic-validation result.
    pub async fn record_bounded_call(
        &self,
        operation: &'static str,
        provider: ProviderId,
        model: &str,
        started_at: Instant,
        outcome: ProviderCallTraceOutcome,
        usage: Option<Usage>,
    ) -> String {
        let provider_call_id = uuid::Uuid::new_v4().to_string();
        self.record_bounded_call_with_id(
            provider_call_id,
            operation,
            provider,
            model,
            started_at,
            outcome,
            usage,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record_bounded_call_with_id(
        &self,
        provider_call_id: String,
        operation: &'static str,
        provider: ProviderId,
        model: &str,
        started_at: Instant,
        outcome: ProviderCallTraceOutcome,
        usage: Option<Usage>,
    ) -> String {
        self.emit(ProviderCallTrace {
            provider_call_id: provider_call_id.clone(),
            turn: self.turn,
            call_kind: "auxiliary".to_string(),
            operation: operation.to_string(),
            provider: provider.storage_key().to_string(),
            model: bounded_trace_text(model, 512),
            reasoning_effort: None,
            outcome: outcome.as_str().to_string(),
            usage,
            duration_ms: duration_millis(started_at.elapsed()),
            delegated_run_id: None,
            delegated_task_id: None,
            delegated_turn: None,
        })
        .await;
        provider_call_id
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn record_delegated_call(
        &self,
        operation: &str,
        provider: ProviderId,
        model: &str,
        reasoning_effort: Option<ReasoningEffort>,
        delegated_run_id: Option<&str>,
        delegated_task_id: &str,
        delegated_turn: usize,
        started_at: Instant,
        outcome: &str,
        usage: Option<Usage>,
    ) {
        self.record_delegated_call_with_id(
            uuid::Uuid::new_v4().to_string(),
            operation,
            provider,
            model,
            reasoning_effort,
            delegated_run_id,
            delegated_task_id,
            delegated_turn,
            started_at,
            outcome,
            usage,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn record_delegated_call_with_id(
        &self,
        provider_call_id: String,
        operation: &str,
        provider: ProviderId,
        model: &str,
        reasoning_effort: Option<ReasoningEffort>,
        delegated_run_id: Option<&str>,
        delegated_task_id: &str,
        delegated_turn: usize,
        started_at: Instant,
        outcome: &str,
        usage: Option<Usage>,
    ) {
        self.emit(ProviderCallTrace {
            provider_call_id,
            turn: self.turn,
            call_kind: "delegated".to_string(),
            operation: operation.to_string(),
            provider: provider.storage_key().to_string(),
            model: model.to_string(),
            reasoning_effort,
            outcome: outcome.to_string(),
            usage,
            duration_ms: duration_millis(started_at.elapsed()),
            delegated_run_id: delegated_run_id.map(ToOwned::to_owned),
            delegated_task_id: Some(delegated_task_id.to_string()),
            delegated_turn: Some(delegated_turn),
        })
        .await;
    }

    async fn emit(&self, trace: ProviderCallTrace) {
        match &self.target {
            ProviderCallTraceTarget::Channel(tx) => {
                let _ = tx.send(trace);
            }
            ProviderCallTraceTarget::Standalone {
                db_path,
                session_id,
                run_id,
            } => {
                let db_path = db_path.clone();
                let session_id = session_id.clone();
                let event = provider_call_trace_event(run_id.clone(), trace);
                match tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    let db = Database::new(&db_path)?;
                    RuntimeTraceStore::new(&db)
                        .append_event_with_next_sequence(&session_id, &event)?;
                    Ok(())
                })
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => warn!(%error, "Failed to persist auxiliary provider call"),
                    Err(error) => warn!(%error, "Auxiliary provider-call writer task failed"),
                }
            }
        }
    }
}

fn bounded_trace_text(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn new_runtime_trace_run_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) async fn forward_runtime_traces(
    db_path: PathBuf,
    session_id: String,
    run_id: String,
    mut source_rx: mpsc::UnboundedReceiver<LoopEvent>,
    mut provider_call_rx: mpsc::UnboundedReceiver<ProviderCallTrace>,
    sink_tx: mpsc::UnboundedSender<LoopEvent>,
    extension_dispatch: Option<(
        std::sync::Arc<crate::extensions::AgentExtensionManager>,
        crate::extensions::ExtensionCallContext,
    )>,
) {
    let (writer_tx, writer_rx) = std_mpsc::channel();
    let writer_db_path = db_path.clone();
    let writer_session_id = session_id.clone();
    let writer = tokio::task::spawn_blocking(move || {
        write_runtime_trace_batches(writer_db_path, writer_session_id, writer_rx);
    });
    let mut active_turn = 1usize;
    let mut source_open = true;
    let mut provider_calls_open = true;
    let mut sink_open = true;

    while source_open || provider_calls_open {
        tokio::select! {
            biased;
            event = source_rx.recv(), if source_open => {
                let Some(event) = event else {
                    source_open = false;
                    continue;
                };

                // UI/SSE delivery comes first and never waits for trace I/O.
                if sink_open && sink_tx.send(event.clone()).is_err() {
                    sink_open = false;
                }

                if let Some((manager, context)) = extension_dispatch.as_ref() {
                    if manager.observes_loop_event(&event) {
                        let manager = manager.clone();
                        let context = context.clone();
                        let extension_event = event.clone();
                        tokio::spawn(async move {
                            manager.dispatch_event(extension_event, context).await;
                        });
                    }
                }

                let mut trace_event =
                    RuntimeTraceEvent::from_loop_event(run_id.clone(), 0, active_turn, &event);
                if matches!(event, LoopEvent::Usage { .. }) {
                    trace_event.event_type = "usage_snapshot".to_string();
                    if let Some(payload) = trace_event.payload.as_object_mut() {
                        payload.insert("final_snapshot".to_string(), serde_json::Value::Bool(false));
                    }
                }
                let _ = writer_tx.send(trace_event);

                if let LoopEvent::TurnComplete { turn, has_more } = event {
                    active_turn = if has_more {
                        turn.saturating_add(1)
                    } else {
                        turn
                    };
                }
            }
            provider_call = provider_call_rx.recv(), if provider_calls_open => {
                let Some(provider_call) = provider_call else {
                    provider_calls_open = false;
                    continue;
                };
                let trace_event = provider_call_trace_event(run_id.clone(), provider_call);
                let _ = writer_tx.send(trace_event);
            }
        }
    }

    drop(writer_tx);
    if let Err(error) = writer.await {
        warn!(
            session_id = %session_id,
            error = %error,
            "Runtime trace writer task failed"
        );
    }
}

fn provider_call_trace_event(
    run_id: String,
    provider_call: ProviderCallTrace,
) -> RuntimeTraceEvent {
    let usage = provider_call.usage.as_ref();
    RuntimeTraceEvent {
        run_id,
        sequence: 0,
        turn: provider_call.turn,
        event_type: "provider_call".to_string(),
        call_kind: Some(provider_call.call_kind.clone()),
        operation: Some(provider_call.operation.clone()),
        payload: serde_json::json!({
            "provider_call_id": provider_call.provider_call_id,
            "call_kind": provider_call.call_kind,
            "operation": provider_call.operation,
            "provider": provider_call.provider,
            "model": provider_call.model,
            "reasoning_effort": provider_call.reasoning_effort,
            "final_snapshot": true,
            "outcome": provider_call.outcome,
            "duration_ms": provider_call.duration_ms,
            "usage_available": usage.is_some(),
            "prompt_tokens": usage.map(|usage| usage.prompt_tokens),
            "input_tokens": usage.map(Usage::input_tokens),
            "completion_tokens": usage.map(|usage| usage.completion_tokens),
            "reasoning_tokens": usage.map(|usage| usage.reasoning_tokens),
            "cache_creation_input_tokens": usage.map(|usage| usage.cache_creation_input_tokens),
            "cache_read_input_tokens": usage.map(|usage| usage.cache_read_input_tokens),
            "total_tokens": usage.map(Usage::logical_total_tokens),
            "delegated_run_id": provider_call.delegated_run_id,
            "delegated_task_id": provider_call.delegated_task_id,
            "delegated_turn": provider_call.delegated_turn,
        }),
        failure_category: None,
        stop_reason: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn write_runtime_trace_batches(
    db_path: PathBuf,
    session_id: String,
    receiver: std_mpsc::Receiver<RuntimeTraceEvent>,
) {
    let db = match Database::new(&db_path) {
        Ok(db) => db,
        Err(error) => {
            warn!(
                session_id = %session_id,
                error = %error,
                "Failed to open database for runtime trace capture"
            );
            return;
        }
    };
    let store = RuntimeTraceStore::new(&db);
    let mut batch = Vec::with_capacity(TRACE_WRITE_BATCH_SIZE);
    let mut received_since_flush = 0usize;

    loop {
        let mut disconnected = false;
        match receiver.recv_timeout(TRACE_WRITE_FLUSH_INTERVAL) {
            Ok(event) => {
                received_since_flush += 1;
                if !batch
                    .last_mut()
                    .is_some_and(|previous| coalesce_adjacent_delta(previous, &event))
                {
                    batch.push(event);
                }
                if received_since_flush < TRACE_WRITE_BATCH_SIZE {
                    continue;
                }
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) if batch.is_empty() => continue,
            Err(std_mpsc::RecvTimeoutError::Timeout) => {}
            Err(std_mpsc::RecvTimeoutError::Disconnected) if batch.is_empty() => break,
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                disconnected = true;
            }
        }

        if let Err(error) = store.append_events_with_next_sequences(&session_id, &batch) {
            warn!(
                session_id = %session_id,
                event_count = batch.len(),
                error = %error,
                "Failed to persist runtime trace batch"
            );
        }
        batch.clear();
        received_since_flush = 0;

        if disconnected {
            break;
        }
    }

    if let Err(error) =
        store.prune_session_to_latest(&session_id, MAX_RUNTIME_TRACE_EVENTS_PER_SESSION)
    {
        warn!(
            session_id = %session_id,
            error = %error,
            "Failed to apply runtime trace retention limit"
        );
    }
}

fn coalesce_adjacent_delta(previous: &mut RuntimeTraceEvent, next: &RuntimeTraceEvent) -> bool {
    if previous.run_id != next.run_id
        || previous.turn != next.turn
        || previous.event_type != next.event_type
        || !matches!(
            previous.event_type.as_str(),
            "text_delta" | "thinking_delta" | "tool_output_delta"
        )
    {
        return false;
    }
    if previous.event_type == "tool_output_delta"
        && previous.payload.get("id") != next.payload.get("id")
    {
        return false;
    }

    let Some(previous_chars) = previous
        .payload
        .get("chars")
        .and_then(|value| value.as_u64())
    else {
        return false;
    };
    let Some(next_chars) = next.payload.get("chars").and_then(|value| value.as_u64()) else {
        return false;
    };
    let Some(payload) = previous.payload.as_object_mut() else {
        return false;
    };
    payload.insert(
        "chars".to_string(),
        serde_json::Value::from(previous_chars.saturating_add(next_chars)),
    );
    true
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use tempfile::TempDir;
    use tokio::sync::mpsc;

    use super::{
        coalesce_adjacent_delta, forward_runtime_traces, ProviderCallTrace,
        ProviderCallTraceContext, ProviderCallTraceOutcome,
    };
    use crate::agent::loop_events::{LoopEvent, LoopStopReason};
    use crate::ai::client::SimpleCallResult;
    use crate::ai::providers::{ProviderId, ReasoningEffort};
    use crate::ai::types::Usage;
    use crate::storage::{Database, RuntimeTraceEvent, RuntimeTraceStore};

    fn create_test_db() -> (Database, TempDir, String) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).expect("Failed to create database");
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session_id, "Forwarder Test", now, now],
            )
            .expect("Failed to create session");
        (db, temp_dir, session_id)
    }

    #[test]
    fn adjacent_trace_deltas_coalesce_without_crossing_tool_boundaries() {
        let mut first = RuntimeTraceEvent::from_loop_event(
            "run-1",
            0,
            1,
            &LoopEvent::TextDelta {
                delta: "ab".to_string(),
            },
        );
        let second = RuntimeTraceEvent::from_loop_event(
            "run-1",
            0,
            1,
            &LoopEvent::TextDelta {
                delta: "cde".to_string(),
            },
        );
        assert!(coalesce_adjacent_delta(&mut first, &second));
        assert_eq!(first.payload["chars"], 5);

        let other_turn = RuntimeTraceEvent::from_loop_event(
            "run-1",
            0,
            2,
            &LoopEvent::TextDelta {
                delta: "ignored".to_string(),
            },
        );
        assert!(!coalesce_adjacent_delta(&mut first, &other_turn));
        assert_eq!(first.payload["chars"], 5);
    }

    #[tokio::test]
    async fn caller_owned_provider_call_id_is_reused_by_trace_projection() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let context = ProviderCallTraceContext::for_run(tx, 4);
        let result: anyhow::Result<SimpleCallResult> = Ok(SimpleCallResult {
            text: "ok".to_string(),
            usage: None,
        });

        let returned = context
            .record_simple_call_with_id(
                "worker-call-ledger-id".to_string(),
                "compaction_summary",
                ProviderId::Grok,
                "grok-test",
                Instant::now(),
                &result,
            )
            .await;

        assert_eq!(returned, "worker-call-ledger-id");
        let trace = rx.recv().await.expect("trace should be emitted");
        assert_eq!(trace.provider_call_id, "worker-call-ledger-id");
        assert_eq!(trace.turn, 4);
    }

    #[tokio::test]
    async fn runtime_trace_forwarder_persists_and_forwards_events() {
        let (db, temp_dir, session_id) = create_test_db();
        let db_path = temp_dir.path().join("test.db");

        let (source_tx, source_rx) = mpsc::unbounded_channel();
        let (provider_call_tx, provider_call_rx) = mpsc::unbounded_channel();
        let (sink_tx, mut sink_rx) = mpsc::unbounded_channel();

        let forwarder = tokio::spawn(forward_runtime_traces(
            db_path,
            session_id.clone(),
            "run-1".to_string(),
            source_rx,
            provider_call_rx,
            sink_tx,
            None,
        ));

        source_tx
            .send(LoopEvent::ToolCallStart {
                id: "tool-1".to_string(),
                name: "grep".to_string(),
            })
            .expect("send should succeed");
        source_tx
            .send(LoopEvent::TurnComplete {
                turn: 1,
                has_more: false,
            })
            .expect("send should succeed");
        source_tx
            .send(LoopEvent::Finished {
                session_id: session_id.clone(),
                stop_reason: LoopStopReason::Completed,
            })
            .expect("send should succeed");
        drop(source_tx);
        drop(provider_call_tx);

        forwarder.await.expect("forwarder task should exit cleanly");

        let forwarded = sink_rx.recv().await.expect("first event should forward");
        assert!(matches!(forwarded, LoopEvent::ToolCallStart { .. }));

        let summary = RuntimeTraceStore::new(&db)
            .summarize_session(&session_id)
            .expect("summary should succeed");
        assert_eq!(summary.total_events, 3);
        assert_eq!(summary.total_runs, 1);
        assert_eq!(summary.total_turns, 1);
        assert_eq!(summary.last_stop_reason, Some(LoopStopReason::Completed));
    }

    #[tokio::test]
    async fn live_usage_snapshots_are_forwarded_but_one_provider_call_is_accounted() {
        let (db, temp_dir, session_id) = create_test_db();
        let db_path = temp_dir.path().join("test.db");
        let (source_tx, source_rx) = mpsc::unbounded_channel();
        let (provider_call_tx, provider_call_rx) = mpsc::unbounded_channel();
        let (sink_tx, mut sink_rx) = mpsc::unbounded_channel();

        let forwarder = tokio::spawn(forward_runtime_traces(
            db_path,
            session_id.clone(),
            "run-usage".to_string(),
            source_rx,
            provider_call_rx,
            sink_tx,
            None,
        ));

        let input_snapshot = LoopEvent::Usage {
            prompt_tokens: 100,
            input_tokens: 1_000,
            completion_tokens: 0,
            reasoning_tokens: 0,
            cache_creation_input_tokens: 200,
            cache_read_input_tokens: 700,
            total_tokens: 1_000,
        };
        let final_snapshot = LoopEvent::Usage {
            prompt_tokens: 100,
            input_tokens: 1_000,
            completion_tokens: 50,
            reasoning_tokens: 40,
            cache_creation_input_tokens: 200,
            cache_read_input_tokens: 700,
            total_tokens: 1_050,
        };
        source_tx.send(input_snapshot).expect("input usage send");
        source_tx.send(final_snapshot).expect("final usage send");
        provider_call_tx
            .send(ProviderCallTrace {
                provider_call_id: "call-1".to_string(),
                turn: 1,
                call_kind: "agent_loop".to_string(),
                operation: "agent_turn".to_string(),
                provider: "anthropic".to_string(),
                model: "claude-test".to_string(),
                reasoning_effort: None,
                outcome: "completed".to_string(),
                usage: Some(Usage {
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    reasoning_tokens: 40,
                    total_tokens: 1_050,
                    cache_creation_input_tokens: 200,
                    cache_read_input_tokens: 700,
                }),
                duration_ms: 25,
                delegated_run_id: None,
                delegated_task_id: None,
                delegated_turn: None,
            })
            .expect("provider call send");
        source_tx
            .send(LoopEvent::Finished {
                session_id: session_id.clone(),
                stop_reason: LoopStopReason::Completed,
            })
            .expect("finish send");
        drop(source_tx);
        drop(provider_call_tx);

        forwarder.await.expect("forwarder task should exit cleanly");

        assert!(matches!(
            sink_rx.recv().await,
            Some(LoopEvent::Usage {
                total_tokens: 1_000,
                ..
            })
        ));
        assert!(matches!(
            sink_rx.recv().await,
            Some(LoopEvent::Usage {
                total_tokens: 1_050,
                ..
            })
        ));

        let events = RuntimeTraceStore::new(&db)
            .list_events(&session_id, None)
            .expect("traces should load");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "usage_snapshot")
                .count(),
            2
        );
        let provider_calls = events
            .iter()
            .filter(|event| event.event_type == "provider_call")
            .collect::<Vec<_>>();
        assert_eq!(provider_calls.len(), 1);
        assert_eq!(provider_calls[0].payload["provider_call_id"], "call-1");
        assert_eq!(provider_calls[0].call_kind.as_deref(), Some("agent_loop"));
        assert_eq!(provider_calls[0].operation.as_deref(), Some("agent_turn"));
        assert_eq!(provider_calls[0].payload["final_snapshot"], true);
        assert_eq!(provider_calls[0].payload["total_tokens"], 1_050);
        assert_eq!(provider_calls[0].payload["completion_tokens"], 50);
    }

    #[tokio::test]
    async fn standalone_auxiliary_calls_persist_operation_and_optional_usage() {
        let (db, temp_dir, session_id) = create_test_db();
        let context = ProviderCallTraceContext::standalone(
            temp_dir.path().join("test.db"),
            session_id.clone(),
            1,
        );
        let measured: anyhow::Result<SimpleCallResult> = Ok(SimpleCallResult {
            text: "summary".to_string(),
            usage: Some(Usage {
                prompt_tokens: 80,
                completion_tokens: 20,
                reasoning_tokens: 5,
                total_tokens: 100,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            }),
        });
        context
            .record_simple_call(
                "compaction_summary",
                ProviderId::Anthropic,
                "claude-test",
                Instant::now(),
                &measured,
            )
            .await;

        let omitted: anyhow::Result<SimpleCallResult> = Ok(SimpleCallResult {
            text: "allow".to_string(),
            usage: None,
        });
        context
            .record_simple_call(
                "autonomy_classifier_fast",
                ProviderId::OpenRouter,
                "compatible-model",
                Instant::now(),
                &omitted,
            )
            .await;

        context
            .record_delegated_call(
                "delegated_agent_turn",
                ProviderId::Grok,
                "grok-4.5",
                Some(ReasoningEffort::High),
                Some("delegated-run-1"),
                "builder-1",
                3,
                Instant::now(),
                "completed",
                Some(Usage {
                    prompt_tokens: 1_000,
                    completion_tokens: 100,
                    reasoning_tokens: 25,
                    total_tokens: 1_100,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 900,
                }),
            )
            .await;
        let events = RuntimeTraceStore::new(&db)
            .list_events(&session_id, None)
            .expect("traces should load");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].call_kind.as_deref(), Some("auxiliary"));
        assert_eq!(events[0].operation.as_deref(), Some("compaction_summary"));
        assert_eq!(events[0].payload["usage_available"], true);
        assert_eq!(events[0].payload["total_tokens"], 100);
        assert_eq!(
            events[1].operation.as_deref(),
            Some("autonomy_classifier_fast")
        );
        assert_eq!(events[1].payload["usage_available"], false);
        assert!(events[1].payload["total_tokens"].is_null());
        assert_eq!(events[2].call_kind.as_deref(), Some("delegated"));
        assert_eq!(events[2].payload["provider"], "grok");
        assert_eq!(events[2].payload["delegated_run_id"], "delegated-run-1");
        assert_eq!(events[2].payload["delegated_task_id"], "builder-1");
        assert_eq!(events[2].payload["delegated_turn"], 3);
        assert_eq!(events[2].payload["cache_read_input_tokens"], 900);
        assert_eq!(events[2].payload["input_tokens"], 1_900);
    }

    #[tokio::test]
    async fn stable_run_context_records_each_content_free_provider_attempt() {
        let (db, temp_dir, session_id) = create_test_db();
        let context = ProviderCallTraceContext::standalone_with_run_id(
            temp_dir.path().join("test.db"),
            session_id.clone(),
            "hive-run-42",
            1,
        );
        assert_eq!(context.run_id(), Some("hive-run-42"));
        let first_id = context
            .record_bounded_call(
                "hive_worker_introduction_opening",
                ProviderId::Grok,
                &"m".repeat(700),
                Instant::now(),
                ProviderCallTraceOutcome::SemanticInvalid,
                Some(Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    reasoning_tokens: 0,
                    total_tokens: 15,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                }),
            )
            .await;
        let second_id = context
            .record_bounded_call(
                "hive_worker_introduction_opening",
                ProviderId::Grok,
                "grok-test",
                Instant::now(),
                ProviderCallTraceOutcome::Cancelled,
                None,
            )
            .await;
        assert_ne!(first_id, second_id);

        let events = RuntimeTraceStore::new(&db)
            .list_events(&session_id, None)
            .expect("provider traces");
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| event.run_id == "hive-run-42"));
        assert_eq!(events[0].payload["provider_call_id"], first_id);
        assert_eq!(events[0].payload["outcome"], "semantic_invalid");
        assert_eq!(events[0].payload["model"].as_str().unwrap().len(), 512);
        assert_eq!(events[0].payload["total_tokens"], 15);
        assert_eq!(events[1].payload["provider_call_id"], second_id);
        assert_eq!(events[1].payload["outcome"], "cancelled");
        assert_eq!(events[1].payload["usage_available"], false);
    }
}
