use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::agent::subagent::{AgentProgress, AgentProgressStatus};
use crate::agent::ProviderCallTraceContext;
use crate::agent::{DelegatedProgressEvent, DelegatedRunStage, DelegatedToolKind};
use crate::ai::client::AiClient;
use crate::ai::providers::ReasoningEffort;
use crate::ai::types::AiToolCall;
use crate::process::{CommandEnvironmentPolicy, ProcessRegistry};
use crate::skills::SkillsManager;
use crate::storage::{Database, DelegatedRunRole, DelegatedRunStore, WorkMode, WorkspaceMode};
use crate::tools::registry::{
    agent_call_execution_profile, agent_call_may_start_run, agent_call_starts_run,
    FileObservationTracker, PermissionMode, ShellIsolationPolicy, ToolContext, ToolRegistry,
    ToolResult,
};

use super::super::loop_events::LoopEvent;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Registry dispatch selected after the agent loop has resolved whether
/// optional extensions belong to this run context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegistryExtensionDispatch {
    Standard,
    Prepared,
    Disabled,
}

fn configure_worker_goal_shell_context(ctx: &mut ToolContext) {
    let root = ctx
        .filesystem_access_root()
        .unwrap_or_else(|| ctx.working_dir.clone());
    let root_text = root.display().to_string();
    let runtime_root = root.join(".mitsuro/worker-goal-runtime");

    // Use the shared explicit-only environment primitive with deterministic
    // sandbox-local values. Foreground and tracked background commands consume
    // this same map; no host environment entry is inherited.
    ctx.command_environment.clear();
    for (key, value) in [
        (
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        ),
        ("HOME", root_text.clone()),
        ("USERPROFILE", root_text),
        ("USER", "mitsuro-worker".to_string()),
        ("LOGNAME", "mitsuro-worker".to_string()),
        ("TMPDIR", "/tmp".to_string()),
        ("TMP", "/tmp".to_string()),
        ("TEMP", "/tmp".to_string()),
        (
            "XDG_CACHE_HOME",
            runtime_root.join("cache").display().to_string(),
        ),
        (
            "CARGO_HOME",
            runtime_root.join("cargo").display().to_string(),
        ),
        (
            "RUSTUP_HOME",
            runtime_root.join("rustup").display().to_string(),
        ),
        (
            "npm_config_cache",
            runtime_root.join("npm").display().to_string(),
        ),
        (
            "NPM_CONFIG_CACHE",
            runtime_root.join("npm").display().to_string(),
        ),
    ] {
        ctx.command_environment.insert(key.to_string(), value);
    }
    ctx.command_environment_policy = CommandEnvironmentPolicy::Explicit;
    ctx.shell_isolation_policy = ShellIsolationPolicy::WorkspaceOnly;
}

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
    supervised_approval_granted: bool,
    work_mode: WorkMode,
    delegated_progress_tx: Option<&mpsc::UnboundedSender<DelegatedProgressEvent>>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    provider_call_trace: Option<&ProviderCallTraceContext>,
    subagent_max_turns_override: Option<usize>,
    delegated_reasoning_effort: Option<ReasoningEffort>,
    execution_tool_allowlist: Option<&HashSet<String>>,
    hive_group_run: Option<&crate::storage::HiveGroupRunContext>,
    file_observations: Arc<FileObservationTracker>,
    worker_goal_shell_isolation: bool,
    extension_dispatch: RegistryExtensionDispatch,
    execution_cancellation: Option<CancellationToken>,
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
    .with_supervised_approval(supervised_approval_granted)
    .with_subagent_max_turns(subagent_max_turns_override)
    .with_delegated_reasoning_effort(delegated_reasoning_effort)
    .with_execution_tool_allowlist(execution_tool_allowlist)
    .with_hive_group_run(hive_group_run.cloned())
    .with_ai_client(ai_client.clone())
    .with_skills_manager(Arc::clone(skills_manager))
    .with_tool_registry(Arc::clone(tool_registry))
    .with_loop_event_tx(event_tx.clone())
    .with_file_observation_tracker(file_observations)
    .with_output_stream(output_tx, call.id.clone());

    if worker_goal_shell_isolation {
        configure_worker_goal_shell_context(&mut ctx);
    }

    if let Some(cancellation) = execution_cancellation {
        ctx = ctx.with_execution_cancellation(cancellation);
    }

    if let Some(trace) = provider_call_trace {
        ctx = ctx.with_provider_call_trace(trace.clone());
    }

    let mut delegated_forwarder_handle = None;
    if should_install_delegated_progress_bridge(call) {
        if let Some(parent_tx) = delegated_progress_tx.cloned() {
            let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<AgentProgress>();
            ctx = ctx.with_agent_progress(progress_tx);

            let tool_call_id = call.id.clone();
            let parent_session_id = session_id.to_string();
            let fallback_kind = delegated_kind_from_agent_call(&call.arguments);
            let delegated_store = Database::new(db_path).ok().map(DelegatedRunStore::new);
            delegated_forwarder_handle = Some(tokio::spawn(async move {
                let mut run_kinds = HashMap::<String, DelegatedToolKind>::new();
                while let Some(progress) = progress_rx.recv().await {
                    let delegated_run_id = progress
                        .delegated_run_id
                        .clone()
                        .unwrap_or_else(|| tool_call_id.clone());
                    let kind = *run_kinds
                        .entry(delegated_run_id.clone())
                        .or_insert_with(|| {
                            delegated_kind_from_durable_run(
                                delegated_store.as_ref(),
                                &delegated_run_id,
                                &parent_session_id,
                                &tool_call_id,
                            )
                            .unwrap_or(fallback_kind)
                        });
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

    let result = match extension_dispatch {
        RegistryExtensionDispatch::Standard => {
            tool_registry
                .execute(&call.name, call.arguments.clone(), &ctx)
                .await
        }
        RegistryExtensionDispatch::Prepared => {
            tool_registry
                .execute_prepared(&call.name, call.arguments.clone(), &ctx)
                .await
        }
        RegistryExtensionDispatch::Disabled => {
            tool_registry
                .execute_without_extensions(&call.name, call.arguments.clone(), &ctx)
                .await
        }
    }
    .unwrap_or_else(|| {
        ToolResult::error_with_code("unknown_tool", format!("Unknown tool: {}", call.name))
    });

    drop(ctx);
    if let Some(handle) = delegated_forwarder_handle {
        if successful_background_agent_start(call, &result) {
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

fn should_install_delegated_progress_bridge(call: &AiToolCall) -> bool {
    call.name == "agent" && agent_call_may_start_run(&call.arguments)
}

pub(super) fn successful_background_agent_start(call: &AiToolCall, result: &ToolResult) -> bool {
    if call.name != "agent" || result.is_error {
        return false;
    }

    // Spawn/resume declare their background behavior before execution. A followup
    // does not: it can either steer a live mailbox or turn a terminal record into
    // a new spawn. For followup, only the returned lifecycle envelope can prove
    // that the child retained the sender after the tool returned.
    if agent_call_starts_run(&call.arguments) && agent_runs_in_background(&call.arguments) {
        return true;
    }

    let Ok(output) = serde_json::from_str::<serde_json::Value>(&result.output) else {
        return false;
    };
    output
        .get("data")
        .unwrap_or(&output)
        .get("status")
        .and_then(serde_json::Value::as_str)
        == Some("background_started")
}

fn delegated_kind_from_agent_call(arguments: &serde_json::Value) -> DelegatedToolKind {
    match agent_call_execution_profile(arguments) {
        "plan" => DelegatedToolKind::Plan,
        "verify" => DelegatedToolKind::Verify,
        "build" => DelegatedToolKind::Build,
        _ => DelegatedToolKind::Explore,
    }
}

fn delegated_kind_from_durable_run(
    store: Option<&DelegatedRunStore>,
    delegated_run_id: &str,
    parent_session_id: &str,
    tool_call_id: &str,
) -> Option<DelegatedToolKind> {
    let record = store?.get_run(delegated_run_id).ok()??;
    let owned_by_call = record.parent_session_id == parent_session_id
        && record
            .parent_tool_call_id
            .as_deref()
            .is_none_or(|parent_tool_call_id| parent_tool_call_id == tool_call_id);
    if !owned_by_call {
        return None;
    }
    Some(match record.role {
        DelegatedRunRole::Explore => DelegatedToolKind::Explore,
        DelegatedRunRole::Build => DelegatedToolKind::Build,
        DelegatedRunRole::Planner => DelegatedToolKind::Plan,
        DelegatedRunRole::Verifier => DelegatedToolKind::Verify,
    })
}

fn delegated_stage_from_progress(progress: &AgentProgress) -> DelegatedRunStage {
    match progress.status {
        AgentProgressStatus::Created
        | AgentProgressStatus::Queued
        | AgentProgressStatus::Leased => DelegatedRunStage::Created,
        AgentProgressStatus::Running | AgentProgressStatus::Retrying => DelegatedRunStage::Running,
        AgentProgressStatus::Complete => DelegatedRunStage::Complete,
        AgentProgressStatus::Degraded => DelegatedRunStage::Degraded,
        AgentProgressStatus::Failed => DelegatedRunStage::Failed,
        AgentProgressStatus::Cancelled => DelegatedRunStage::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        agent_runs_in_background, delegated_kind_from_agent_call, delegated_stage_from_progress,
        should_install_delegated_progress_bridge, successful_background_agent_start,
    };
    use crate::agent::subagent::{AgentProgress, AgentProgressStatus};
    use crate::agent::{DelegatedRunStage, DelegatedToolKind};
    use crate::ai::types::AiToolCall;
    use crate::tools::registry::ToolResult;
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

    #[test]
    fn followup_progress_bridge_detaches_only_when_result_started_background_run() {
        let followup = AiToolCall {
            id: "followup-call".to_string(),
            name: "agent".to_string(),
            arguments: json!({
                "action": "followup",
                "delegated_run_id": "terminal-or-live",
                "message": "continue",
                "run_in_background": true
            }),
        };
        assert!(should_install_delegated_progress_bridge(&followup));

        let live_result = ToolResult::success_data(json!({
            "status": "queued",
            "delivery": "accepted_by_live_mailbox",
            "delegated_run_id": "terminal-or-live"
        }));
        assert!(!successful_background_agent_start(&followup, &live_result));

        let resumed_result = ToolResult::success_data(json!({
            "status": "background_started",
            "delegated_run_id": "new-run"
        }));
        assert!(successful_background_agent_start(
            &followup,
            &resumed_result
        ));
    }

    #[test]
    fn explicit_background_spawn_keeps_existing_detach_behavior() {
        let spawn = AiToolCall {
            id: "spawn-call".to_string(),
            name: "agent".to_string(),
            arguments: json!({
                "action": "spawn",
                "run_in_background": true,
                "name": "reader",
                "instructions": "inspect"
            }),
        };
        assert!(should_install_delegated_progress_bridge(&spawn));
        assert!(successful_background_agent_start(
            &spawn,
            &ToolResult::success("legacy background response")
        ));
    }

    #[test]
    fn progress_stage_uses_canonical_status_not_action_prose() {
        let progress = |status, action: Option<&str>| AgentProgress {
            status,
            current_action: action.map(ToString::to_string),
            ..AgentProgress::default()
        };

        assert_eq!(
            delegated_stage_from_progress(&progress(
                AgentProgressStatus::Degraded,
                Some("ordinary summary")
            )),
            DelegatedRunStage::Degraded
        );
        assert_eq!(
            delegated_stage_from_progress(&progress(
                AgentProgressStatus::Cancelled,
                Some("ordinary summary")
            )),
            DelegatedRunStage::Cancelled
        );
        assert_eq!(
            delegated_stage_from_progress(&progress(
                AgentProgressStatus::Failed,
                Some("cancelled")
            )),
            DelegatedRunStage::Failed
        );
    }
}
