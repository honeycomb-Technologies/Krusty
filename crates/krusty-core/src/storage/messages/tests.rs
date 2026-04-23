use chrono::Utc;
use tempfile::TempDir;

use super::MessageStore;
use crate::storage::Database;

fn create_test_db() -> (Database, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");
    let db = Database::new(&db_path).expect("Failed to create database");
    (db, temp_dir)
}

#[test]
fn test_save_and_load_messages() {
    let (db, _temp) = create_test_db();
    let store = MessageStore::new(&db);

    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, "Test", now, now],
        )
        .expect("Failed to create session");

    store
        .save_message(&session_id, "user", r#"[{"type":"text","text":"Hello"}]"#)
        .expect("Failed to save message");
    store
        .save_message(
            &session_id,
            "assistant",
            r#"[{"type":"text","text":"Hi there"}]"#,
        )
        .expect("Failed to save message");

    let messages = store
        .load_session_messages(&session_id)
        .expect("Failed to load messages");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].0, "user");
    assert_eq!(messages[1].0, "assistant");
}

#[test]
fn test_update_last_message_preserves_created_at() {
    let (db, _temp) = create_test_db();
    let store = MessageStore::new(&db);

    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, "Test", now, now],
        )
        .expect("Failed to create session");

    store
        .save_message(&session_id, "user", r#"[{"type":"text","text":"first"}]"#)
        .expect("Failed to save first message");
    store
        .save_message(
            &session_id,
            "assistant",
            r#"[{"type":"text","text":"reply"}]"#,
        )
        .expect("Failed to save assistant message");
    store
        .save_message(&session_id, "user", r#"[{"type":"text","text":"second"}]"#)
        .expect("Failed to save second message");

    let before: String = db
        .conn()
        .query_row(
            "SELECT created_at FROM messages
             WHERE session_id = ?1 AND role = 'user'
             ORDER BY id DESC LIMIT 1",
            [session_id.as_str()],
            |row| row.get(0),
        )
        .expect("Failed to read created_at before update");

    store
        .update_last_message(&session_id, "user", r#"[{"type":"text","text":"updated"}]"#)
        .expect("Failed to update last user message");

    let (content, after): (String, String) = db
        .conn()
        .query_row(
            "SELECT content, created_at FROM messages
             WHERE session_id = ?1 AND role = 'user'
             ORDER BY id DESC LIMIT 1",
            [session_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("Failed to read updated message");

    assert_eq!(content, r#"[{"type":"text","text":"updated"}]"#);
    assert_eq!(after, before);
}

#[test]
fn test_replace_session_messages_rewrites_history() {
    let (db, _temp) = create_test_db();
    let store = MessageStore::new(&db);

    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, "Test", now, now],
        )
        .expect("Failed to create session");

    store
        .save_message(&session_id, "user", r#"[{"type":"text","text":"old"}]"#)
        .expect("Failed to save message");
    store
        .save_message(
            &session_id,
            "assistant",
            r#"[{"type":"text","text":"reply"}]"#,
        )
        .expect("Failed to save message");

    store
        .replace_session_messages(
            &session_id,
            &[
                (
                    "system".to_string(),
                    r#"[{"type":"text","text":"summary"}]"#.to_string(),
                ),
                (
                    "user".to_string(),
                    r#"[{"type":"text","text":"continue"}]"#.to_string(),
                ),
            ],
        )
        .expect("Failed to replace messages");

    let messages = store
        .load_session_messages(&session_id)
        .expect("Failed to load replaced messages");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].0, "system");
    assert_eq!(messages[1].0, "user");
}
