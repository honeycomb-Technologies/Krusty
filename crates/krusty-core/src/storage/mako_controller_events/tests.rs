use tempfile::TempDir;

use crate::storage::Database;

use super::{MakoControllerEventStore, MakoControllerEventType, NewMakoControllerEvent};

fn store() -> (MakoControllerEventStore, TempDir) {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("controller-events.db")).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES ('session-1', 'Mako controller', '2026-07-01T00:00:00.000000Z',
                     '2026-07-01T00:00:00.000000Z', 'mako');
             INSERT INTO mako_controllers (
                 id, scope_key, user_id, session_id, status, timezone,
                 max_concurrent_runs, created_at, updated_at
             ) VALUES (
                 'controller-1', 'local:test', NULL, 'session-1', 'active', 'UTC', 1,
                 '2026-07-01T00:00:00.000000Z', '2026-07-01T00:00:00.000000Z'
             );",
        )
        .unwrap();
    (MakoControllerEventStore::new(db), temp)
}

fn event(dedupe_key: Option<&str>, run_id: &str) -> NewMakoControllerEvent {
    NewMakoControllerEvent {
        controller_id: "controller-1".into(),
        event_type: MakoControllerEventType::RunQueued,
        run_id: None,
        schedule_id: None,
        dedupe_key: dedupe_key.map(str::to_owned),
        payload: serde_json::json!({"run_id": run_id}),
        created_at: "2026-07-01T00:00:00.000000Z".into(),
    }
}

#[test]
fn events_have_a_controller_local_monotonic_sequence() {
    let (store, _temp) = store();
    assert_eq!(store.append(&event(None, "run-1")).unwrap().sequence, 1);
    assert_eq!(store.append(&event(None, "run-2")).unwrap().sequence, 2);

    let events = store.list_after("controller-1", 0, 20).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload["run_id"], "run-1");
    assert_eq!(events[1].payload["run_id"], "run-2");
}

#[test]
fn dedupe_key_returns_the_original_event_without_advancing_sequence() {
    let (store, _temp) = store();
    let original = store.append(&event(Some("run:1:queued"), "run-1")).unwrap();
    let replayed = store
        .append(&event(Some("run:1:queued"), "changed"))
        .unwrap();
    assert_eq!(replayed, original);
    assert_eq!(store.append(&event(None, "run-2")).unwrap().sequence, 2);
    assert_eq!(
        store
            .conn()
            .query_row("SELECT COUNT(*) FROM mako_controller_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        2
    );
}
