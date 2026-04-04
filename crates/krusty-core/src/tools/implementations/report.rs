use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::{Database, ReportStore};
use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// CreateReport
// ---------------------------------------------------------------------------

pub struct CreateReportTool;

#[derive(Deserialize)]
struct CreateReportParams {
    title: String,
    content: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    sources: Option<Vec<String>>,
}

#[async_trait]
impl Tool for CreateReportTool {
    fn name(&self) -> &str {
        "create_report"
    }

    fn description(&self) -> &str {
        "Persist a research report. Reports survive across sessions and are written to disk as Markdown."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Create reports to persist research findings, architecture analyses, or investigation results. Reports are stored in the database and also written as Markdown files to .krusty/reports/ inside the active workspace.

When to create a report:
- After a deep investigation that produced reusable findings
- Architecture or design analyses that future sessions should reference
- Research that took significant effort and shouldn't be repeated

Structure:
- title: Descriptive name for the report
- content: Full Markdown content (can be lengthy)
- summary: One-line summary for listing views
- tags: Categorization labels for search (e.g., "auth", "database", "api")
- sources: References consulted (files, URLs, RFCs, etc.)"#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Report title"
                },
                "content": {
                    "type": "string",
                    "description": "Full Markdown content of the report"
                },
                "summary": {
                    "type": "string",
                    "description": "One-line summary for listing views"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Categorization tags for search"
                },
                "sources": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "References consulted (files, URLs, RFCs)"
                }
            },
            "required": ["title", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<CreateReportParams>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if params.title.is_empty() {
            return ToolResult::invalid_parameters("title must not be empty");
        }
        if params.content.is_empty() {
            return ToolResult::invalid_parameters("content must not be empty");
        }

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

        match store.create_report(
            &params.title,
            session_id,
            project_dir.as_deref(),
            Some(report_root),
            &params.content,
            summary,
            &tags,
            &sources,
        ) {
            Ok(report_id) => ToolResult::success_data(json!({
                "report_id": report_id,
                "title": params.title,
            })),
            Err(e) => ToolResult::error(format!("Failed to create report: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// ListReports
// ---------------------------------------------------------------------------

pub struct ListReportsTool;

#[derive(Deserialize)]
struct ListReportsParams {
    #[serde(default)]
    query: Option<String>,
}

#[async_trait]
impl Tool for ListReportsTool {
    fn name(&self) -> &str {
        "list_reports"
    }

    fn description(&self) -> &str {
        "List or search existing reports for the current project."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Query existing reports for context from prior research. Omit query to list all reports for the current project. Provide a query string to search by title or tags.

Use this to check if relevant research already exists before starting a new investigation."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query to filter by title or tags (omit for all reports)"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<ListReportsParams>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let db_path = match &ctx.db_path {
            Some(db) => db,
            None => return ToolResult::error("No active session for report listing"),
        };

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(e) => return ToolResult::error(format!("Database error: {e}")),
        };

        let store = ReportStore::new(db);
        let project_dir = ctx.project_dir.as_ref().map(|p| p.to_string_lossy());
        let project_dir_str = project_dir.as_deref();

        let reports = if let Some(query) = &params.query {
            store.search_reports(query, project_dir_str)
        } else {
            store.list_reports(project_dir_str)
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
}

// ---------------------------------------------------------------------------
// ReadReport
// ---------------------------------------------------------------------------

pub struct ReadReportTool;

#[derive(Deserialize)]
struct ReadReportParams {
    report_id: String,
}

#[async_trait]
impl Tool for ReadReportTool {
    fn name(&self) -> &str {
        "read_report"
    }

    fn description(&self) -> &str {
        "Read the full content of a report by ID."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Retrieve the full content of a specific report. Use ListReports first to find the report ID, then ReadReport to get the complete content, tags, and sources."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "report_id": {
                    "type": "string",
                    "description": "ID of the report to read"
                }
            },
            "required": ["report_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<ReadReportParams>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if params.report_id.is_empty() {
            return ToolResult::invalid_parameters("report_id must not be empty");
        }

        let db_path = match &ctx.db_path {
            Some(db) => db,
            None => return ToolResult::error("No active session for report reading"),
        };

        let db = match Database::new(db_path) {
            Ok(db) => db,
            Err(e) => return ToolResult::error(format!("Database error: {e}")),
        };

        let store = ReportStore::new(db);

        match store.get_report(&params.report_id) {
            Ok(Some(report)) => ToolResult::success_data(json!({
                "id": report.id,
                "title": report.title,
                "content": report.content,
                "tags": report.tags,
                "sources": report.sources,
            })),
            Ok(None) => ToolResult::error_with_code(
                "not_found",
                format!("Report '{}' not found", params.report_id),
            ),
            Err(e) => ToolResult::error(format!("Failed to read report: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_ctx() -> ToolContext {
        ToolContext::default()
    }

    #[tokio::test]
    async fn create_report_requires_title() {
        let result = CreateReportTool
            .execute(
                json!({ "title": "", "content": "something" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("title"));
    }

    #[tokio::test]
    async fn create_report_requires_content() {
        let result = CreateReportTool
            .execute(json!({ "title": "Test", "content": "" }), &default_ctx())
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("content"));
    }

    #[tokio::test]
    async fn create_report_requires_session() {
        let result = CreateReportTool
            .execute(
                json!({ "title": "Test", "content": "Body" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("session"));
    }

    #[tokio::test]
    async fn list_reports_requires_db() {
        let result = ListReportsTool.execute(json!({}), &default_ctx()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn read_report_requires_id() {
        let result = ReadReportTool
            .execute(json!({ "report_id": "" }), &default_ctx())
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn read_report_requires_db() {
        let result = ReadReportTool
            .execute(json!({ "report_id": "some-id" }), &default_ctx())
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn create_report_missing_fields() {
        let result = CreateReportTool.execute(json!({}), &default_ctx()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn read_report_missing_fields() {
        let result = ReadReportTool.execute(json!({}), &default_ctx()).await;
        assert!(result.is_error);
    }
}
