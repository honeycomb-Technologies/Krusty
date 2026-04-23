//! Tool execution for the agentic loop.
//!
//! Handles:
//! - Centralized approval + retry policy via `tool_control`
//! - Special tool dispatch (mode switch, plan tasks)
//! - Regular tool execution via `ToolRegistry::execute()`
//! - Tool output streaming via `ToolOutputChunk` → `LoopEvent::ToolOutputDelta`

mod plan_updates;
mod regular;
mod user_message;

use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::ai::client::AiClient;
use crate::ai::types::{AiToolCall, Content};
use crate::process::ProcessRegistry;
use crate::storage::WorkMode;
use crate::tools::registry::{PermissionMode, ToolRegistry, ToolResult};

use super::loop_events::{LoopEvent, LoopInput};
use super::plan_handler;
use super::tool_control::{AuthorizationDecision, RetryDirective, ToolControl};

use self::plan_updates::emit_plan_update;
use self::regular::execute_regular_tool;
use self::user_message::execute_send_user_message;

/// Execute a batch of tool calls, emitting LoopEvents and receiving LoopInputs
/// for the approval workflow.
///
/// Returns `(tool_results, next_work_mode)`.
pub(crate) async fn execute_tools(
    tool_calls: &[AiToolCall],
    tool_registry: &Arc<ToolRegistry>,
    ai_client: &Arc<AiClient>,
    working_dir: &Path,
    project_dir: Option<&Path>,
    process_registry: &Arc<ProcessRegistry>,
    session_id: &str,
    db_path: &Path,
    user_id: Option<&str>,
    permission_mode: PermissionMode,
    current_mode: WorkMode,
    delegated_progress_tx: Option<&mpsc::UnboundedSender<crate::agent::DelegatedProgressEvent>>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
    subagent_max_turns_override: Option<usize>,
    disabled_tools: Option<&[String]>,
) -> (Vec<Content>, WorkMode) {
    let mut work_mode = current_mode;
    let mut results = Vec::new();
    let tool_control = ToolControl::new(permission_mode);

    for call in tool_calls {
        if let Some(disabled) = disabled_tools {
            if disabled.iter().any(|d| d == call.name.as_str()) {
                let denied = ToolResult::error_with_code(
                    "disabled_by_project",
                    format!("Tool '{}' is disabled in .krusty/settings.json", call.name),
                );
                results.push(tool_control.publish_result(call, &denied, event_tx));
                continue;
            }
        }

        match tool_control.authorize(call, event_tx, input_rx).await {
            AuthorizationDecision::Execute => {}
            AuthorizationDecision::Deny(denial) => {
                let denied = denial.tool_result();
                results.push(tool_control.publish_result(call, &denied, event_tx));
                continue;
            }
        }

        let _ = event_tx.send(LoopEvent::ToolExecuting {
            id: call.id.clone(),
            name: call.name.clone(),
        });

        if call.name == "set_work_mode" || call.name == "enter_plan_mode" {
            let switch = plan_handler::handle_mode_switch(call, session_id, db_path, work_mode);
            work_mode = switch.next_mode;

            if let Some(reason) = switch.mode_change_reason {
                let _ = event_tx.send(LoopEvent::ModeChange {
                    mode: work_mode.to_string(),
                    reason: Some(reason),
                });
            }

            results.push(tool_control.publish_result(call, &switch.tool_result, event_tx));
            continue;
        }

        if matches!(
            call.name.as_str(),
            "task_start" | "task_complete" | "add_subtask" | "set_dependency"
        ) {
            let result = plan_handler::handle_plan_task(call, session_id, db_path);
            if !result.is_error {
                emit_plan_update(session_id, db_path, event_tx);
            }
            results.push(tool_control.publish_result(call, &result, event_tx));
            continue;
        }

        if call.name == "send_user_message" {
            let result = execute_send_user_message(
                call,
                tool_registry,
                working_dir,
                project_dir,
                session_id,
                db_path,
                event_tx,
            )
            .await;
            results.push(tool_control.publish_result(call, &result, event_tx));
            continue;
        }

        let mut retries_attempted = 0usize;
        let result = loop {
            let result = execute_regular_tool(
                call,
                tool_registry,
                ai_client,
                working_dir,
                project_dir,
                process_registry,
                session_id,
                db_path,
                user_id,
                permission_mode,
                work_mode,
                delegated_progress_tx,
                event_tx,
                subagent_max_turns_override,
            )
            .await;

            match tool_control.retry_directive(call, &result, retries_attempted) {
                RetryDirective::Stop => break result,
                RetryDirective::RetryOnce { reason } => {
                    retries_attempted += 1;
                    tracing::warn!(
                        tool = %call.name,
                        tool_call_id = %call.id,
                        retries_attempted,
                        reason,
                        "Retrying tool execution under centralized policy"
                    );
                }
            }
        };

        results.push(tool_control.publish_result(call, &result, event_tx));
    }

    (results, work_mode)
}
