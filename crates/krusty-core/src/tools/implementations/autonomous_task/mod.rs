mod common;
mod create;
mod list;
mod update;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct AutonomousTaskTool;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TaskAction {
    Create,
    Update,
    List,
}

#[derive(Clone, Deserialize)]
pub(super) struct Params {
    action: TaskAction,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    blocked_by: Option<Vec<String>>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    transition: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    status_filter: Option<String>,
}

#[async_trait]
impl Tool for AutonomousTaskTool {
    fn name(&self) -> &str {
        "autonomous_task"
    }

    fn description(&self) -> &str {
        "Create, update, or list autonomous coordination tasks for the current session."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Manage the autonomous task ledger.

Actions:
- "create": Add a new trackable task. Requires subject. Optional: description, blocked_by.
- "update": Transition an existing task. Requires task_id and transition. Valid transitions: claim, complete, fail. Optional: owner for claim, result for complete/fail.
- "list": Show current tasks. Optional: status_filter = pending | in_progress | completed | failed.

Use this ledger to keep autonomous work explicit, traceable, and verifiable."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "update", "list"],
                    "description": "Task ledger operation to perform"
                },
                "subject": {
                    "type": "string",
                    "description": "Task subject (required for create)"
                },
                "description": {
                    "type": "string",
                    "description": "Task description (optional for create)"
                },
                "blocked_by": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Task IDs that must complete before this task can start"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (required for update)"
                },
                "transition": {
                    "type": "string",
                    "enum": ["claim", "complete", "fail"],
                    "description": "State transition to apply during update"
                },
                "owner": {
                    "type": "string",
                    "description": "Owner name for claim transition"
                },
                "result": {
                    "type": "string",
                    "description": "Outcome description for complete/fail transitions"
                },
                "status_filter": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "failed"],
                    "description": "Optional filter for list"
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
            TaskAction::Create => create::execute(params, ctx),
            TaskAction::Update => update::execute(params, ctx),
            TaskAction::List => list::execute(params, ctx),
        }
    }
}
