//! AskUserQuestion tool - Interactive user prompts
//!
//! This tool allows Claude to ask the user clarifying questions with
//! multiple choice options. It's intercepted in the UI layer (streaming.rs)
//! and never actually executes here - the UI handles showing the prompt
//! and sending the tool result back.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::registry::Tool;
use crate::tools::{ToolContext, ToolResult};

pub struct AskUserQuestionTool;

#[async_trait]
impl Tool for AskUserQuestionTool {
    fn name(&self) -> &str {
        "AskUserQuestion"
    }

    fn description(&self) -> &str {
        "Ask blocking questions and present 2-4 options when useful."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use sparingly — only when you genuinely cannot proceed without user input. If you can make a reasonable judgment call, do so instead of asking.

When to ask:
- Requirements are ambiguous and guessing wrong wastes significant effort
- Multiple valid approaches exist and user preference matters
- You need confirmation before a destructive or irreversible action

When NOT to ask:
- Routine decisions (file naming, import ordering, formatting)
- Questions you could answer by reading the codebase
- Confirmation of actions the user already requested

Keep questions under 500 characters. Provide 2-4 distinct options with brief trade-off descriptions. For simple yes/no, just ask in chat text instead.

Questions timeout after 5 minutes if the user doesn't respond."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "header": {
                                "type": "string",
                                "description": "Short title (max 20 characters)"
                            },
                            "question": {
                                "type": "string"
                            },
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {
                                            "type": "string"
                                        },
                                        "description": {
                                            "type": "string"
                                        }
                                    },
                                    "required": ["label"],
                                    "additionalProperties": false
                                }
                            },
                            "multiSelect": {
                                "type": "boolean",
                                "description": "Allow multiple selections"
                            }
                        },
                        "required": ["header", "question", "options"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
        // This tool is intercepted in streaming.rs and handled by the UI.
        // If we get here, something went wrong with the interception.
        ToolResult::error("AskUserQuestion should be handled by UI layer, not executed directly")
    }
}
