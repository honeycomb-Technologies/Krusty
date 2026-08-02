use serde_json::json;

use crate::storage::reports::{promote_report_content, CreateReportInput};
use crate::storage::{refresh_current_snapshot, Database, MemoryStore, MemoryType, ReportStore};
use crate::tools::registry::{ToolContext, ToolResult};

use super::Params;

pub(super) fn execute(params: Params, ctx: &ToolContext) -> ToolResult {
    let Some(title) = params.title.as_deref().filter(|value| !value.is_empty()) else {
        return ToolResult::invalid_parameters("create requires non-empty title");
    };
    let Some(content) = params.content.as_deref().filter(|value| !value.is_empty()) else {
        return ToolResult::invalid_parameters("create requires non-empty content");
    };

    let (db_path, session_id) = match (&ctx.db_path, &ctx.session_id) {
        (Some(db), Some(sid)) => (db, sid),
        _ => return ToolResult::error("No active session for report creation"),
    };

    let db = match Database::new(db_path) {
        Ok(db) => db,
        Err(e) => return ToolResult::error(format!("Database error: {e}")),
    };

    let store = ReportStore::new(db);
    let project_dir = ctx.project_dir.as_ref().map(|p| p.to_string_lossy());
    let report_root = ctx
        .project_dir
        .as_deref()
        .unwrap_or(ctx.working_dir.as_path());
    let summary = params.summary.as_deref().unwrap_or("");
    let tags = params.tags.unwrap_or_default();
    let sources = params.sources.unwrap_or_default();
    let user_id = ctx.user_id.as_deref();
    let promotion_memory_type = if params.promote_to_memory {
        match params.memory_type.as_deref() {
            Some(raw) => match raw.parse::<MemoryType>() {
                Ok(memory_type) => Some(memory_type),
                Err(_) => {
                    return ToolResult::invalid_parameters(format!(
                        "Invalid memory_type '{}'. Valid types: user, feedback, project, reference",
                        raw
                    ));
                }
            },
            None => Some(MemoryType::Project),
        }
    } else {
        None
    };

    match store.create_report(CreateReportInput {
        title,
        session_id,
        project_dir: project_dir.as_deref(),
        report_root: Some(report_root),
        content,
        summary,
        tags: &tags,
        sources: &sources,
    }) {
        Ok(report_id) => {
            let promoted_memory = if let Some(memory_type) = promotion_memory_type {
                let memory_store = MemoryStore::new(match Database::new(db_path) {
                    Ok(db) => db,
                    Err(e) => return ToolResult::error(format!("Database error: {e}")),
                });
                let report = match store.get_report(&report_id) {
                    Ok(Some(report)) => report,
                    Ok(None) => {
                        return ToolResult::error(format!(
                            "Report '{}' was created but could not be reloaded for promotion",
                            report_id
                        ));
                    }
                    Err(e) => {
                        return ToolResult::error(format!(
                            "Failed to reload created report for promotion: {e}"
                        ));
                    }
                };
                let memory_content = promote_report_content(&report);
                match memory_store.save_or_update_by_title(
                    memory_type,
                    &report.title,
                    &memory_content,
                    project_dir.as_deref(),
                    user_id,
                ) {
                    Ok((memory, created)) => Some(json!({
                        "id": memory.id,
                        "memory_type": memory.memory_type.as_str(),
                        "title": memory.title,
                        "created": created,
                    })),
                    Err(e) => {
                        return ToolResult::error(format!(
                            "Report created but memory promotion failed: {e}"
                        ));
                    }
                }
            } else {
                None
            };

            if let Err(e) = refresh_current_snapshot(db_path, project_dir.as_deref(), user_id) {
                return ToolResult::error(format!(
                    "report created but snapshot refresh failed: {e}"
                ));
            }

            ToolResult::success_data(json!({
                "report_id": report_id,
                "title": title,
                "promoted_memory": promoted_memory,
            }))
        }
        Err(e) => ToolResult::error(format!("Failed to create report: {e}")),
    }
}
