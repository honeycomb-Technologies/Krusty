//! Add Subtask tool - Create a subtask under an existing task
//!
//! This tool is intercepted by the UI and handled specially.
//! It creates a new subtask with auto-generated ID and updates the parent.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct AddSubtaskTool;

#[async_trait]
impl Tool for AddSubtaskTool {
    fn name(&self) -> &str {
        "add_subtask"
    }

    fn description(&self) -> &str {
        "Add a subtask to break down a complex task. Creates a child task with auto-generated ID (e.g., '1.1' -> '1.1.1')."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "parent_id": {
                    "type": "string",
                    "description": "ID of the parent task (e.g., '1.1')"
                },
                "description": {
                    "type": "string",
                    "description": "Description of the subtask"
                },
                "context": {
                    "type": "string",
                    "description": "Optional implementation details or notes"
                }
            },
            "required": ["parent_id", "description"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        // This tool is handled specially by the UI - this code shouldn't run
        ToolResult {
            output: json!({ "note": "Subtask creation handled by UI" }).to_string(),
            is_error: false,
        }
    }
}
