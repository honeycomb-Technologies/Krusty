use rusqlite::params;
use tempfile::TempDir;

use super::*;
use crate::storage::Database;

fn create_store() -> (AutonomousTaskStore, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let db = Database::new(&tmp.path().join("tasks.db")).expect("db");
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params!["sess-1", "Task Test", now, now],
        )
        .expect("seed session");
    (AutonomousTaskStore::new(db), tmp)
}

#[test]
fn create_and_list_tasks() {
    let (store, _tmp) = create_store();
    let id = store
        .create_task("sess-1", "Write parser", "Implement the SQL parser", &[])
        .unwrap();
    assert!(!id.is_empty());

    let tasks = store.list_tasks("sess-1").unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].subject, "Write parser");
    assert_eq!(tasks[0].status, TaskStatus::Pending);
}

#[test]
fn claim_complete_fail_lifecycle() {
    let (store, _tmp) = create_store();
    let t1 = store.create_task("sess-1", "Task A", "", &[]).unwrap();
    let t2 = store.create_task("sess-1", "Task B", "", &[]).unwrap();

    store.claim_task(&t1, "agent-1").unwrap();
    let task = store.get_task(&t1).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::InProgress);
    assert_eq!(task.owner.as_deref(), Some("agent-1"));

    store.complete_task(&t1, "done").unwrap();
    let task = store.get_task(&t1).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Completed);
    assert!(task.completed_at.is_some());

    store.fail_task(&t2, "compile error").unwrap();
    let task = store.get_task(&t2).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.result.as_deref(), Some("compile error"));
}

#[test]
fn get_available_respects_blocked_by() {
    let (store, _tmp) = create_store();
    let t1 = store.create_task("sess-1", "Foundation", "", &[]).unwrap();
    let t2 = store
        .create_task(
            "sess-1",
            "Depends on foundation",
            "",
            std::slice::from_ref(&t1),
        )
        .unwrap();
    let _t3 = store.create_task("sess-1", "Independent", "", &[]).unwrap();

    let available = store.get_available_tasks("sess-1").unwrap();
    assert_eq!(available.len(), 2);
    assert!(available.iter().all(|t| t.id != t2));

    store.complete_task(&t1, "ok").unwrap();
    let available = store.get_available_tasks("sess-1").unwrap();
    assert_eq!(available.len(), 2);
    assert!(available.iter().any(|t| t.id == t2));
}

#[test]
fn get_task_returns_none_for_missing() {
    let (store, _tmp) = create_store();
    assert!(store.get_task("nonexistent").unwrap().is_none());
}
