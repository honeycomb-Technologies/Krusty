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
use crate::storage::{
    Database, PartialAssistantState, PendingInteractionSnapshot, RecoveryDecision,
    RecoveryNonResumableReason, RecoveryStatus, SessionManager, SessionRecoveryState, WorkMode,
};
use crate::tools::registry::{FileObservationTracker, PermissionMode, ToolRegistry, ToolResult};

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
    recovery_partial_assistant: Option<&PartialAssistantState>,
    delegated_progress_tx: Option<&mpsc::UnboundedSender<crate::agent::DelegatedProgressEvent>>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
    subagent_max_turns_override: Option<usize>,
    disabled_tools: Option<&[String]>,
    file_observations: Arc<FileObservationTracker>,
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

        let requires_approval = tool_control.requires_approval(call);
        if requires_approval {
            persist_pending_tool_approval_recovery(
                db_path,
                session_id,
                recovery_partial_assistant,
                call,
            );
        }

        match tool_control.authorize(call, event_tx, input_rx).await {
            AuthorizationDecision::Execute => {
                if requires_approval {
                    persist_tool_executing_recovery_after_approval(
                        db_path,
                        session_id,
                        recovery_partial_assistant,
                        call,
                    );
                }
            }
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
                Arc::clone(&file_observations),
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

fn persist_pending_tool_approval_recovery(
    db_path: &Path,
    session_id: &str,
    partial_assistant: Option<&PartialAssistantState>,
    call: &AiToolCall,
) {
    let Some(partial_assistant) = partial_assistant else {
        return;
    };

    let recovery = SessionRecoveryState::new_with_pending_interactions(
        RecoveryStatus::AwaitingInput,
        Some(super::loop_events::LoopStopReason::AwaitingInput),
        None,
        partial_assistant.clone(),
        vec![PendingInteractionSnapshot::tool_approval_from_call(
            &call.id,
            &call.name,
            &call.arguments,
        )],
        RecoveryDecision::NonResumable {
            reason: RecoveryNonResumableReason::AwaitingHumanInput,
        },
    );

    let result = Database::new(db_path).and_then(|db| {
        let manager = SessionManager::new(db);
        manager.update_recovery_state(session_id, &recovery)
    });

    if let Err(error) = result {
        tracing::warn!(
            session_id = %session_id,
            tool_call_id = %call.id,
            tool_name = %call.name,
            "Failed to persist pending tool approval recovery snapshot: {error}"
        );
    }
}

fn persist_tool_executing_recovery_after_approval(
    db_path: &Path,
    session_id: &str,
    partial_assistant: Option<&PartialAssistantState>,
    call: &AiToolCall,
) {
    let Some(partial_assistant) = partial_assistant else {
        return;
    };

    let recovery = SessionRecoveryState::new(
        RecoveryStatus::ToolExecuting,
        None,
        None,
        partial_assistant.clone(),
        RecoveryDecision::NonResumable {
            reason: RecoveryNonResumableReason::ToolExecutionInProgress,
        },
    );

    let result = Database::new(db_path).and_then(|db| {
        let manager = SessionManager::new(db);
        manager.update_recovery_state(session_id, &recovery)
    });

    if let Err(error) = result {
        tracing::warn!(
            session_id = %session_id,
            tool_call_id = %call.id,
            tool_name = %call.name,
            "Failed to persist tool-executing recovery snapshot after approval: {error}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::RecoveryToolCall;
    use serde_json::json;
    use tempfile::TempDir;

    fn create_session_db() -> (TempDir, std::path::PathBuf, String) {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let db_path = temp_dir.path().join("krusty.db");
        let db = Database::new(&db_path).expect("database should initialize");
        let manager = SessionManager::new(db);
        let session_id = manager
            .create_session("approval recovery", Some("gpt-5"), Some("/tmp"))
            .expect("session should be created");
        (temp_dir, db_path, session_id)
    }

    #[test]
    fn approved_tool_recovery_transition_marks_tool_executing_without_pending_prompt() {
        let (_temp_dir, db_path, session_id) = create_session_db();
        let call = AiToolCall {
            id: "call-edit".to_string(),
            name: "edit".to_string(),
            arguments: json!({"file_path": "src/lib.rs"}),
        };
        let partial_assistant = PartialAssistantState {
            text: "I will edit the file.".to_string(),
            thinking: String::new(),
            tool_calls: vec![RecoveryToolCall::from_call_parts(
                &call.id,
                &call.name,
                &call.arguments,
            )],
        };

        persist_pending_tool_approval_recovery(
            &db_path,
            &session_id,
            Some(&partial_assistant),
            &call,
        );
        persist_tool_executing_recovery_after_approval(
            &db_path,
            &session_id,
            Some(&partial_assistant),
            &call,
        );

        let db = Database::new(&db_path).expect("database should reopen");
        let manager = SessionManager::new(db);
        let loaded = manager
            .load_recovery_state(&session_id)
            .expect("recovery load should succeed")
            .expect("recovery state should be present");

        assert_eq!(loaded.status, RecoveryStatus::ToolExecuting);
        assert!(loaded.pending_interactions.is_empty());
        assert_eq!(
            loaded.decision,
            RecoveryDecision::NonResumable {
                reason: RecoveryNonResumableReason::ToolExecutionInProgress
            }
        );
        assert_eq!(
            loaded.partial_assistant.tool_calls[0].arguments.value["file_path"],
            "src/lib.rs"
        );
    }
}
