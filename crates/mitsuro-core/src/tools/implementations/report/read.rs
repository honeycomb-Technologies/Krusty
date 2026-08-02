use serde_json::json;

use crate::storage::{Database, ReportStore};
use crate::tools::registry::{ToolContext, ToolResult};

use super::Params;

pub(super) fn execute(params: Params, ctx: &ToolContext) -> ToolResult {
    let Some(report_id) = params
        .report_id
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return ToolResult::invalid_parameters("read requires non-empty report_id");
    };

    let db_path = match &ctx.db_path {
        Some(db) => db,
        None => return ToolResult::error("No active session for report reading"),
    };

    let db = match Database::new(db_path) {
        Ok(db) => db,
        Err(e) => return ToolResult::error(format!("Database error: {e}")),
    };

    let store = ReportStore::new(db);

    match store.get_report(report_id) {
        Ok(Some(report)) => ToolResult::success_data(json!({
            "id": report.id,
            "title": report.title,
            "content": report.content,
            "tags": report.tags,
            "sources": report.sources,
        })),
        Ok(None) => {
            ToolResult::error_with_code("not_found", format!("Report '{}' not found", report_id))
        }
        Err(e) => ToolResult::error(format!("Failed to read report: {e}")),
    }
}
