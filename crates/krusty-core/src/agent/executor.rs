//! Tool execution for the agentic loop.
//!
//! Handles:
//! - Centralized approval + retry policy via `tool_control`
//! - Special tool dispatch (mode switch, plan tasks)
//! - Regular tool execution via `ToolRegistry::execute()`
//! - Tool output streaming via `ToolOutputChunk` → `LoopEvent::ToolOutputDelta`

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::agent::subagent::AgentProgress;
use crate::agent::AgentConfig as RuntimeAgentConfig;
use crate::agent::DelegatedRunStage;
use crate::agent::{DelegatedProgressEvent, DelegatedToolKind};
use crate::ai::client::AiClient;
use crate::ai::types::{AiToolCall, Content};
use crate::plan::PlanFile;
use crate::plan::PlanManager;
use crate::process::ProcessRegistry;
use crate::storage::{WorkMode, WorkspaceMode};
use crate::tools::registry::{PermissionMode, ToolContext, ToolRegistry, ToolResult};

use super::loop_events::{LoopEvent, LoopInput, PlanTaskInfo};
use super::plan_handler;
use super::tool_control::{AuthorizationDecision, RetryDirective, ToolControl};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

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
    delegated_progress_tx: Option<&mpsc::UnboundedSender<DelegatedProgressEvent>>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
    subagent_max_turns_override: Option<usize>,
    disabled_tools: Option<&[String]>,
) -> (Vec<Content>, WorkMode) {
    let mut work_mode = current_mode;
    let mut results = Vec::new();
    let tool_control = ToolControl::new(permission_mode);

    for call in tool_calls {
        // Check project-level disabled_tools before any execution
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

        // ── Mode switch tools ──────────────────────────────────────
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

        // ── Plan task tools ────────────────────────────────────────
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

        // ── SendUserMessage interception ──────────────────────────────────
        if call.name == "send_user_message" {
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

            results.push(tool_control.publish_result(call, &result, event_tx));
            continue;
        }

        // ── Regular tool execution ─────────────────────────────────
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

fn emit_plan_update(session_id: &str, db_path: &Path, event_tx: &mpsc::UnboundedSender<LoopEvent>) {
    let tasks = load_plan_update_tasks(session_id, db_path);
    let _ = event_tx.send(LoopEvent::PlanUpdate { tasks });
}

fn load_plan_update_tasks(session_id: &str, db_path: &Path) -> Vec<PlanTaskInfo> {
    match PlanManager::new(db_path.to_path_buf()).and_then(|pm| pm.get_plan(session_id)) {
        Ok(Some(plan)) => plan_task_infos(&plan),
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "Failed to load plan while emitting plan update"
            );
            Vec::new()
        }
    }
}

fn plan_task_infos(plan: &PlanFile) -> Vec<PlanTaskInfo> {
    plan.phases
        .iter()
        .flat_map(|phase| phase.tasks.iter())
        .map(|task| PlanTaskInfo {
            description: task.description.clone(),
            completed: task.completed,
        })
        .collect()
}

async fn execute_regular_tool(
    call: &AiToolCall,
    tool_registry: &Arc<ToolRegistry>,
    ai_client: &Arc<AiClient>,
    working_dir: &Path,
    project_dir: Option<&Path>,
    process_registry: &Arc<ProcessRegistry>,
    session_id: &str,
    db_path: &Path,
    user_id: Option<&str>,
    permission_mode: PermissionMode,
    work_mode: WorkMode,
    delegated_progress_tx: Option<&mpsc::UnboundedSender<DelegatedProgressEvent>>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    subagent_max_turns_override: Option<usize>,
) -> ToolResult {
    let (output_tx, mut output_rx) =
        mpsc::unbounded_channel::<crate::tools::registry::ToolOutputChunk>();

    let forwarder_event_tx = event_tx.clone();
    let forwarder_tool_id = call.id.clone();
    let forwarder_tool_name = call.name.clone();
    let forwarder_handle = tokio::spawn(async move {
        let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat_interval.tick().await;

        loop {
            tokio::select! {
                chunk = output_rx.recv() => {
                    match chunk {
                        Some(chunk) => {
                            if !chunk.chunk.is_empty() {
                                let _ = forwarder_event_tx.send(LoopEvent::ToolOutputDelta {
                                    id: forwarder_tool_id.clone(),
                                    delta: chunk.chunk,
                                });
                            }
                            if chunk.is_complete {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                _ = heartbeat_interval.tick() => {
                    let _ = forwarder_event_tx.send(LoopEvent::ToolExecuting {
                        id: forwarder_tool_id.clone(),
                        name: forwarder_tool_name.clone(),
                    });
                }
            }
        }
    });

    let mut ctx = ToolContext {
        working_dir: working_dir.to_path_buf(),
        project_dir: project_dir.map(Path::to_path_buf),
        workspace_mode: if project_dir.is_some() {
            WorkspaceMode::Selected
        } else {
            WorkspaceMode::Neutral
        },
        session_id: Some(session_id.to_string()),
        db_path: Some(db_path.to_path_buf()),
        process_registry: Some(process_registry.clone()),
        plan_mode: work_mode == WorkMode::Plan,
        user_id: user_id.map(ToString::to_string),
        sandbox_root: Some(working_dir.to_path_buf()),
        ..Default::default()
    }
    .with_permission_mode(permission_mode)
    .with_subagent_max_turns(
        subagent_max_turns_override.or(RuntimeAgentConfig::default().subagent_max_turns),
    )
    .with_ai_client(ai_client.clone())
    .with_tool_registry(Arc::clone(tool_registry))
    .with_loop_event_tx(event_tx.clone())
    .with_output_stream(output_tx, call.id.clone());

    let mut delegated_forwarder_handle = None;
    if matches!(call.name.as_str(), "agent") {
        if let Some(parent_tx) = delegated_progress_tx.cloned() {
            let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<AgentProgress>();
            ctx = ctx.with_agent_progress(progress_tx);

            let tool_call_id = call.id.clone();
            let parent_session_id = session_id.to_string();
            // The unified agent tool determines its own kind internally;
            // default to Explore for the progress-forwarding envelope.
            let kind = DelegatedToolKind::Explore;
            delegated_forwarder_handle = Some(tokio::spawn(async move {
                while let Some(progress) = progress_rx.recv().await {
                    let delegated_run_id = progress
                        .delegated_run_id
                        .clone()
                        .unwrap_or_else(|| tool_call_id.clone());
                    let stage = delegated_stage_from_progress(&progress);
                    let _ = parent_tx.send(DelegatedProgressEvent {
                        delegated_run_id,
                        parent_session_id: parent_session_id.clone(),
                        tool_call_id: tool_call_id.clone(),
                        kind,
                        stage,
                        progress,
                    });
                }
            }));
        }
    }

    let result = tool_registry
        .execute(&call.name, call.arguments.clone(), &ctx)
        .await
        .unwrap_or_else(|| {
            ToolResult::error_with_code("unknown_tool", format!("Unknown tool: {}", call.name))
        });

    drop(ctx);
    if let Some(handle) = delegated_forwarder_handle {
        let _ = handle.await;
    }
    let _ = forwarder_handle.await;
    result
}

fn delegated_stage_from_progress(progress: &AgentProgress) -> DelegatedRunStage {
    use crate::agent::subagent::AgentProgressStatus;

    match progress.status {
        AgentProgressStatus::Running => {
            let action = progress
                .current_action
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if action.contains("starting") {
                DelegatedRunStage::Created
            } else if action.contains("synthesizing") {
                DelegatedRunStage::Synthesizing
            } else {
                DelegatedRunStage::Running
            }
        }
        AgentProgressStatus::Complete => DelegatedRunStage::Complete,
        AgentProgressStatus::Failed => {
            let action = progress
                .current_action
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if action.contains("cancel") {
                DelegatedRunStage::Cancelled
            } else if action.contains("degraded") {
                DelegatedRunStage::Degraded
            } else {
                DelegatedRunStage::Failed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    use super::{emit_plan_update, load_plan_update_tasks};
    use crate::agent::loop_events::LoopEvent;
    use crate::plan::{PlanFile, PlanManager, TaskStatus};
    use crate::storage::SessionManager;
    use crate::Database;

    fn create_test_db() -> (std::path::PathBuf, TempDir) {
        let temp_dir = TempDir::new().expect("tempdir");
        let db_path = temp_dir.path().join("executor.db");
        Database::new(&db_path).expect("database");
        (db_path, temp_dir)
    }

    #[test]
    fn load_plan_update_tasks_returns_saved_plan_tasks() {
        let (db_path, _temp_dir) = create_test_db();
        let session_manager =
            SessionManager::new(Database::new(&db_path).expect("database should open"));
        let session_id = session_manager
            .create_session("Executor Test", None, Some("/tmp"))
            .expect("session should be created");

        let mut plan = PlanFile::new("Executor Plan");
        {
            let phase = plan.add_phase("Phase 1");
            phase.add_task("Keep continuity");
            phase.add_task("Harden plan updates");
        }
        plan.phases[0].tasks[1].completed = true;
        plan.phases[0].tasks[1].status = TaskStatus::Completed;

        let plan_manager = PlanManager::new(db_path.clone()).expect("plan manager");
        plan_manager
            .save_plan_for_session(&session_id, &plan)
            .expect("plan should save");

        let tasks = load_plan_update_tasks(&session_id, &db_path);

        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].description, "Keep continuity");
        assert!(!tasks[0].completed);
        assert_eq!(tasks[1].description, "Harden plan updates");
        assert!(tasks[1].completed);
    }

    #[test]
    fn load_plan_update_tasks_returns_empty_when_plan_store_unavailable() {
        let temp_dir = TempDir::new().expect("tempdir");
        let missing_db_path = temp_dir.path().join("missing").join("executor.db");

        let tasks = load_plan_update_tasks("session-1", &missing_db_path);

        assert!(tasks.is_empty());
    }

    #[test]
    fn emit_plan_update_sends_plan_tasks() {
        let (db_path, _temp_dir) = create_test_db();
        let session_manager =
            SessionManager::new(Database::new(&db_path).expect("database should open"));
        let session_id = session_manager
            .create_session("Executor Test", None, Some("/tmp"))
            .expect("session should be created");

        let mut plan = PlanFile::new("Executor Plan");
        plan.add_phase("Phase 1").add_task("Emit update");
        PlanManager::new(db_path.clone())
            .expect("plan manager")
            .save_plan_for_session(&session_id, &plan)
            .expect("plan should save");

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        emit_plan_update(&session_id, &db_path, &event_tx);

        let event = event_rx.try_recv().expect("plan update event");
        assert!(matches!(
            event,
            LoopEvent::PlanUpdate { tasks }
            if tasks.len() == 1 && tasks[0].description == "Emit update" && !tasks[0].completed
        ));
    }
}
