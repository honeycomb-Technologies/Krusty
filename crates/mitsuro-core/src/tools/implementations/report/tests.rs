use serde_json::{json, Value};
use tempfile::TempDir;

use super::ReportTool;
use crate::storage::{Database, MemoryStore, SessionManager, WorkspaceMode};
use crate::tools::registry::{Tool, ToolContext};

fn default_ctx() -> ToolContext {
    ToolContext::default()
}

fn session_ctx() -> (ToolContext, TempDir) {
    let temp = TempDir::new().expect("temp dir");
    let db_path = temp.path().join("mitsuro.db");
    let session_id = SessionManager::new(Database::new(&db_path).expect("db"))
        .create_session(
            "report test",
            None,
            Some(temp.path().to_string_lossy().as_ref()),
        )
        .expect("session");
    let ctx = ToolContext::default()
        .with_workspace(Some(temp.path().to_path_buf()), WorkspaceMode::Selected)
        .with_session_metadata(session_id, db_path);
    (ctx, temp)
}

#[tokio::test]
async fn report_create_requires_title() {
    let result = ReportTool
        .execute(
            json!({ "action": "create", "title": "", "content": "something" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.output.contains("title"));
}

#[tokio::test]
async fn report_create_requires_content() {
    let result = ReportTool
        .execute(
            json!({ "action": "create", "title": "Test", "content": "" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.output.contains("content"));
}

#[tokio::test]
async fn report_create_requires_session() {
    let result = ReportTool
        .execute(
            json!({ "action": "create", "title": "Test", "content": "Body" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.output.contains("session"));
}

#[tokio::test]
async fn report_list_requires_db() {
    let result = ReportTool
        .execute(json!({ "action": "list" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn report_read_requires_id() {
    let result = ReportTool
        .execute(json!({ "action": "read", "report_id": "" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn report_read_requires_db() {
    let result = ReportTool
        .execute(
            json!({ "action": "read", "report_id": "some-id" }),
            &default_ctx(),
        )
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn report_create_missing_fields() {
    let result = ReportTool
        .execute(json!({ "action": "create" }), &default_ctx())
        .await;
    assert!(result.is_error);
}

#[tokio::test]
async fn report_create_promotes_memory_when_requested() {
    let (ctx, _temp) = session_ctx();

    let result = ReportTool
        .execute(
            json!({
                "action": "create",
                "title": "Wake audit",
                "content": "# Wake
            The wake flow is stable.",
                "summary": "Wake flow is stable.",
                "promote_to_memory": true,
                "memory_type": "project"
            }),
            &ctx,
        )
        .await;

    assert!(!result.is_error);
    let payload: Value = serde_json::from_str(&result.output).expect("json tool result");
    let promoted = payload
        .get("data")
        .and_then(|value| value.as_object())
        .and_then(|data| data.get("promoted_memory"))
        .and_then(|value| value.as_object())
        .expect("promoted memory object");
    assert_eq!(
        promoted.get("memory_type").and_then(|value| value.as_str()),
        Some("project")
    );
    assert_eq!(
        promoted.get("title").and_then(|value| value.as_str()),
        Some("Wake audit")
    );

    let memories = MemoryStore::new(Database::new(ctx.db_path.as_ref().expect("db")).unwrap())
        .list(
            ctx.project_dir.as_ref().and_then(|path| path.to_str()),
            None,
        );
    let durable = memories
        .iter()
        .find(|memory| memory.title == "Wake audit")
        .expect("durable promoted memory");
    assert_eq!(durable.content, "Wake flow is stable.");
}

#[tokio::test]
async fn report_create_rejects_invalid_promoted_memory_type() {
    let (ctx, _temp) = session_ctx();

    let result = ReportTool
        .execute(
            json!({
                "action": "create",
                "title": "Wake audit",
                "content": "# Wake
            The wake flow is stable.",
                "promote_to_memory": true,
                "memory_type": "unknown"
            }),
            &ctx,
        )
        .await;

    assert!(result.is_error);
    assert!(result.output.contains("Invalid memory_type"));
}

#[tokio::test]
async fn report_read_missing_fields() {
    let result = ReportTool
        .execute(json!({ "action": "read" }), &default_ctx())
        .await;
    assert!(result.is_error);
}
