use serde_json::json;

use crate::storage::{AutonomousTaskStore, Database};
use crate::tools::registry::{ToolContext, ToolResult};

use super::common::refresh_snapshot_warning;
use super::Params;

pub(super) fn execute(params: Params, ctx: &ToolContext) -> ToolResult {
    let Some(subject) = params.subject.as_deref().filter(|value| !value.is_empty()) else {
        return ToolResult::invalid_parameters("create requires non-empty subject");
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
    let description = params.description.as_deref().unwrap_or("");
    let blocked_by = params.blocked_by.unwrap_or_default();

    match store.create_task(session_id, subject, description, &blocked_by) {
        Ok(task_id) => {
            let warnings = refresh_snapshot_warning(ctx, db_path);
            ToolResult::success_data_with(
                json!({
                    "task_id": task_id,
                    "subject": subject,
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
