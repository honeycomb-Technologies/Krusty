//! Task Complete tool - Mark a single plan task as complete
//!
//! This tool is intercepted by the UI and handled specially.
//! ENFORCES: Task must be InProgress (use task_start first)
//! ENFORCES: One task at a time (no batch completion)
//! ENFORCES: Result parameter required

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct TaskCompleteTool;

#[async_trait]
impl Tool for TaskCompleteTool {
    fn name(&self) -> &str {
        "task_complete"
    }

    fn description(&self) -> &str {
        "Complete ONE task that you have started with task_start. The task MUST be in-progress. Provide a specific result describing what you accomplished for THIS task."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to complete (must have been started with task_start)"
                },
                "result": {
                    "type": "string",
                    "description": "REQUIRED: Specific summary of what was accomplished for THIS task"
                },
                "evidence": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Concrete validation evidence such as tests, diffs, measurements, or resolved decisions"
                }
            },
            "required": ["task_id", "result"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        // This tool is handled specially by the UI - this code shouldn't run
        ToolResult {
            output: json!({ "note": "Task completion handled by UI" }).to_string(),
            is_error: false,
        }
    }
}
