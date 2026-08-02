use tempfile::TempDir;

use crate::storage::Database;

use super::AgentStateStore;

fn create_test_db() -> (Database, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");
    let db = Database::new(&db_path).expect("Failed to create database");
    (db, temp_dir)
}

#[test]
fn test_set_and_get_agent_state() {
    let (db, _temp) = create_test_db();
    let store = AgentStateStore::new(&db);

    let session_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, "Test", now, now],
        )
        .expect("Failed to create session");

    store
        .set_agent_state(&session_id, "streaming")
        .expect("Failed to set agent state");

    let state = store
        .try_get_agent_state(&session_id)
        .expect("Failed to read agent state");

    assert!(state.is_some());
    assert_eq!(state.unwrap().state, "streaming");
}

#[test]
fn test_list_active_sessions() {
    let (db, _temp) = create_test_db();
    let store = AgentStateStore::new(&db);

    let session1 = uuid::Uuid::new_v4().to_string();
    let session2 = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    for session_id in &[&session1, &session2] {
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session_id, "Test", now, now],
            )
            .expect("Failed to create session");
    }

    store
        .set_agent_state(&session1, "streaming")
        .expect("Failed to set agent state");
    store
        .set_agent_state(&session2, "idle")
        .expect("Failed to set agent state");

    let active = store
        .list_active_sessions()
        .expect("Failed to list active sessions");

    assert_eq!(active.len(), 1);
    assert_eq!(active[0].0, session1);
    assert_eq!(active[0].1.state, "streaming");
}
