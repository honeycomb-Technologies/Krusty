use tempfile::TempDir;

use crate::storage::Database;

use super::{HiveController, HiveControllerStatus, HiveControllerStore};

fn store() -> (HiveControllerStore, TempDir) {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("controllers.db")).unwrap();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES ('session-1', 'Hive controller', ?1, ?1, 'hive')",
            ["2026-07-01T00:00:00.000000Z"],
        )
        .unwrap();
    (HiveControllerStore::new(db), temp)
}

#[test]
fn controller_round_trips_by_stable_scope() {
    let (store, _temp) = store();
    store
        .insert(&HiveController {
            id: "controller-1".into(),
            scope_key: "user:alice".into(),
            user_id: Some("alice".into()),
            session_id: "session-1".into(),
            status: HiveControllerStatus::Active,
            timezone: "America/Los_Angeles".into(),
            max_concurrent_runs: 2,
            created_at: "2026-07-01T00:00:00Z".into(),
            updated_at: "2026-07-01T00:00:00Z".into(),
        })
        .unwrap();

    let loaded = store.get_by_scope("user:alice").unwrap().unwrap();
    assert_eq!(loaded.session_id, "session-1");
    assert_eq!(loaded.max_concurrent_runs, 2);
}

#[test]
fn controller_rejects_unknown_timezone() {
    let (store, _temp) = store();
    let result = store.insert(&HiveController {
        id: "controller-1".into(),
        scope_key: "local".into(),
        user_id: None,
        session_id: "session-1".into(),
        status: HiveControllerStatus::Active,
        timezone: "Moon/Base-One".into(),
        max_concurrent_runs: 1,
        created_at: "2026-07-01T00:00:00Z".into(),
        updated_at: "2026-07-01T00:00:00Z".into(),
    });
    assert!(result.is_err());
    assert_eq!(
        store
            .conn()
            .query_row("SELECT COUNT(*) FROM hive_controllers", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
