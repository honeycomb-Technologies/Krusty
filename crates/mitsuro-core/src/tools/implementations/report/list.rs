use serde_json::{json, Value};

use crate::storage::{Database, ReportStore};
use crate::tools::registry::{ToolContext, ToolResult};

use super::{resolve_reader_scope, Params};

pub(super) fn execute(params: Params, ctx: &ToolContext) -> ToolResult {
    let db_path = match &ctx.db_path {
        Some(db) => db,
        None => return ToolResult::error("No active session for report listing"),
    };

    let db = match Database::new(db_path) {
        Ok(db) => db,
        Err(e) => return ToolResult::error(format!("Database error: {e}")),
    };

    let reader = match resolve_reader_scope(ctx, &db) {
        Ok(reader) => reader,
        Err(e) => return ToolResult::error(format!("Report access denied: {e}")),
    };

    let store = ReportStore::new(db);
    let project_dir = ctx.project_dir.as_ref().map(|p| p.to_string_lossy());
    let project_dir_str = project_dir.as_deref();

    let reports = if let Some(query) = params.query.as_deref() {
        store.search_reports_for_memory_reader(
            query,
            project_dir_str,
            reader.user_id.as_deref(),
            reader.worker_id.as_deref(),
        )
    } else {
        store.list_reports_for_memory_reader(
            project_dir_str,
            reader.user_id.as_deref(),
            reader.worker_id.as_deref(),
        )
    };

    match reports {
        Ok(reports) => {
            let total = reports.len();
            let entries: Vec<Value> = reports
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "title": r.title,
                        "summary": r.summary,
                        "created_at": r.created_at,
                    })
                })
                .collect();

            ToolResult::success_data(json!({
                "reports": entries,
                "total": total,
            }))
        }
        Err(e) => ToolResult::error(format!("Failed to list reports: {e}")),
    }
}
