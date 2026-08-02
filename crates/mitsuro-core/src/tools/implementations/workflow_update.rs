//! Typed verification and completion commands for an active durable Goal.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct WorkflowUpdateTool;

#[async_trait]
impl Tool for WorkflowUpdateTool {
    fn name(&self) -> &str {
        "workflow_update"
    }

    fn description(&self) -> &str {
        "Record evidence against a Goal verification criterion or complete the Goal after every required plan step and criterion is satisfied."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["verify_criterion", "complete_goal"]
                },
                "goal_id": { "type": "string" },
                "expected_revision": { "type": "integer", "minimum": 1 },
                "criterion_id": { "type": "string" },
                "status": {
                    "type": "string",
                    "enum": ["passed", "failed"]
                },
                "evidence": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": ["action", "goal_id", "expected_revision"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success_data(json!({
            "note": "Workflow update handled by the orchestrator"
        }))
    }
}
