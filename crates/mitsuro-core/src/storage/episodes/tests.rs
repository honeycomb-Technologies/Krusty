use chrono::Utc;
use tempfile::TempDir;

use super::{EpisodeSearch, EpisodeStore};
use crate::storage::Database;

fn setup() -> (TempDir, Database) {
    let temp = TempDir::new().expect("tempdir");
    let db = Database::new(&temp.path().join("episodes.db")).expect("database");
    (temp, db)
}

fn create_session(db: &Database, user_id: Option<&str>, project_dir: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    if let Some(user_id) = user_id {
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO users (id, email, license_tier)
                 VALUES (?1, ?2, 'free')",
                rusqlite::params![user_id, format!("{user_id}@episodes.test")],
            )
            .expect("user");
    }
    db.conn()
        .execute(
            "INSERT INTO sessions (
                id, title, created_at, updated_at, working_dir, project_dir,
                workspace_mode, user_id, session_type
             ) VALUES (?1, 'episode test', ?2, ?2, ?3, ?3, 'selected', ?4, 'hive')",
            rusqlite::params![id, now, project_dir, user_id],
        )
        .expect("session");
    id
}

fn create_message(db: &Database, session_id: &str, role: &str, content: &str, now: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, role, content, now],
        )
        .expect("message");
    db.conn().last_insert_rowid()
}

#[test]
fn search_is_user_and_project_scoped() {
    let (_temp, db) = setup();
    let alice = create_session(&db, Some("alice"), "/work/alpha");
    let bob = create_session(&db, Some("bob"), "/work/alpha");
    let alice_other = create_session(&db, Some("alice"), "/work/beta");
    let local = create_session(&db, None, "/work/alpha");
    let store = EpisodeStore::new(&db);
    let now = Utc::now().to_rfc3339();

    for (session, text) in [
        (&alice, "the hive scheduler uses leases"),
        (&bob, "the hive scheduler belongs to bob"),
        (&alice_other, "the beta scheduler is separate"),
        (&local, "the local scheduler is unowned"),
    ] {
        let content = serde_json::json!([{"type": "text", "text": text}]).to_string();
        let message_id = create_message(&db, session, "user", &content, &now);
        store
            .record_message(session, message_id, "user", &content, &now)
            .expect("episode");
    }

    let mut query = EpisodeSearch::new("scheduler", Some("alice"));
    query.project_dir = Some("/work/alpha");
    let results = store.search(&query).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].session_id, alice);

    let mut local_query = EpisodeSearch::new("scheduler", None);
    local_query.project_dir = Some("/work/alpha");
    let local_results = store.search(&local_query).expect("local search");
    assert_eq!(local_results.len(), 1);
    assert_eq!(local_results[0].session_id, local);
}

#[test]
fn record_is_idempotent_and_excludes_non_text_data() {
    let (_temp, db) = setup();
    let session = create_session(&db, None, "/work/local");
    let store = EpisodeStore::new(&db);
    let now = Utc::now().to_rfc3339();
    let content = serde_json::json!([
        {"type": "thinking", "thinking": "private", "signature": "sig"},
        {"type": "text", "text": "visible memory"},
        {"type": "tool_result", "tool_use_id": "t1", "output": "raw secret"}
    ])
    .to_string();
    let message_id = create_message(&db, &session, "assistant", &content, &now);

    let first = store
        .record_message(&session, message_id, "assistant", &content, &now)
        .expect("first")
        .expect("indexed");
    let second = store
        .record_message(&session, message_id, "assistant", &content, &now)
        .expect("second")
        .expect("indexed");
    assert_eq!(first, second);

    let results = store
        .search(&EpisodeSearch::new("visible", None))
        .expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].body, "visible memory");
    assert!(!results[0].body.contains("private"));
    assert!(!results[0].body.contains("secret"));
}
