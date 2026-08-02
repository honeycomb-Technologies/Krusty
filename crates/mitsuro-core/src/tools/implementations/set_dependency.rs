//! Set Dependency tool - Create a dependency between tasks
//!
//! This tool is intercepted by the UI and handled specially.
//! It establishes that one task must complete before another can start.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct SetDependencyTool;

#[async_trait]
impl Tool for SetDependencyTool {
    fn name(&self) -> &str {
        "set_dependency"
    }

    fn description(&self) -> &str {
        "Set a dependency between tasks. The task specified by task_id will be blocked until blocked_by task completes. Prevents circular dependencies."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "ID of the task that will be blocked (e.g., '2.1')"
                },
                "blocked_by": {
                    "type": "string",
                    "description": "ID of the task that must complete first (e.g., '1.1')"
                }
            },
            "required": ["task_id", "blocked_by"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        // This tool is handled specially by the UI - this code shouldn't run
        ToolResult {
            output: json!({ "note": "Dependency setting handled by UI" }).to_string(),
            is_error: false,
        }
    }
}
