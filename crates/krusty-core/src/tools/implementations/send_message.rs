use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::tools::parse_params;
use crate::tools::registry::{Tool, ToolContext, ToolResult};

const VALID_MESSAGE_TYPES: &[&str] = &["text", "shutdown_request", "shutdown_response"];

pub struct SendMessageTool;

#[derive(Deserialize)]
struct Params {
    to: String,
    message: String,
    #[serde(default = "default_message_type")]
    message_type: String,
    #[serde(default)]
    from: Option<String>,
}

fn default_message_type() -> String {
    "text".to_string()
}

fn mailbox_dir(working_dir: &std::path::Path) -> PathBuf {
    working_dir.join(".krusty").join("mailbox")
}

fn write_message(path: &PathBuf, line: &str) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a message to a named teammate agent via the file-based mailbox system. Supports direct messages and broadcast."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            r#"Use this to communicate with your teammate agents during autonomous Mako execution.

When to use:
- Sending task assignments or instructions to a specific teammate
- Broadcasting status updates to all teammates
- Coordinating work between agents (e.g., "builder-1, the API is ready for integration")
- Sending shutdown requests to gracefully stop teammates

Message types:
- "text" (default): Regular communication between agents
- "shutdown_request": Ask a teammate to finish up and stop
- "shutdown_response": Acknowledge a shutdown request

Addressing:
- Use the teammate's name (e.g., "builder-1") for direct messages
- Use "*" to broadcast to all teammates

The "from" field is optional — it defaults to "coordinator" if not provided. Teammates should set it to their own name."#,
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient agent name, or \"*\" for broadcast to all agents"
                },
                "message": {
                    "type": "string",
                    "description": "The message content to send"
                },
                "message_type": {
                    "type": "string",
                    "enum": ["text", "shutdown_request", "shutdown_response"],
                    "description": "Message type (default: text)"
                },
                "from": {
                    "type": "string",
                    "description": "Sender name override (defaults to coordinator)"
                }
            },
            "required": ["to", "message"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let params = match parse_params::<Params>(params) {
            Ok(p) => p,
            Err(e) => return e,
        };

        if params.to.is_empty() {
            return ToolResult::invalid_parameters("'to' must not be empty");
        }

        if params.message.is_empty() {
            return ToolResult::invalid_parameters("'message' must not be empty");
        }

        if !VALID_MESSAGE_TYPES.contains(&params.message_type.as_str()) {
            return ToolResult::invalid_parameters(format!(
                "Invalid message_type '{}'. Must be one of: text, shutdown_request, shutdown_response",
                params.message_type
            ));
        }

        let sender = params.from.unwrap_or_else(|| "coordinator".to_string());
        let dir = mailbox_dir(&ctx.working_dir);

        if let Err(e) = fs::create_dir_all(&dir) {
            return ToolResult::error(format!("Failed to create mailbox directory: {e}"));
        }

        let line = json!({
            "from": sender,
            "message": params.message,
            "message_type": params.message_type,
            "timestamp": Utc::now().to_rfc3339(),
        })
        .to_string();

        if params.to == "*" {
            return self.broadcast(&dir, &sender, &line);
        }

        let path = dir.join(format!("{}.jsonl", params.to));
        if let Err(e) = write_message(&path, &line) {
            return ToolResult::error(format!("Failed to write to mailbox: {e}"));
        }

        ToolResult::success_data(json!({
            "delivered": true,
            "to": params.to,
        }))
    }
}

impl SendMessageTool {
    fn broadcast(&self, dir: &PathBuf, sender: &str, line: &str) -> ToolResult {
        let sender_file = format!("{sender}.jsonl");
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => {
                return ToolResult::success_data(json!({
                    "delivered": true,
                    "to": "*",
                    "recipients": 0,
                }));
            }
        };

        let mut count = 0u32;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.ends_with(".jsonl") || name_str.as_ref() == sender_file {
                continue;
            }
            if let Err(e) = write_message(&entry.path(), line) {
                return ToolResult::error(format!("Broadcast failed writing to {}: {e}", name_str));
            }
            count += 1;
        }

        ToolResult::success_data(json!({
            "delivered": true,
            "to": "*",
            "recipients": count,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ctx_in(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            working_dir: dir.to_path_buf(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn direct_message() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_in(tmp.path());

        let result = SendMessageTool
            .execute(
                json!({ "to": "builder-1", "message": "start the tests" }),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.output);

        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["delivered"], true);
        assert_eq!(parsed["data"]["to"], "builder-1");

        let mailbox = tmp.path().join(".krusty/mailbox/builder-1.jsonl");
        let contents = fs::read_to_string(mailbox).unwrap();
        let msg: Value = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(msg["from"], "coordinator");
        assert_eq!(msg["message"], "start the tests");
        assert_eq!(msg["message_type"], "text");
        assert!(msg["timestamp"].as_str().is_some());
    }

    #[tokio::test]
    async fn custom_sender() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_in(tmp.path());

        let result = SendMessageTool
            .execute(
                json!({ "to": "coordinator", "message": "done", "from": "builder-1" }),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.output);

        let mailbox = tmp.path().join(".krusty/mailbox/coordinator.jsonl");
        let msg: Value = serde_json::from_str(fs::read_to_string(mailbox).unwrap().trim()).unwrap();
        assert_eq!(msg["from"], "builder-1");
    }

    #[tokio::test]
    async fn shutdown_request() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_in(tmp.path());

        let result = SendMessageTool
            .execute(
                json!({
                    "to": "builder-1",
                    "message": "wrap up",
                    "message_type": "shutdown_request"
                }),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.output);

        let mailbox = tmp.path().join(".krusty/mailbox/builder-1.jsonl");
        let msg: Value = serde_json::from_str(fs::read_to_string(mailbox).unwrap().trim()).unwrap();
        assert_eq!(msg["message_type"], "shutdown_request");
    }

    #[tokio::test]
    async fn broadcast_to_existing_mailboxes() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_in(tmp.path());
        let dir = tmp.path().join(".krusty/mailbox");
        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join("builder-1.jsonl"), "").unwrap();
        fs::write(dir.join("builder-2.jsonl"), "").unwrap();
        fs::write(dir.join("coordinator.jsonl"), "").unwrap();

        let result = SendMessageTool
            .execute(
                json!({ "to": "*", "message": "sync up", "from": "coordinator" }),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.output);

        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["to"], "*");
        assert_eq!(parsed["data"]["recipients"], 2);

        let coord_contents = fs::read_to_string(dir.join("coordinator.jsonl")).unwrap();
        assert!(coord_contents.is_empty(), "sender should be excluded");
    }

    #[tokio::test]
    async fn broadcast_empty_mailbox_dir() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_in(tmp.path());

        let result = SendMessageTool
            .execute(json!({ "to": "*", "message": "hello" }), &ctx)
            .await;
        assert!(!result.is_error, "{}", result.output);

        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["data"]["recipients"], 0);
    }

    #[tokio::test]
    async fn empty_to_rejected() {
        let tmp = TempDir::new().unwrap();
        let result = SendMessageTool
            .execute(json!({ "to": "", "message": "hi" }), &ctx_in(tmp.path()))
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn empty_message_rejected() {
        let tmp = TempDir::new().unwrap();
        let result = SendMessageTool
            .execute(
                json!({ "to": "builder-1", "message": "" }),
                &ctx_in(tmp.path()),
            )
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn invalid_message_type_rejected() {
        let tmp = TempDir::new().unwrap();
        let result = SendMessageTool
            .execute(
                json!({ "to": "builder-1", "message": "hi", "message_type": "urgent" }),
                &ctx_in(tmp.path()),
            )
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn missing_required_fields() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_in(tmp.path());

        let no_to = SendMessageTool
            .execute(json!({ "message": "hi" }), &ctx)
            .await;
        assert!(no_to.is_error);

        let no_msg = SendMessageTool
            .execute(json!({ "to": "builder-1" }), &ctx)
            .await;
        assert!(no_msg.is_error);

        let empty = SendMessageTool.execute(json!({}), &ctx).await;
        assert!(empty.is_error);
    }

    #[tokio::test]
    async fn multiple_messages_append() {
        let tmp = TempDir::new().unwrap();
        let ctx = ctx_in(tmp.path());

        SendMessageTool
            .execute(json!({ "to": "builder-1", "message": "first" }), &ctx)
            .await;
        SendMessageTool
            .execute(json!({ "to": "builder-1", "message": "second" }), &ctx)
            .await;

        let mailbox = tmp.path().join(".krusty/mailbox/builder-1.jsonl");
        let contents = fs::read_to_string(&mailbox).unwrap();
        let lines: Vec<&str> = contents.trim().split('\n').collect();
        assert_eq!(lines.len(), 2);
    }
}
