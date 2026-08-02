//! Set Work Mode tool - explicit mode transitions
//!
//! This tool is intercepted by the UI/server and handled specially.
//! It allows switching between build and plan modes.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct SetWorkModeTool;

#[async_trait]
impl Tool for SetWorkModeTool {
    fn name(&self) -> &str {
        "set_work_mode"
    }

    fn description(&self) -> &str {
        "Switch between build and plan modes. Use mode='plan' to plan before implementation, and mode='build' to execute changes."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["build", "plan"],
                    "description": "Target mode"
                },
                "reason": {
                    "type": "string",
                    "description": "Optional brief explanation shown to user"
                },
                "clear_existing": {
                    "type": "boolean",
                    "description": "If true and mode='plan', clears any existing active plan before switching"
                }
            },
            "required": ["mode"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        // This tool is handled specially by the UI/server.
        ToolResult {
            output: json!({
                "note": "Work mode switch handled by caller"
            })
            .to_string(),
            is_error: false,
        }
    }
}
