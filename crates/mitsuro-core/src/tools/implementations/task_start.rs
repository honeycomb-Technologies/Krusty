//! Task Start tool - Mark a task as in-progress
//!
//! This tool is intercepted by the UI and handled specially.
//! It marks a task as in-progress and validates it's not blocked.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct TaskStartTool;

#[async_trait]
impl Tool for TaskStartTool {
    fn name(&self) -> &str {
        "task_start"
    }

    fn description(&self) -> &str {
        "Mark a task as in-progress before beginning work. Fails if the task is blocked by incomplete dependencies."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID to start working on (e.g., '1.1')"
                }
            },
            "required": ["task_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        // This tool is handled specially by the UI - this code shouldn't run
        ToolResult {
            output: json!({ "note": "Task start handled by UI" }).to_string(),
            is_error: false,
        }
    }
}
