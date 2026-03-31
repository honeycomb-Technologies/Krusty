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
) -> (Vec<Content>, WorkMode) {
    let mut work_mode = current_mode;
    let mut results = Vec::new();
    let tool_control = ToolControl::new(permission_mode);

    for call in tool_calls {
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
    let tasks = PlanManager::new(db_path.to_path_buf())
        .ok()
        .and_then(|pm| pm.get_plan(session_id).ok().flatten())
        .map(|plan| {
            plan.phases
                .iter()
                .flat_map(|phase| phase.tasks.iter())
                .map(|task| PlanTaskInfo {
                    description: task.description.clone(),
                    completed: task.completed,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let _ = event_tx.send(LoopEvent::PlanUpdate { tasks });
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
