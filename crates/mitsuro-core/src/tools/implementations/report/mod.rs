mod create;
mod list;
mod read;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::storage::{
    load_group_worker_lane_with_conn, resolve_worker_conversation_with_conn, Database, ReportScope,
};
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

pub(super) struct ReportReaderScope {
    pub(super) user_id: Option<String>,
    pub(super) worker_id: Option<String>,
    pub(super) worker_memory_namespace_id: Option<String>,
}

impl ReportReaderScope {
    pub(super) fn report_scope(&self) -> anyhow::Result<ReportScope> {
        match (
            self.worker_id.as_deref(),
            self.worker_memory_namespace_id.as_deref(),
        ) {
            (None, None) => Ok(ReportScope::owner_shared()),
            (Some(worker_id), Some(namespace_id)) => {
                ReportScope::worker_private(worker_id, namespace_id).map_err(anyhow::Error::msg)
            }
            _ => anyhow::bail!("Worker report scope is incomplete"),
        }
    }
}

/// Resolve the durable conversation identity used by report read paths.
///
/// Ordinary Chat/Code and primary Hive sessions receive an owner-shared
/// reader. Worker DMs receive that Worker's private extension. Internal group
/// lanes are accepted only while executing the matching group run and only
/// after the persisted lane passes its membership, owner, and session checks.
/// This makes a missing or malformed lane an error rather than silently
/// falling back to owner-wide report access.
pub(super) fn resolve_reader_scope(
    ctx: &ToolContext,
    db: &Database,
) -> anyhow::Result<ReportReaderScope> {
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("report access requires an active session"))?;
    let session = db
        .conn()
        .query_row(
            "SELECT user_id, session_type FROM sessions WHERE id = ?1",
            [session_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("report access session does not exist"))?;
    anyhow::ensure!(
        session.0.as_deref() == ctx.user_id.as_deref(),
        "report access does not match the session owner"
    );

    let binding = resolve_worker_conversation_with_conn(db.conn(), session_id)?;
    let Some(binding) = binding else {
        anyhow::ensure!(
            ctx.hive_group_run.is_none(),
            "group Worker report access has no persisted lane binding"
        );
        return Ok(ReportReaderScope {
            user_id: session.0,
            worker_id: None,
            worker_memory_namespace_id: None,
        });
    };
    anyhow::ensure!(
        binding.worker.user_id.as_deref() == session.0.as_deref(),
        "Worker report access does not match the session owner"
    );
    anyhow::ensure!(
        session.1 == "hive",
        "Worker report access requires a Hive session"
    );

    match (binding.group_id.as_deref(), ctx.hive_group_run.as_ref()) {
        (None, None) => Ok(ReportReaderScope {
            user_id: session.0,
            worker_id: Some(binding.worker.id),
            worker_memory_namespace_id: Some(binding.worker.memory_namespace_id),
        }),
        (None, Some(_)) => anyhow::bail!("group Worker report access is bound to a direct DM"),
        (Some(_), None) => {
            anyhow::bail!("internal group Worker lanes require an explicit group run scope")
        }
        (Some(group_id), Some(run)) => {
            anyhow::ensure!(
                run.group_id == group_id && run.worker_id == binding.worker.id,
                "group Worker report access does not match its persisted lane"
            );
            let lane = load_group_worker_lane_with_conn(db.conn(), &run.group_id, &run.worker_id)?
                .ok_or_else(|| {
                    anyhow::anyhow!("group Worker report access has no persisted lane binding")
                })?;
            anyhow::ensure!(
                lane.session_id == session_id,
                "group Worker report access does not match its persisted lane"
            );
            Ok(ReportReaderScope {
                user_id: session.0,
                worker_id: Some(binding.worker.id),
                worker_memory_namespace_id: Some(binding.worker.memory_namespace_id),
            })
        }
    }
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
