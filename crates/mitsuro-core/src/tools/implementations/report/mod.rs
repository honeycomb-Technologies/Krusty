mod create;
mod list;
mod read;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct ReportTool;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReportAction {
    Create,
    List,
    Read,
}

#[derive(Clone, Deserialize)]
pub(super) struct Params {
    action: ReportAction,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    sources: Option<Vec<String>>,
    #[serde(default)]
    promote_to_memory: bool,
    #[serde(default)]
    memory_type: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    report_id: Option<String>,
}

#[async_trait]
impl Tool for ReportTool {
    fn name(&self) -> &str {
        "report"
    }

    fn description(&self) -> &str {
        "Create, list, or read persistent research reports for the current project."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Manage persistent research reports.

Actions:
- "create": Persist findings, architecture analyses, or investigation results. Requires title and content. Optional: summary, tags, sources, promote_to_memory, memory_type.
- "list": List or search existing reports for the current project. Optional: query.
- "read": Load the full content of a report by ID. Requires report_id.

Use reports for findings worth keeping across sessions. Promote durable conclusions into memory when future runs should retain them automatically."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "read"],
                    "description": "Report operation to perform"
                },
                "title": {
                    "type": "string",
                    "description": "Report title (required for create)"
                },
                "content": {
                    "type": "string",
                    "description": "Full Markdown content of the report (required for create)"
                },
                "summary": {
                    "type": "string",
                    "description": "One-line summary for listing views (optional for create)"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Categorization tags for search (optional for create)"
                },
                "sources": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "References consulted (optional for create)"
                },
                "promote_to_memory": {
                    "type": "boolean",
                    "description": "Also promote the report summary into persistent memory (optional for create)"
                },
                "memory_type": {
                    "type": "string",
                    "enum": ["user", "feedback", "project", "reference"],
                    "description": "Memory type to use when promote_to_memory is true"
                },
                "query": {
                    "type": "string",
                    "description": "Search query for list"
                },
                "report_id": {
                    "type": "string",
                    "description": "Report ID for read"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        match params.action {
            ReportAction::Create => create::execute(params, ctx),
            ReportAction::List => list::execute(params, ctx),
            ReportAction::Read => read::execute(params, ctx),
        }
    }
}
