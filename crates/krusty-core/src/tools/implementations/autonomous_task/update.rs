use serde_json::json;

use crate::storage::{AutonomousTaskStore, Database};
use crate::tools::registry::{ToolContext, ToolResult};

use super::common::refresh_snapshot_warning;
use super::Params;

const VALID_TRANSITIONS: &[&str] = &["claim", "complete", "fail"];

pub(super) fn execute(params: Params, ctx: &ToolContext) -> ToolResult {
    let Some(task_id) = params.task_id.as_deref().filter(|value| !value.is_empty()) else {
        return ToolResult::invalid_parameters("update requires non-empty task_id");
    };
    let Some(transition) = params.transition.as_deref() else {
        return ToolResult::invalid_parameters("update requires transition");
    };
    if !VALID_TRANSITIONS.contains(&transition) {
        return ToolResult::invalid_parameters(format!(
            "Invalid transition '{}'. Must be one of: claim, complete, fail",
            transition
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

    let result = match transition {
        "claim" => {
            let owner = params.owner.as_deref().unwrap_or("coordinator");
            store.claim_task(task_id, owner)
        }
        "complete" => {
            let result_text = params.result.as_deref().unwrap_or("completed");
            store.complete_task(task_id, result_text)
        }
        "fail" => {
            let result_text = params.result.as_deref().unwrap_or("failed");
            store.fail_task(task_id, result_text)
        }
        _ => unreachable!(),
    };

    match result {
        Ok(()) => {
            let warnings = refresh_snapshot_warning(ctx, db_path);
            ToolResult::success_data_with(
                json!({
                    "task_id": task_id,
                    "transition": transition,
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
