use chrono::Utc;
use std::sync::{Arc, Barrier};
use std::time::Duration;
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

#[test]
fn pending_steering_is_hidden_survives_replacement_and_promotes_once_at_end() {
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
        .expect("initial message should save");
    store
        .queue_pending_steering(
            &session_id,
            "steer-1",
            r#"[{"type":"text","text":"redirect"}]"#,
        )
        .expect("steering should stage");

    assert_eq!(
        store
            .load_session_messages(&session_id)
            .expect("canonical messages should load")
            .len(),
        1,
        "staged steering must not leak through canonical history"
    );
    assert_eq!(
        store
            .get_message_count(&session_id)
            .expect("canonical count should load"),
        1
    );
    assert_eq!(
        store
            .load_session_message_records(&session_id)
            .expect("canonical records should load")
            .len(),
        1
    );

    store
        .replace_session_messages(
            &session_id,
            &[
                (
                    "system".to_string(),
                    r#"[{"type":"text","text":"summary"}]"#.to_string(),
                ),
                (
                    "assistant".to_string(),
                    r#"[{"type":"text","text":"latest"}]"#.to_string(),
                ),
            ],
        )
        .expect("replacement should preserve pending steering");

    assert!(store
        .promote_pending_steering(&session_id, "steer-1")
        .expect("promotion should succeed")
        .is_some());
    assert!(store
        .promote_pending_steering(&session_id, "steer-1")
        .expect("duplicate promotion should be harmless")
        .is_none());

    let messages = store
        .load_session_messages(&session_id)
        .expect("promoted history should load");
    assert_eq!(
        messages
            .iter()
            .map(|(role, _)| role.as_str())
            .collect::<Vec<_>>(),
        vec!["system", "assistant", "user"]
    );
    assert_eq!(
        messages
            .iter()
            .filter(|(_, content)| content.contains("redirect"))
            .count(),
        1
    );
}

#[test]
fn enqueue_once_rejects_duplicate_completion_before_and_after_promotion() {
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
    let content = r#"[{"type":"text","text":"child finished"}]"#;

    assert!(store
        .queue_pending_steering_once(&session_id, "child-wake-run-1", content)
        .expect("first completion should enqueue"));
    assert!(!store
        .queue_pending_steering_once(&session_id, "child-wake-run-1", content)
        .expect("duplicate pending completion should be idempotent"));
    assert!(store
        .promote_pending_steering(&session_id, "child-wake-run-1")
        .expect("completion should promote")
        .is_some());
    assert!(!store
        .queue_pending_steering_once(&session_id, "child-wake-run-1", content)
        .expect("duplicate promoted completion should remain idempotent"));

    let canonical = store
        .load_session_messages(&session_id)
        .expect("canonical history should load");
    assert_eq!(
        canonical
            .iter()
            .filter(|(_, message)| message.contains("child finished"))
            .count(),
        1,
        "one completion event must produce exactly one canonical user message"
    );
}

#[test]
fn orphaned_steering_recovers_after_the_interrupted_runs_final_assistant() {
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
        .save_message(&session_id, "user", r#"[{"type":"text","text":"start"}]"#)
        .expect("initial message should save");
    store
        .queue_pending_steering(
            &session_id,
            "steer-1",
            r#"[{"type":"text","text":"redirect"}]"#,
        )
        .expect("steering should stage");
    store
        .save_message(
            &session_id,
            "assistant",
            r#"[{"type":"text","text":"interrupted run finished"}]"#,
        )
        .expect("assistant completion should save");

    assert_eq!(
        store
            .promote_orphaned_pending_steering(&session_id)
            .expect("orphan recovery should succeed"),
        1
    );
    assert_eq!(
        store
            .promote_orphaned_pending_steering(&session_id)
            .expect("orphan recovery should be idempotent"),
        0
    );

    let messages = store
        .load_session_messages(&session_id)
        .expect("recovered history should load");
    assert_eq!(
        messages
            .iter()
            .map(|(role, _)| role.as_str())
            .collect::<Vec<_>>(),
        vec!["user", "assistant", "user"]
    );
    assert!(messages[2].1.contains("redirect"));
}

#[test]
fn orphaned_steering_waits_for_a_concurrent_writer_instead_of_losing_its_snapshot() {
    let (db, temp) = create_test_db();
    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, "Test", now, now],
        )
        .expect("Failed to create session");
    MessageStore::new(&db)
        .queue_pending_steering(
            &session_id,
            "steer-1",
            r#"[{"type":"text","text":"redirect"}]"#,
        )
        .expect("steering should stage");

    let promotion_db = Database::new(&temp.path().join("test.db"))
        .expect("promotion connection should initialize before contention");
    let blocker = Database::new(&temp.path().join("test.db"))
        .expect("blocker connection should initialize before contention");
    let blocker_tx = rusqlite::Transaction::new_unchecked(
        blocker.conn(),
        rusqlite::TransactionBehavior::Immediate,
    )
    .expect("blocker should reserve the writer");
    blocker_tx
        .execute(
            "UPDATE sessions SET title = 'Concurrent writer' WHERE id = ?1",
            [&session_id],
        )
        .expect("blocker should create a real WAL write");

    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let worker_session_id = session_id;
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        MessageStore::new(&promotion_db).promote_orphaned_pending_steering(&worker_session_id)
    });

    barrier.wait();
    // Give the promotion connection time to contend for the writer. An
    // immediate transaction waits here; the former deferred transaction read
    // a snapshot and then failed its write upgrade with SQLITE_BUSY_SNAPSHOT.
    std::thread::sleep(Duration::from_millis(100));
    blocker_tx
        .commit()
        .expect("blocker should release the writer");

    assert_eq!(
        worker
            .join()
            .expect("promotion worker should not panic")
            .expect("promotion should wait for the writer and then succeed"),
        1
    );
}
