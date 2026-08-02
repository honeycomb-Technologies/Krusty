use tempfile::TempDir;

use super::{HiveRunPriority, HiveRuntimeStateStatus, HiveRuntimeStateStore};
use crate::storage::Database;

fn create_store() -> (HiveRuntimeStateStore, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let db = Database::new(&tmp.path().join("hive.db")).expect("db");
    seed_session(&db, "sess-1", "Hive Test");
    (HiveRuntimeStateStore::new(db), tmp)
}

fn seed_session(db: &Database, session_id: &str, title: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, title, now, now],
        )
        .expect("seed session");
}

fn seed_session_from_store(store: &HiveRuntimeStateStore, session_id: &str, title: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    store
        .conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, title, now, now],
        )
        .expect("seed session");
}

#[test]
fn set_and_get_state_round_trip() {
    let (store, _tmp) = create_store();
    store
        .set_state(
            "sess-1",
            HiveRuntimeStateStatus::Running,
            None,
            None,
            None,
            Some("run-1"),
            Some("dispatch"),
            HiveRunPriority::Normal,
        )
        .expect("state write");

    let state = store
        .get_state("sess-1")
        .expect("state read")
        .expect("state present");
    assert_eq!(state.status, HiveRuntimeStateStatus::Running);
    assert_eq!(state.current_run_id.as_deref(), Some("run-1"));
    assert_eq!(state.last_wake_reason.as_deref(), Some("dispatch"));
    assert_eq!(state.priority, HiveRunPriority::Normal);
}

#[test]
fn list_recoverable_states_only_returns_running_and_sleeping() {
    let (store, _tmp) = create_store();
    seed_session_from_store(&store, "sess-2", "Hive Sleep");
    store
        .set_state(
            "sess-1",
            HiveRuntimeStateStatus::Running,
            None,
            None,
            None,
            None,
            None,
            HiveRunPriority::Normal,
        )
        .expect("write running state");
    store
        .set_state(
            "sess-2",
            HiveRuntimeStateStatus::Sleeping,
            Some("2026-01-01T00:00:00Z"),
            Some("waiting"),
            None,
            None,
            Some("sleep"),
            HiveRunPriority::High,
        )
        .expect("write sleeping state");
    store
        .conn()
        .execute(
            "UPDATE hive_runtime_state SET status = 'paused' WHERE session_id = ?1",
            rusqlite::params!["sess-1"],
        )
        .expect("rewrite sess-1 paused");
    store
        .set_state(
            "sess-1",
            HiveRuntimeStateStatus::Running,
            None,
            None,
            None,
            None,
            None,
            HiveRunPriority::Normal,
        )
        .expect("rewrite sess-1 running");

    let states = store.list_recoverable_states().expect("recoverable states");
    assert_eq!(states.len(), 2);
    assert!(states.iter().any(
        |state| state.session_id == "sess-1" && state.status == HiveRuntimeStateStatus::Running
    ));
    assert!(states
        .iter()
        .any(|state| state.session_id == "sess-2"
            && state.status == HiveRuntimeStateStatus::Sleeping));
}

#[test]
fn list_states_for_sessions_returns_requested_rows_only() {
    let (store, _tmp) = create_store();
    seed_session_from_store(&store, "sess-2", "Hive Sleep");
    seed_session_from_store(&store, "sess-3", "Hive Idle");
    store
        .set_state(
            "sess-1",
            HiveRuntimeStateStatus::Running,
            None,
            None,
            None,
            Some("run-1"),
            Some("dispatch"),
            HiveRunPriority::Low,
        )
        .expect("write sess-1 state");
    store
        .set_state(
            "sess-2",
            HiveRuntimeStateStatus::Sleeping,
            Some("2026-01-01T00:00:00Z"),
            Some("waiting"),
            None,
            None,
            Some("sleep"),
            HiveRunPriority::High,
        )
        .expect("write sess-2 state");

    let states = store
        .list_states_for_sessions(&["sess-1".to_string(), "sess-3".to_string()])
        .expect("batch state lookup");

    assert_eq!(states.len(), 1);
    assert_eq!(
        states.get("sess-1").map(|state| state.status),
        Some(HiveRuntimeStateStatus::Running)
    );
    assert_eq!(
        states.get("sess-1").map(|state| state.priority),
        Some(HiveRunPriority::Low)
    );
    assert!(!states.contains_key("sess-3"));
}

#[test]
fn set_priority_creates_or_updates_runtime_state() {
    let (store, _tmp) = create_store();
    store
        .set_priority("sess-1", HiveRunPriority::High)
        .expect("priority write");

    let state = store
        .get_state("sess-1")
        .expect("state read")
        .expect("state present");
    assert_eq!(state.status, HiveRuntimeStateStatus::Idle);
    assert_eq!(state.priority, HiveRunPriority::High);
}
