use serde_json::{json, Value};

use crate::storage::{AutonomousTaskStore, Database, TaskStatus};
use crate::tools::registry::{ToolContext, ToolResult};

use super::Params;

pub(super) fn execute(params: Params, ctx: &ToolContext) -> ToolResult {
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
