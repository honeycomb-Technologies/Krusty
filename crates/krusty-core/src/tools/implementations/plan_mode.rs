//! Plan Mode tool - AI-controlled mode switching
//!
//! This tool is intercepted by the UI and handled specially.
//! It allows the AI to switch into Plan mode when the user requests planning.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct EnterPlanModeTool;

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "enter_plan_mode"
    }

    fn description(&self) -> &str {
        "Enter read-only Plan mode before implementation; clear_existing starts a fresh plan."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Why planning is needed"
                },
                "clear_existing": {
                    "type": "boolean",
                    "description": "Discard the current plan first"
                }
            },
            "required": ["reason"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        // This tool is handled specially by the UI - this code shouldn't run
        ToolResult {
            output: json!({
                "note": "Plan mode switch handled by UI"
            })
            .to_string(),
            is_error: false,
        }
    }
}
