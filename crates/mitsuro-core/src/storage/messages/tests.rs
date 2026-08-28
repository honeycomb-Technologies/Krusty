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
fn save_message_once_adopts_the_first_canonical_payload_and_repairs_its_projection() {
    let (db, _temp) = create_test_db();
    let store = MessageStore::new(&db);
    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, 'Introduction', ?2, ?2)",
            rusqlite::params![session_id, now],
        )
        .unwrap();

    let first = store
        .save_message_once(
            &session_id,
            "assistant",
            r#"[{"type":"text","text":"Who should I become?"}]"#,
            "worker-introduction:worker-1:v1",
        )
        .expect("save opening");
    // Simulate a lost best-effort projection after the canonical commit. The
    // retry must adopt the same message and restore the episode row from the
    // canonical payload, not the conflicting retry body.
    db.conn()
        .execute(
            "DELETE FROM conversation_episodes WHERE source_message_id = ?1",
            [first],
        )
        .unwrap();
    let adopted = store
        .save_message_once(
            &session_id,
            "assistant",
            r#"[{"type":"text","text":"conflicting retry"}]"#,
            "worker-introduction:worker-1:v1",
        )
        .expect("adopt opening");

    assert_eq!(adopted, first);
    let (count, content): (i64, String) = db
        .conn()
        .query_row(
            "SELECT COUNT(*), content FROM messages
             WHERE session_id = ?1 AND idempotency_key = ?2",
            rusqlite::params![session_id, "worker-introduction:worker-1:v1"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert!(content.contains("Who should I become?"));
    assert!(!content.contains("conflicting retry"));
    let episode_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM conversation_episodes WHERE source_message_id = ?1",
            [first],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(episode_count, 1);
}

#[test]
fn save_message_once_scopes_keys_to_the_session() {
    let (db, _temp) = create_test_db();
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES
                 ('session-a', 'A', ?1, ?1),
                 ('session-b', 'B', ?1, ?1)",
            [&now],
        )
        .unwrap();
    let store = MessageStore::new(&db);
    let a = store
        .save_message_once("session-a", "assistant", "[]", "opening:v1")
        .unwrap();
    let b = store
        .save_message_once("session-b", "assistant", "[]", "opening:v1")
        .unwrap();

    assert_ne!(a, b);
}

#[test]
fn concurrent_save_message_once_calls_return_one_canonical_id() {
    let (db, temp) = create_test_db();
    let path = temp.path().join("test.db");
    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, 'Introduction', ?2, ?2)",
            rusqlite::params![session_id, now],
        )
        .unwrap();
    // Initialize both connections before synchronizing the writers so schema
    // migration itself is not part of the contention being tested.
    let databases = [
        Database::new(&path).expect("first writer database"),
        Database::new(&path).expect("second writer database"),
    ];
    let barrier = Arc::new(Barrier::new(2));
    let handles = databases
        .into_iter()
        .enumerate()
        .map(|(index, database)| {
            let barrier = Arc::clone(&barrier);
            let session_id = session_id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                MessageStore::new(&database)
                    .save_message_once(
                        &session_id,
                        "assistant",
                        &format!(r#"[{{"type":"text","text":"writer {index}"}}]"#),
                        "opening:v1",
                    )
                    .expect("concurrent save once")
            })
        })
        .collect::<Vec<_>>();
    let ids = handles
        .into_iter()
        .map(|handle| handle.join().expect("join message writer"))
        .collect::<Vec<_>>();

    assert_eq!(ids[0], ids[1]);
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM messages
             WHERE session_id = ?1 AND idempotency_key = 'opening:v1'",
            [&session_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn save_first_assistant_once_is_first_and_adopts_its_canonical_payload() {
    let (db, _temp) = create_test_db();
    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, 'Introduction', ?2, ?2)",
            rusqlite::params![session_id, now],
        )
        .unwrap();
    let store = MessageStore::new(&db);

    let first = store
        .save_first_assistant_once(
            &session_id,
            r#"[{"type":"text","text":"What are we here to build?"}]"#,
            "opening:v1",
        )
        .unwrap();
    store
        .save_message(
            &session_id,
            "user",
            r#"[{"type":"text","text":"A compiler"}]"#,
        )
        .unwrap();
    let adopted = store
        .save_first_assistant_once(
            &session_id,
            r#"[{"type":"text","text":"conflicting retry"}]"#,
            "opening:v1",
        )
        .unwrap();

    assert_eq!(adopted, first);
    let rows = store.load_session_messages(&session_id).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "assistant");
    assert!(rows[0].1.contains("What are we here to build?"));
    assert_eq!(rows[1].0, "user");

    let keyed = store
        .load_message_by_idempotency_key(&session_id, "opening:v1")
        .unwrap()
        .expect("keyed opening");
    assert_eq!(keyed.id, first);
    assert_eq!(keyed.role, "assistant");
    assert!(keyed.content_json.contains("What are we here to build?"));
    assert!(store
        .load_message_by_idempotency_key(&session_id, "missing")
        .unwrap()
        .is_none());
}

#[test]
fn racing_user_message_blocks_a_late_first_assistant() {
    let (db, temp) = create_test_db();
    let path = temp.path().join("test.db");
    let session_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, 'Introduction', ?2, ?2)",
            rusqlite::params![session_id, now],
        )
        .unwrap();
    let introduction_db = Database::new(&path).unwrap();
    let user_db = Database::new(&path).unwrap();
    let user_tx = rusqlite::Transaction::new_unchecked(
        user_db.conn(),
        rusqlite::TransactionBehavior::Immediate,
    )
    .unwrap();
    user_tx
        .execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, 'user', '[]', ?2)",
            rusqlite::params![session_id, now],
        )
        .unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let writer_barrier = Arc::clone(&barrier);
    let writer_session_id = session_id.clone();
    let writer = std::thread::spawn(move || {
        writer_barrier.wait();
        MessageStore::new(&introduction_db).save_first_assistant_once(
            &writer_session_id,
            r#"[{"type":"text","text":"Too late"}]"#,
            "opening:v1",
        )
    });
    barrier.wait();
    std::thread::sleep(Duration::from_millis(100));
    user_tx.commit().unwrap();

    let error = writer
        .join()
        .expect("join introduction writer")
        .expect_err("the committed user must win the first-message race");
    assert!(
        error.to_string().contains("already has messages"),
        "{error}"
    );
    let rows = MessageStore::new(&db)
        .load_session_messages(&session_id)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "user");
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
fn enqueue_once_rejects_duplicate_steering_before_and_after_promotion() {
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
    let content = r#"[{"type":"text","text":"durable follow-up"}]"#;

    assert!(store
        .queue_pending_steering_once(&session_id, "durable-steer-1", content)
        .expect("first steering input should enqueue"));
    assert!(!store
        .queue_pending_steering_once(&session_id, "durable-steer-1", content)
        .expect("duplicate pending steering should be idempotent"));
    assert!(store
        .promote_pending_steering(&session_id, "durable-steer-1")
        .expect("steering should promote")
        .is_some());
    assert!(!store
        .queue_pending_steering_once(&session_id, "durable-steer-1", content)
        .expect("duplicate promoted steering should remain idempotent"));

    let canonical = store
        .load_session_messages(&session_id)
        .expect("canonical history should load");
    assert_eq!(
        canonical
            .iter()
            .filter(|(_, message)| message.contains("durable follow-up"))
            .count(),
        1,
        "one durable input must produce exactly one canonical user message"
    );
}

#[test]
fn orphan_recovery_promotes_ordinary_steering_but_never_reserved_child_wakes() {
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
        .queue_pending_steering(
            &session_id,
            "child-wake-foreign-run",
            r#"[{"type":"text","text":"foreign child result"}]"#,
        )
        .expect("child completion fixture should stage");
    store
        .queue_pending_steering(
            &session_id,
            "steer-1",
            r#"[{"type":"text","text":"legitimate follow-up"}]"#,
        )
        .expect("ordinary steering should stage");

    assert_eq!(
        store
            .promote_orphaned_pending_steering(&session_id)
            .expect("ordinary orphan recovery should succeed"),
        1
    );
    assert!(store
        .has_pending_steering(&session_id, "child-wake-foreign-run")
        .expect("reserved child wake should remain non-canonical"));
    assert!(store
        .promote_pending_steering(&session_id, "child-wake-foreign-run")
        .expect_err("generic promotion must reject the reserved child-wake namespace")
        .to_string()
        .contains("workspace-authorized promotion"));

    let canonical = store
        .load_session_messages(&session_id)
        .expect("canonical history should load");
    assert_eq!(canonical.len(), 1);
    assert!(canonical[0].1.contains("legitimate follow-up"));
    assert!(!canonical[0].1.contains("foreign child result"));
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
