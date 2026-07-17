use serde_json::json;
use tempfile::TempDir;

use super::AutonomousTaskTool;
use crate::storage::{Database, MemoryStore, CURRENT_SNAPSHOT_TITLE};
use crate::tools::registry::{Tool, ToolContext};

fn default_ctx() -> ToolContext {
    ToolContext::default()
}

fn session_ctx() -> (ToolContext, TempDir) {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("tasks.db");
    let db = Database::new(&db_path).expect("db");
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type, working_dir, project_dir)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "sess-1",
                "Task Test",
                now,
                now,
                "mako",
                "/repo",
                "/repo"
            ],
        )
        .expect("seed session");

    (
        ToolContext::default()
            .with_workspace(
                Some(std::path::PathBuf::from("/repo")),
                crate::storage::WorkspaceMode::Selected,
            )
            .with_session_metadata("sess-1".to_string(), db_path),
        temp,
    )
}

#[tokio::test]
async fn autonomous_task_create_requires_subject() {
    let result = AutonomousTaskTool
        .execute(json!({ "action": "create", "subject": "" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn autonomous_task_create_requires_session() {
    let result = AutonomousTaskTool
        .execute(
            json!({ "action": "create", "subject": "Test task" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.output.contains("session"));
}

#[tokio::test]
async fn autonomous_task_update_rejects_invalid_transition() {
    let result = AutonomousTaskTool
        .execute(
            json!({ "action": "update", "task_id": "t-1", "transition": "restart" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.output.contains("Invalid transition"));
}

#[tokio::test]
async fn autonomous_task_update_requires_session() {
    let result = AutonomousTaskTool
        .execute(
            json!({ "action": "update", "task_id": "t-1", "transition": "claim" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn autonomous_task_list_requires_session() {
    let result = AutonomousTaskTool
        .execute(json!({ "action": "list" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn autonomous_task_create_missing_subject_field() {
    let result = AutonomousTaskTool
        .execute(json!({ "action": "create" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn autonomous_task_update_missing_fields() {
    let result = AutonomousTaskTool
        .execute(json!({ "action": "update" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn autonomous_task_create_refreshes_current_snapshot() {
    let (ctx, _temp) = session_ctx();

    let result = AutonomousTaskTool
        .execute(
            json!({ "action": "create", "subject": "Lock wake policy" }),
            &ctx,
        )
        .await;
    assert!(!result.is_error);

    let store = MemoryStore::new(Database::new(ctx.db_path.as_ref().expect("db")).expect("db"));
    let snapshot = store
        .find_by_title_for_user(CURRENT_SNAPSHOT_TITLE, Some("/repo"), None)
        .expect("snapshot should exist");
    assert!(snapshot.content.contains("Open tasks: 1"));
    assert!(snapshot.content.contains("Lock wake policy"));
}
