//! Runtime trace forwarding at the canonical loop-event boundary.

use std::path::PathBuf;

use tokio::sync::mpsc;
use tracing::warn;

use crate::storage::{Database, RuntimeTraceEvent, RuntimeTraceStore};

use super::loop_events::LoopEvent;

pub(crate) fn new_runtime_trace_run_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) async fn forward_runtime_traces(
    db_path: PathBuf,
    session_id: String,
    run_id: String,
    mut source_rx: mpsc::UnboundedReceiver<LoopEvent>,
    sink_tx: mpsc::UnboundedSender<LoopEvent>,
) {
    let db = match Database::new(&db_path) {
        Ok(db) => Some(db),
        Err(err) => {
            warn!(
                session_id = %session_id,
                error = %err,
                "Failed to open database for runtime trace capture"
            );
            None
        }
    };

    let mut sequence = db
        .as_ref()
        .and_then(|db| RuntimeTraceStore::new(db).next_sequence(&session_id).ok())
        .unwrap_or(1);
    let mut active_turn = 1usize;

    while let Some(event) = source_rx.recv().await {
        if let Some(db) = db.as_ref() {
            let trace_event =
                RuntimeTraceEvent::from_loop_event(run_id.clone(), sequence, active_turn, &event);
            if let Err(err) = RuntimeTraceStore::new(db).append_event(&session_id, &trace_event) {
                warn!(
                    session_id = %session_id,
                    sequence,
                    error = %err,
                    "Failed to persist runtime trace event"
                );
            }
        }

        sequence += 1;

        if sink_tx.send(event.clone()).is_err() {
            break;
        }

        if let LoopEvent::TurnComplete { turn, has_more } = event {
            active_turn = if has_more {
                turn.saturating_add(1)
            } else {
                turn
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    use super::forward_runtime_traces;
    use crate::agent::loop_events::{LoopEvent, LoopStopReason};
    use crate::storage::{Database, RuntimeTraceStore};

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

    #[tokio::test]
    async fn runtime_trace_forwarder_persists_and_forwards_events() {
        let (db, temp_dir, session_id) = create_test_db();
        let db_path = temp_dir.path().join("test.db");

        let (source_tx, source_rx) = mpsc::unbounded_channel();
        let (sink_tx, mut sink_rx) = mpsc::unbounded_channel();

        let forwarder = tokio::spawn(forward_runtime_traces(
            db_path,
            session_id.clone(),
            "run-1".to_string(),
            source_rx,
            sink_tx,
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
}
