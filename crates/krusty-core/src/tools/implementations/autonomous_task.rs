use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::{refresh_current_snapshot, AutonomousTaskStore, Database, TaskStatus};
use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// CreateTask
// ---------------------------------------------------------------------------

pub struct CreateTaskTool;

#[derive(Deserialize)]
struct CreateTaskParams {
    subject: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    blocked_by: Option<Vec<String>>,
}

#[async_trait]
impl Tool for CreateTaskTool {
    fn name(&self) -> &str {
        "create_task"
    }

    fn description(&self) -> &str {
        "Create a trackable task for autonomous coordination. Tasks can depend on other tasks via blocked_by."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Create tasks to decompose work into trackable units. Each task should represent a meaningful deliverable — not a single tool call.

Guidelines:
- Use descriptive subjects that teammates can understand without additional context
- Set blocked_by when a task genuinely cannot start until another completes (shared state, API contract, etc.)
- Don't create dependency chains for tasks that could run in parallel
- Create all tasks for a phase before spawning teammates to work on them"#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "Short subject line describing the task"
                },
                "description": {
                    "type": "string",
                    "description": "Detailed description of what needs to be done"
                },
                "blocked_by": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs that must complete before this task can start"
                }
            },
            "required": ["subject"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<CreateTaskParams>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if params.subject.is_empty() {
            return ToolResult::invalid_parameters("subject must not be empty");
        }

        let (db_path, session_id) = match (&ctx.db_path, &ctx.session_id) {
            (Some(db), Some(sid)) => (db, sid),
            _ => return ToolResult::error("No active session for task management"),
        };

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(e) => return ToolResult::error(format!("Database error: {e}")),
        };

        let store = AutonomousTaskStore::new(db);
        let description = params.description.as_deref().unwrap_or("");
        let blocked_by = params.blocked_by.unwrap_or_default();

        match store.create_task(session_id, &params.subject, description, &blocked_by) {
            Ok(task_id) => {
                let warnings = refresh_snapshot_warning(ctx, db_path);
                ToolResult::success_data_with(
                    json!({
                        "task_id": task_id,
                        "subject": params.subject,
                        "status": "pending",
                    }),
                    warnings,
                    None,
                    None,
                )
            }
            Err(e) => ToolResult::error(format!("Failed to create task: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// UpdateTask
// ---------------------------------------------------------------------------

pub struct UpdateTaskTool;

#[derive(Deserialize)]
struct UpdateTaskParams {
    task_id: String,
    action: String,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    result: Option<String>,
}

const VALID_ACTIONS: &[&str] = &["claim", "complete", "fail"];

#[async_trait]
impl Tool for UpdateTaskTool {
    fn name(&self) -> &str {
        "update_task"
    }

    fn description(&self) -> &str {
        "Update a task's status: claim it for work, mark it complete, or mark it failed."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Transition tasks through their lifecycle:
- "claim": Mark a task as in_progress. Provide owner (teammate name) so progress is traceable.
- "complete": Mark a task as done. Provide result summarizing what was accomplished.
- "fail": Mark a task as failed. Provide result explaining what went wrong.

Always claim before completing. Provide meaningful result strings — they're used for verification and reporting."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "ID of the task to update"
                },
                "action": {
                    "type": "string",
                    "enum": ["claim", "complete", "fail"],
                    "description": "Transition to apply"
                },
                "owner": {
                    "type": "string",
                    "description": "Name of the teammate claiming the task (required for claim)"
                },
                "result": {
                    "type": "string",
                    "description": "Outcome description (required for complete/fail)"
                }
            },
            "required": ["task_id", "action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<UpdateTaskParams>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if !VALID_ACTIONS.contains(&params.action.as_str()) {
            return ToolResult::invalid_parameters(format!(
                "Invalid action '{}'. Must be one of: claim, complete, fail",
                params.action
            ));
        }

        let (db_path, _session_id) = match (&ctx.db_path, &ctx.session_id) {
            (Some(db), Some(sid)) => (db, sid),
            _ => return ToolResult::error("No active session for task management"),
        };

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(e) => return ToolResult::error(format!("Database error: {e}")),
        };

        let store = AutonomousTaskStore::new(db);

        let result = match params.action.as_str() {
            "claim" => {
                let owner = params.owner.as_deref().unwrap_or("coordinator");
                store.claim_task(&params.task_id, owner)
            }
            "complete" => {
                let result_text = params.result.as_deref().unwrap_or("completed");
                store.complete_task(&params.task_id, result_text)
            }
            "fail" => {
                let result_text = params.result.as_deref().unwrap_or("failed");
                store.fail_task(&params.task_id, result_text)
            }
            _ => unreachable!(),
        };

        match result {
            Ok(()) => {
                let warnings = refresh_snapshot_warning(ctx, db_path);
                ToolResult::success_data_with(
                    json!({
                        "task_id": params.task_id,
                        "action": params.action,
                        "success": true,
                    }),
                    warnings,
                    None,
                    None,
                )
            }
            Err(e) => ToolResult::error(format!("Failed to update task: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// ListTasks
// ---------------------------------------------------------------------------

pub struct ListTasksTool;

#[derive(Deserialize)]
struct ListTasksParams {
    #[serde(default)]
    status_filter: Option<String>,
}

#[async_trait]
impl Tool for ListTasksTool {
    fn name(&self) -> &str {
        "list_tasks"
    }

    fn description(&self) -> &str {
        "List tasks for the current session, optionally filtered by status."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Check task progress during coordination. Use status_filter to focus on specific states:
- "pending": Tasks not yet started (check for newly unblocked work)
- "in_progress": Tasks currently being worked on
- "completed": Finished tasks (verify results)
- "failed": Tasks that need retry or escalation

Omit status_filter to see all tasks."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status_filter": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "failed"],
                    "description": "Filter tasks by status (omit for all tasks)"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<ListTasksParams>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let (db_path, session_id) = match (&ctx.db_path, &ctx.session_id) {
            (Some(db), Some(sid)) => (db, sid),
            _ => return ToolResult::error("No active session for task management"),
        };

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(e) => return ToolResult::error(format!("Database error: {e}")),
        };

        let store = AutonomousTaskStore::new(db);

        let tasks = match store.list_tasks(session_id) {
            Ok(t) => t,
            Err(e) => return ToolResult::error(format!("Failed to list tasks: {e}")),
        };

        let filter = params.status_filter.as_deref().and_then(TaskStatus::parse);

        let filtered: Vec<_> = if let Some(status) = filter {
            tasks.into_iter().filter(|t| t.status == status).collect()
        } else {
            tasks
        };

        let total = filtered.len();
        let task_json: Vec<Value> = filtered
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "subject": t.subject,
                    "status": t.status.to_string(),
                    "owner": t.owner,
                    "blocked_by": t.blocked_by,
                    "result": t.result,
                })
            })
            .collect();

        ToolResult::success_data(json!({
            "tasks": task_json,
            "total": total,
        }))
    }
}

fn refresh_snapshot_warning(ctx: &ToolContext, db_path: &std::path::Path) -> Vec<String> {
    let project_dir = ctx
        .project_dir
        .as_deref()
        .map(|path| path.to_string_lossy().into_owned());

    match refresh_current_snapshot(db_path, project_dir.as_deref(), ctx.user_id.as_deref()) {
        Ok(_) => Vec::new(),
        Err(err) => vec![format!("task updated but snapshot refresh failed: {err}")],
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::storage::{Database, MemoryStore, CURRENT_SNAPSHOT_TITLE};

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
                .with_session_metadata("sess-1".to_string(), db_path.clone()),
            temp,
        )
    }

    #[tokio::test]
    async fn create_task_requires_subject() {
        let result = CreateTaskTool
            .execute(json!({ "subject": "" }), &default_ctx())
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn create_task_requires_session() {
        let result = CreateTaskTool
            .execute(json!({ "subject": "Test task" }), &default_ctx())
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("session"));
    }

    #[tokio::test]
    async fn update_task_rejects_invalid_action() {
        let result = UpdateTaskTool
            .execute(
                json!({ "task_id": "t-1", "action": "restart" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("Invalid action"));
    }

    #[tokio::test]
    async fn update_task_requires_session() {
        let result = UpdateTaskTool
            .execute(
                json!({ "task_id": "t-1", "action": "claim" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn list_tasks_requires_session() {
        let result = ListTasksTool.execute(json!({}), &default_ctx()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn create_task_missing_subject_field() {
        let result = CreateTaskTool.execute(json!({}), &default_ctx()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn update_task_missing_fields() {
        let result = UpdateTaskTool.execute(json!({}), &default_ctx()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn create_task_refreshes_current_snapshot() {
        let (ctx, _temp) = session_ctx();

        let result = CreateTaskTool
            .execute(json!({ "subject": "Lock wake policy" }), &ctx)
            .await;
        assert!(!result.is_error);

        let store = MemoryStore::new(Database::new(ctx.db_path.as_ref().expect("db")).expect("db"));
        let snapshot = store
            .find_by_title_for_user(CURRENT_SNAPSHOT_TITLE, Some("/repo"), None)
            .expect("snapshot should exist");
        assert!(snapshot.content.contains("Open tasks: 1"));
        assert!(snapshot.content.contains("Lock wake policy"));
    }
}
