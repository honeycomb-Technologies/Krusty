//! Typed Goal and plan proposal tool.
//!
//! The orchestrator intercepts this tool. It may create or revise draft
//! workflow state, but it can never approve a plan, activate a Goal, change
//! permissions, or mutate project files.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::registry::{Tool, ToolContext, ToolResult};

pub struct WorkflowProposeTool;

#[async_trait]
impl Tool for WorkflowProposeTool {
    fn name(&self) -> &str {
        "workflow_propose"
    }

    fn description(&self) -> &str {
        "Create a typed draft Goal and execution plan, or propose a new plan revision for an existing Goal. This never approves or starts work."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "object",
                    "description": "Required when the session has no Goal. Omit when revising an existing Goal plan.",
                    "properties": {
                        "title": { "type": "string" },
                        "objective": {
                            "type": "string",
                            "description": "Concrete outcome, not an activity description."
                        },
                        "constraints": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "criteria": {
                            "type": "array",
                            "minItems": 1,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "description": { "type": "string" },
                                    "required": { "type": "boolean" }
                                },
                                "required": ["description"],
                                "additionalProperties": false
                            }
                        },
                        "token_budget": {
                            "type": ["integer", "null"],
                            "minimum": 1
                        }
                    },
                    "required": ["title", "objective", "criteria"],
                    "additionalProperties": false
                },
                "goal_id": {
                    "type": "string",
                    "description": "Existing Goal ID when proposing a replacement plan revision."
                },
                "expected_revision": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Current aggregate revision for an existing Goal."
                },
                "plan": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "rationale": { "type": ["string", "null"] },
                        "source_message_id": { "type": ["integer", "null"] },
                        "predecessor_id": { "type": ["string", "null"] },
                        "steps": {
                            "type": "array",
                            "minItems": 1,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "display_key": { "type": "string" },
                                    "description": { "type": "string" },
                                    "context": { "type": ["string", "null"] },
                                    "parent_display_key": { "type": ["string", "null"] },
                                    "dependencies": {
                                        "type": "array",
                                        "items": { "type": "string" }
                                    },
                                    "acceptance_criteria": {
                                        "type": "array",
                                        "items": { "type": "string" }
                                    },
                                    "required": { "type": "boolean" }
                                },
                                "required": ["display_key", "description"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["title", "steps"],
                    "additionalProperties": false
                }
            },
            "required": ["plan"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success_data(json!({
            "note": "Workflow proposal handled by the orchestrator"
        }))
    }
}
