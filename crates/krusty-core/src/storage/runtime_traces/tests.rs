use rusqlite::params;
use tempfile::TempDir;

use super::{ReplayExpectations, RuntimeTraceEvent, RuntimeTraceStore, TraceFailureCategory};
use crate::agent::loop_events::{LoopEvent, LoopStopReason};
use crate::storage::database::Database;

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
            params![session_id, "Trace Test", now, now],
        )
        .expect("Failed to create session");
    (db, temp_dir, session_id)
}

#[test]
fn runtime_trace_store_round_trip() {
    let (db, _temp_dir, session_id) = create_test_db();
    let store = RuntimeTraceStore::new(&db);

    let event = RuntimeTraceEvent::from_loop_event(
        "run-1",
        1,
        1,
        &LoopEvent::ToolCallComplete {
            id: "tool-1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({ "path": "src/main.rs" }),
        },
    );
    store
        .append_event(&session_id, &event)
        .expect("trace append should succeed");

    let events = store
        .list_events(&session_id, None)
        .expect("trace list should succeed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "tool_call_complete");
    assert_eq!(events[0].payload["arguments"]["type"], "object");
}

#[test]
fn runtime_trace_store_limit_returns_most_recent_events_in_order() {
    let (db, _temp_dir, session_id) = create_test_db();
    let store = RuntimeTraceStore::new(&db);

    for sequence in 1..=3 {
        let event = RuntimeTraceEvent::from_loop_event(
            "run-1",
            sequence,
            1,
            &LoopEvent::TurnComplete {
                turn: sequence as usize,
                has_more: sequence < 3,
            },
        );
        store
            .append_event(&session_id, &event)
            .expect("trace append should succeed");
    }

    let events = store
        .list_events(&session_id, Some(2))
        .expect("trace list should succeed");
    let sequences: Vec<i64> = events.iter().map(|event| event.sequence).collect();
    assert_eq!(sequences, vec![2, 3]);
}

#[test]
fn runtime_trace_summary_classifies_failures_and_pinches() {
    let (db, _temp_dir, session_id) = create_test_db();
    let store = RuntimeTraceStore::new(&db);

    let traces = [
        RuntimeTraceEvent::from_loop_event(
            "run-1",
            1,
            1,
            &LoopEvent::SessionPinched {
                reason: "pressure".to_string(),
                source_session_id: session_id.clone(),
                new_session_id: "child-session".to_string(),
                estimated_tokens_before: 100_000,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-1",
            2,
            1,
            &LoopEvent::ToolResult {
                id: "tool-1".to_string(),
                output: "permission denied".to_string(),
                is_error: true,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-1",
            3,
            1,
            &LoopEvent::Finished {
                session_id: session_id.clone(),
                stop_reason: LoopStopReason::ProviderError,
            },
        ),
    ];

    for event in traces {
        store
            .append_event(&session_id, &event)
            .expect("trace append should succeed");
    }

    let summary = store
        .summarize_session(&session_id)
        .expect("summary should succeed");
    assert_eq!(summary.total_events, 3);
    assert_eq!(summary.total_runs, 1);
    assert_eq!(summary.total_turns, 1);
    assert_eq!(summary.tool_errors, 1);
    assert_eq!(summary.provider_failures, 1);
    assert_eq!(summary.session_pinches, 1);
    assert!(summary
        .failure_categories
        .contains(&TraceFailureCategory::ToolExecutionError));
    assert!(summary
        .failure_categories
        .contains(&TraceFailureCategory::ProviderError));
}

#[test]
fn replay_gate_accepts_long_session_workload_with_pinch() {
    let (db, _temp_dir, session_id) = create_test_db();
    let store = RuntimeTraceStore::new(&db);

    let traces = [
        RuntimeTraceEvent::from_loop_event(
            "run-long",
            1,
            1,
            &LoopEvent::ToolCallComplete {
                id: "tool-1".to_string(),
                name: "read".to_string(),
                arguments: serde_json::json!({ "file_path": "src/main.rs" }),
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-long",
            2,
            1,
            &LoopEvent::ToolResult {
                id: "tool-1".to_string(),
                output: "read ok".to_string(),
                is_error: false,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-long",
            3,
            1,
            &LoopEvent::TurnComplete {
                turn: 1,
                has_more: true,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-long",
            4,
            2,
            &LoopEvent::SessionPinched {
                reason: "context_pressure".to_string(),
                source_session_id: session_id.clone(),
                new_session_id: "child-session".to_string(),
                estimated_tokens_before: 140_000,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-long",
            5,
            2,
            &LoopEvent::ToolCallComplete {
                id: "tool-2".to_string(),
                name: "write".to_string(),
                arguments: serde_json::json!({ "file_path": "src/lib.rs" }),
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-long",
            6,
            2,
            &LoopEvent::ToolResult {
                id: "tool-2".to_string(),
                output: "write ok".to_string(),
                is_error: false,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-long",
            7,
            2,
            &LoopEvent::TurnComplete {
                turn: 2,
                has_more: true,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-long",
            8,
            3,
            &LoopEvent::Finished {
                session_id: session_id.clone(),
                stop_reason: LoopStopReason::Completed,
            },
        ),
    ];

    for event in traces {
        store
            .append_event(&session_id, &event)
            .expect("trace append should succeed");
    }

    let summary = store
        .summarize_session(&session_id)
        .expect("summary should succeed");
    let expectations = ReplayExpectations {
        min_total_turns: 3,
        min_session_pinches: 1,
        required_event_types: vec![
            "tool_call_complete".to_string(),
            "tool_result".to_string(),
            "session_pinched".to_string(),
        ],
        ..ReplayExpectations::strict()
    };

    let result = expectations.evaluate(&summary);
    assert!(
        result.passed,
        "unexpected violations: {:?}",
        result.violations
    );
    assert_eq!(summary.session_pinches, 1);
    assert_eq!(summary.tool_calls, 2);
}

#[test]
fn replay_gate_accepts_approval_pause_and_resume_workload() {
    let (db, _temp_dir, session_id) = create_test_db();
    let store = RuntimeTraceStore::new(&db);

    let traces = [
        RuntimeTraceEvent::from_loop_event(
            "run-awaiting-input",
            1,
            1,
            &LoopEvent::ToolApprovalRequired {
                id: "tool-1".to_string(),
                name: "write".to_string(),
                arguments: serde_json::json!({ "file_path": "src/main.rs" }),
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-awaiting-input",
            2,
            1,
            &LoopEvent::AwaitingInput {
                tool_call_id: "tool-1".to_string(),
                tool_name: "write".to_string(),
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-awaiting-input",
            3,
            1,
            &LoopEvent::Finished {
                session_id: session_id.clone(),
                stop_reason: LoopStopReason::AwaitingInput,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-approved",
            4,
            2,
            &LoopEvent::ToolApproved {
                id: "tool-1".to_string(),
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-approved",
            5,
            2,
            &LoopEvent::ToolExecuting {
                id: "tool-1".to_string(),
                name: "write".to_string(),
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-approved",
            6,
            2,
            &LoopEvent::ToolResult {
                id: "tool-1".to_string(),
                output: "write ok".to_string(),
                is_error: false,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-approved",
            7,
            2,
            &LoopEvent::Finished {
                session_id: session_id.clone(),
                stop_reason: LoopStopReason::Completed,
            },
        ),
    ];

    for event in traces {
        store
            .append_event(&session_id, &event)
            .expect("trace append should succeed");
    }

    let summary = store
        .summarize_session(&session_id)
        .expect("summary should succeed");
    let expectations = ReplayExpectations {
        min_total_runs: 2,
        min_total_turns: 2,
        min_awaiting_input_events: 1,
        required_event_types: vec![
            "tool_approval_required".to_string(),
            "awaiting_input".to_string(),
            "tool_approved".to_string(),
            "tool_result".to_string(),
        ],
        ..ReplayExpectations::strict()
    };

    let result = expectations.evaluate(&summary);
    assert!(
        result.passed,
        "unexpected violations: {:?}",
        result.violations
    );
    assert_eq!(summary.awaiting_input_events, 1);
    assert_eq!(summary.total_runs, 2);
}

#[test]
fn summarize_latest_run_ignores_prior_provider_interruption_after_recovery() {
    let (db, _temp_dir, session_id) = create_test_db();
    let store = RuntimeTraceStore::new(&db);

    let traces = [
        RuntimeTraceEvent::from_loop_event(
            "run-interrupted",
            1,
            1,
            &LoopEvent::Error {
                error: "provider disconnected".to_string(),
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-interrupted",
            2,
            1,
            &LoopEvent::Finished {
                session_id: session_id.clone(),
                stop_reason: LoopStopReason::ProviderError,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-recovered",
            3,
            2,
            &LoopEvent::ToolCallComplete {
                id: "tool-2".to_string(),
                name: "read".to_string(),
                arguments: serde_json::json!({ "file_path": "src/lib.rs" }),
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-recovered",
            4,
            2,
            &LoopEvent::ToolResult {
                id: "tool-2".to_string(),
                output: "read ok".to_string(),
                is_error: false,
            },
        ),
        RuntimeTraceEvent::from_loop_event(
            "run-recovered",
            5,
            2,
            &LoopEvent::Finished {
                session_id: session_id.clone(),
                stop_reason: LoopStopReason::Completed,
            },
        ),
    ];

    for event in traces {
        store
            .append_event(&session_id, &event)
            .expect("trace append should succeed");
    }

    let whole_session = store
        .summarize_session(&session_id)
        .expect("summary should succeed");
    assert_eq!(whole_session.provider_failures, 1);

    let latest_run = store
        .summarize_latest_run(&session_id)
        .expect("latest run summary should succeed");
    assert_eq!(latest_run.total_runs, 1);
    assert_eq!(latest_run.provider_failures, 0);
    assert_eq!(latest_run.last_stop_reason, Some(LoopStopReason::Completed));

    let result = ReplayExpectations::strict().evaluate(&latest_run);
    assert!(
        result.passed,
        "unexpected violations: {:?}",
        result.violations
    );
}

#[test]
fn replay_gate_rejects_loop_guard_workload() {
    let failing_summary = super::RuntimeTraceSummary {
        total_events: 3,
        total_runs: 1,
        total_turns: 4,
        last_stop_reason: Some(LoopStopReason::LoopGuardTriggered),
        failure_categories: vec![TraceFailureCategory::LoopGuardTriggered],
        event_counts: vec![
            super::TraceEventCount {
                event_type: "tool_call_complete".to_string(),
                count: 3,
            },
            super::TraceEventCount {
                event_type: "finished".to_string(),
                count: 1,
            },
        ],
        ..Default::default()
    };

    let expectations = ReplayExpectations {
        min_total_turns: 2,
        required_event_types: vec!["tool_call_complete".to_string()],
        ..ReplayExpectations::strict()
    };
    let result = expectations.evaluate(&failing_summary);

    assert!(!result.passed);
    assert!(result
        .violations
        .iter()
        .any(|violation| violation.contains("terminal reason")));
}

#[test]
fn replay_gate_rejects_provider_failures() {
    let failing_summary = super::RuntimeTraceSummary {
        total_events: 2,
        total_runs: 1,
        total_turns: 1,
        provider_failures: 1,
        last_stop_reason: Some(LoopStopReason::ProviderError),
        failure_categories: vec![TraceFailureCategory::ProviderError],
        ..Default::default()
    };

    let result = ReplayExpectations::strict().evaluate(&failing_summary);
    assert!(!result.passed);
    assert!(result
        .violations
        .iter()
        .any(|violation| violation.contains("terminal reason")));
}

#[test]
fn latest_sequence_and_after_filter_follow_monotonic_trace_order() {
    let (db, _temp_dir, session_id) = create_test_db();
    let store = RuntimeTraceStore::new(&db);

    store
        .append_event(
            &session_id,
            &RuntimeTraceEvent::from_loop_event(
                "run-1".to_string(),
                1,
                1,
                &LoopEvent::TextDelta {
                    delta: "one".to_string(),
                },
            ),
        )
        .expect("first event should persist");
    store
        .append_event(
            &session_id,
            &RuntimeTraceEvent::from_loop_event(
                "run-1".to_string(),
                2,
                1,
                &LoopEvent::TurnComplete {
                    turn: 1,
                    has_more: true,
                },
            ),
        )
        .expect("second event should persist");
    store
        .append_event(
            &session_id,
            &RuntimeTraceEvent::from_loop_event(
                "run-1".to_string(),
                3,
                2,
                &LoopEvent::Finished {
                    session_id: session_id.clone(),
                    stop_reason: LoopStopReason::Completed,
                },
            ),
        )
        .expect("third event should persist");

    assert_eq!(
        store
            .latest_sequence(&session_id)
            .expect("latest sequence should load"),
        Some(3)
    );

    let filtered = store
        .list_events_after(&session_id, 1, Some(10))
        .expect("filtered events should load");
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].sequence, 2);
    assert_eq!(filtered[1].sequence, 3);
}
