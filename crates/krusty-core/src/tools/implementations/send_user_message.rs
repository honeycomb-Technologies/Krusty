use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

const VALID_LEVELS: &[&str] = &["info", "success", "warning", "error"];

pub struct SendUserMessageTool;

#[derive(Deserialize)]
struct Params {
    message: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default = "default_level")]
    level: String,
}

fn default_level() -> String {
    "info".to_string()
}

#[async_trait]
impl Tool for SendUserMessageTool {
    fn name(&self) -> &str {
        "send_user_message"
    }

    fn description(&self) -> &str {
        "Send a prominent message to the user. In autonomous mode, regular AI text is dimmed — use this tool to deliver highlighted, user-facing communication."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use this to communicate important information directly to the user during autonomous execution. Regular assistant text is dimmed in Mako mode — only SendUserMessage content is highlighted and prominent.

When to use:
- Reporting completion of a significant milestone or task
- Surfacing a decision you made that the user should know about
- Warnings about unexpected conditions or potential issues
- Errors that need user awareness but don't require stopping

When NOT to use:
- Routine progress updates (use regular text)
- Every tool result or minor step
- Asking questions (use AskUserQuestion instead)

Levels:
- "info" (default): Neutral updates, status reports, completions
- "success": Task completed successfully, milestone reached
- "warning": Something unexpected happened but work continues
- "error": Something failed or needs attention

Keep messages concise and actionable. The title (if provided) appears as a header."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message content to display prominently to the user"
                },
                "title": {
                    "type": "string",
                    "description": "Optional header/title for the message"
                },
                "level": {
                    "type": "string",
                    "enum": ["info", "success", "warning", "error"],
                    "description": "Message severity level (default: info)"
                }
            },
            "required": ["message"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if params.message.is_empty() {
            return ToolResult::invalid_parameters("message must not be empty");
        }

        if !VALID_LEVELS.contains(&params.level.as_str()) {
            return ToolResult::invalid_parameters(format!(
                "Invalid level '{}'. Valid levels: info, success, warning, error",
                params.level
            ));
        }

        ToolResult::success_data(json!({
            "delivered": true,
            "title": params.title,
            "level": params.level,
            "message_length": params.message.len(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_ctx() -> ToolContext {
        ToolContext::default()
    }

    #[tokio::test]
    async fn basic_message() {
        let result = SendUserMessageTool
            .execute(json!({ "message": "Build complete" }), &default_ctx())
            .await;
        assert!(!result.is_error, "{}", result.output);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["delivered"], true);
        assert_eq!(parsed["data"]["level"], "info");
        assert_eq!(parsed["data"]["message_length"], 14);
    }

    #[tokio::test]
    async fn with_title_and_level() {
        let result = SendUserMessageTool
            .execute(
                json!({
                    "message": "All tests passing",
                    "title": "CI Status",
                    "level": "success"
                }),
                &default_ctx(),
            )
            .await;
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["title"], "CI Status");
        assert_eq!(parsed["data"]["level"], "success");
    }

    #[tokio::test]
    async fn empty_message_rejected() {
        let result = SendUserMessageTool
            .execute(json!({ "message": "" }), &default_ctx())
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn invalid_level_rejected() {
        let result = SendUserMessageTool
            .execute(
                json!({ "message": "hello", "level": "critical" }),
                &default_ctx(),
            )
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn missing_message_rejected() {
        let result = SendUserMessageTool.execute(json!({}), &default_ctx()).await;
        assert!(result.is_error);
    }
}
