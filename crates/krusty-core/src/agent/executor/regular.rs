use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};

use crate::agent::subagent::{AgentProgress, AgentProgressStatus};
use crate::agent::ProviderCallTraceContext;
use crate::agent::{DelegatedProgressEvent, DelegatedRunStage, DelegatedToolKind};
use crate::ai::client::AiClient;
use crate::ai::types::AiToolCall;
use crate::process::ProcessRegistry;
use crate::skills::SkillsManager;
use crate::storage::{WorkMode, WorkspaceMode};
use crate::tools::registry::{
    FileObservationTracker, PermissionMode, ToolContext, ToolRegistry, ToolResult,
};

use super::super::loop_events::LoopEvent;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

pub(super) async fn execute_regular_tool(
    call: &AiToolCall,
    tool_registry: &Arc<ToolRegistry>,
    ai_client: &Arc<AiClient>,
    working_dir: &Path,
    project_dir: Option<&Path>,
    process_registry: &Arc<ProcessRegistry>,
    skills_manager: &Arc<RwLock<SkillsManager>>,
    session_id: &str,
    db_path: &Path,
    user_id: Option<&str>,
    permission_mode: PermissionMode,
    work_mode: WorkMode,
    delegated_progress_tx: Option<&mpsc::UnboundedSender<DelegatedProgressEvent>>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    provider_call_trace: Option<&ProviderCallTraceContext>,
    subagent_max_turns_override: Option<usize>,
    file_observations: Arc<FileObservationTracker>,
    extension_intercept_prepared: bool,
) -> ToolResult {
    let background_agent = call.name == "agent" && agent_runs_in_background(&call.arguments);
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
    .with_subagent_max_turns(subagent_max_turns_override)
    .with_ai_client(ai_client.clone())
    .with_skills_manager(Arc::clone(skills_manager))
    .with_tool_registry(Arc::clone(tool_registry))
    .with_loop_event_tx(event_tx.clone())
    .with_file_observation_tracker(file_observations)
    .with_output_stream(output_tx, call.id.clone());

    if let Some(trace) = provider_call_trace {
        ctx = ctx.with_provider_call_trace(trace.clone());
    }

    let mut delegated_forwarder_handle = None;
    if matches!(call.name.as_str(), "agent") {
        if let Some(parent_tx) = delegated_progress_tx.cloned() {
            let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<AgentProgress>();
            ctx = ctx.with_agent_progress(progress_tx);

            let tool_call_id = call.id.clone();
            let parent_session_id = session_id.to_string();
            let kind = delegated_kind_from_agent_call(&call.arguments);
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

    let result = if extension_intercept_prepared {
        tool_registry
            .execute_prepared(&call.name, call.arguments.clone(), &ctx)
            .await
    } else {
        tool_registry
            .execute(&call.name, call.arguments.clone(), &ctx)
            .await
    }
    .unwrap_or_else(|| {
        ToolResult::error_with_code("unknown_tool", format!("Unknown tool: {}", call.name))
    });

    drop(ctx);
    if let Some(handle) = delegated_forwarder_handle {
        if background_agent {
            // The spawned agent owns a progress sender until it completes. Awaiting the
            // forwarder here would turn an explicitly background launch back into a
            // synchronous tool call. Dropping a Tokio JoinHandle detaches the forwarder,
            // which can continue relaying progress until the background sender closes.
            drop(handle);
        } else {
            let _ = handle.await;
        }
    }
    let _ = forwarder_handle.await;
    result
}

fn agent_runs_in_background(arguments: &serde_json::Value) -> bool {
    arguments
        .get("run_in_background")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn delegated_kind_from_agent_call(arguments: &serde_json::Value) -> DelegatedToolKind {
    match arguments
        .get("agent_type")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
    {
        "plan" => DelegatedToolKind::Plan,
        "verify" => DelegatedToolKind::Verify,
        "build" => DelegatedToolKind::Build,
        _ => DelegatedToolKind::Explore,
    }
}

fn delegated_stage_from_progress(progress: &AgentProgress) -> DelegatedRunStage {
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
    use super::{agent_runs_in_background, delegated_kind_from_agent_call};
    use crate::agent::DelegatedToolKind;
    use serde_json::json;

    #[test]
    fn delegated_kind_tracks_agent_type() {
        assert_eq!(
            delegated_kind_from_agent_call(&json!({"agent_type":"plan"})),
            DelegatedToolKind::Plan
        );
        assert_eq!(
            delegated_kind_from_agent_call(&json!({"agent_type":"verify"})),
            DelegatedToolKind::Verify
        );
        assert_eq!(
            delegated_kind_from_agent_call(&json!({"agent_type":"build"})),
            DelegatedToolKind::Build
        );
        assert_eq!(
            delegated_kind_from_agent_call(&json!({"agent_type":"explore"})),
            DelegatedToolKind::Explore
        );
    }

    #[test]
    fn background_agent_detection_requires_explicit_true() {
        assert!(agent_runs_in_background(
            &json!({"run_in_background": true})
        ));
        assert!(!agent_runs_in_background(
            &json!({"run_in_background": false})
        ));
        assert!(!agent_runs_in_background(&json!({})));
    }
}
