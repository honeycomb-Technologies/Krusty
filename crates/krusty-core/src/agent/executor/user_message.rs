use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::ai::types::AiToolCall;
use crate::storage::WorkspaceMode;
use crate::tools::registry::{ToolContext, ToolRegistry, ToolResult};

use super::super::loop_events::LoopEvent;

pub(super) async fn execute_send_user_message(
    call: &AiToolCall,
    tool_registry: &Arc<ToolRegistry>,
    working_dir: &Path,
    project_dir: Option<&Path>,
    session_id: &str,
    db_path: &Path,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
) -> ToolResult {
    let msg_ctx = ToolContext {
        working_dir: working_dir.to_path_buf(),
        project_dir: project_dir.map(Path::to_path_buf),
        workspace_mode: if project_dir.is_some() {
            WorkspaceMode::Selected
        } else {
            WorkspaceMode::Neutral
        },
        session_id: Some(session_id.to_string()),
        db_path: Some(db_path.to_path_buf()),
        ..Default::default()
    };

    let result = tool_registry
        .execute(&call.name, call.arguments.clone(), &msg_ctx)
        .await
        .unwrap_or_else(|| {
            ToolResult::error_with_code("unknown_tool", "SendUserMessage not registered")
        });

    if !result.is_error {
        let title = call
            .arguments
            .get("title")
            .and_then(|v| v.as_str())
            .map(String::from);
        let message = call
            .arguments
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let level = call
            .arguments
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("info")
            .to_string();
        let _ = event_tx.send(LoopEvent::UserMessage {
            title,
            message,
            level,
        });
    }

    result
}
