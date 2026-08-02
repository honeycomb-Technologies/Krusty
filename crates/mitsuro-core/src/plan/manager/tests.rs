use super::*;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn setup_test_manager() -> (PlanManager, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let plans_dir = temp_dir.path().join("plans");
    std::fs::create_dir_all(&plans_dir).unwrap();

    // Create database and run migrations
    let db = Database::new(&db_path).unwrap();

    // Create a test session for the plan
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["session-123", "Test Session", &now, &now],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["session-456", "Test Session 2", &now, &now],
        )
        .unwrap();

    let shared_db = Arc::new(Mutex::new(db));
    let manager = PlanManager {
        plans_dir,
        db: shared_db,
    };
    (manager, temp_dir)
}

#[test]
fn test_create_and_load_plan() {
    let (manager, _temp) = setup_test_manager();

    let plan = manager
        .create_plan("Test Plan", "session-123", Some("/tmp/test"))
        .unwrap();

    assert_eq!(plan.title, "Test Plan");
    assert_eq!(plan.session_id, Some("session-123".to_string()));

    // Reload and verify
    let loaded = manager.get_plan("session-123").unwrap().unwrap();
    assert_eq!(loaded.title, "Test Plan");
    assert_eq!(loaded.session_id, Some("session-123".to_string()));
}

#[test]
fn test_plan_per_session() {
    let (manager, _temp) = setup_test_manager();

    manager.create_plan("Plan A", "session-123", None).unwrap();
    manager.create_plan("Plan B", "session-456", None).unwrap();

    // Each session has its own plan
    let plan_a = manager.get_plan("session-123").unwrap().unwrap();
    let plan_b = manager.get_plan("session-456").unwrap().unwrap();

    assert_eq!(plan_a.title, "Plan A");
    assert_eq!(plan_b.title, "Plan B");

    // No plan for non-existent session
    assert!(manager.get_plan("session-999").unwrap().is_none());
}

#[test]
fn test_save_with_changes() {
    let (manager, _temp) = setup_test_manager();

    let mut plan = manager.create_plan("Test", "session-123", None).unwrap();
    {
        let phase = plan.add_phase("Phase 1");
        phase.add_task("Task one");
    }
    plan.check_task("1.1");
    manager.save_plan(&plan).unwrap();

    // Reload and verify
    let loaded = manager.get_plan("session-123").unwrap().unwrap();
    assert!(loaded.find_task("1.1").unwrap().completed);
}

#[test]
fn test_abandon_plan() {
    let (manager, _temp) = setup_test_manager();

    manager.create_plan("Test", "session-123", None).unwrap();
    assert!(manager.has_plan("session-123"));

    manager.abandon_plan("session-123").unwrap();
    assert!(!manager.has_plan("session-123"));
}

#[test]
fn test_get_active_plan_filters_completed_plan() {
    let (manager, _temp) = setup_test_manager();

    let mut plan = manager.create_plan("Test", "session-123", None).unwrap();
    let phase = plan.add_phase("Phase 1");
    phase.add_task("Done task");
    plan.complete_task("1.1", "Implemented").unwrap();
    manager.save_plan(&plan).unwrap();

    assert!(manager.get_active_plan("session-123").unwrap().is_none());
    assert!(manager.get_plan("session-123").unwrap().is_some());
}

#[test]
fn test_list_completed_for_dir_scopes_to_completed_plans_in_same_working_dir() {
    let (manager, _temp) = setup_test_manager();

    let db = manager.db.lock().unwrap();
    db.conn()
        .execute(
            "UPDATE sessions SET working_dir = ?1 WHERE id = ?2",
            rusqlite::params!["/tmp/project-a", "session-123"],
        )
        .unwrap();
    db.conn()
        .execute(
            "UPDATE sessions SET working_dir = ?1 WHERE id = ?2",
            rusqlite::params!["/tmp/project-a", "session-456"],
        )
        .unwrap();
    drop(db);

    let mut completed = manager
        .create_plan("Completed Here", "session-123", Some("/tmp/project-a"))
        .unwrap();
    let phase = completed.add_phase("Phase 1");
    phase.add_task("Done task");
    completed.complete_task("1.1", "Implemented").unwrap();
    manager.save_plan(&completed).unwrap();

    let mut active = manager
        .create_plan("Active Here", "session-456", Some("/tmp/project-a"))
        .unwrap();
    active.add_phase("Phase 1").add_task("Not done");
    manager.save_plan(&active).unwrap();

    let db = manager.db.lock().unwrap();
    db.conn()
            .execute(
                "INSERT INTO sessions (id, title, working_dir, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    "session-789",
                    "Test Session 3",
                    "/tmp/project-b",
                    Utc::now().to_rfc3339(),
                    Utc::now().to_rfc3339(),
                ],
            )
            .unwrap();
    drop(db);

    let mut other_dir = manager
        .create_plan("Completed Elsewhere", "session-789", Some("/tmp/project-b"))
        .unwrap();
    let phase = other_dir.add_phase("Phase 1");
    phase.add_task("Done task");
    other_dir.complete_task("1.1", "Implemented").unwrap();
    manager.save_plan(&other_dir).unwrap();

    let plans = manager.list_completed_for_dir("/tmp/project-a").unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].title, "Completed Here");
    assert_eq!(plans[0].working_dir.as_deref(), Some("/tmp/project-a"));
}
