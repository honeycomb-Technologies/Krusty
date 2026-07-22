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

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use futures::future::join_all;
use tokio::sync::mpsc;

use crate::ai::client::AiClient;
use crate::ai::types::{AiToolCall, Content};
use crate::process::ProcessRegistry;
use crate::skills::SkillsManager;
use crate::storage::{
    Database, PartialAssistantState, PendingInteractionSnapshot, RecoveryDecision,
    RecoveryNonResumableReason, RecoveryStatus, SessionManager, SessionRecoveryState, WorkMode,
    WorkspaceMode,
};
use crate::tools::registry::{
    tool_policy_for_call, FileObservationTracker, PermissionMode, ToolCategory, ToolContext,
    ToolRegistry, ToolResult,
};

#[cfg(test)]
use super::loop_events::LoopInput;
use super::loop_events::{LoopEvent, LoopInputInbox};
use super::plan_handler;
use super::tool_control::{AuthorizationDecision, RetryDirective, ToolControl};
use super::ProviderCallTraceContext;

use self::plan_updates::emit_plan_update;
use self::regular::execute_regular_tool;
use self::user_message::execute_send_user_message;

pub(crate) struct ToolExecutionBatch {
    pub(crate) results: Vec<Content>,
    pub(crate) next_work_mode: WorkMode,
    pub(crate) cancelled: bool,
}

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
    skills_manager: &Arc<tokio::sync::RwLock<SkillsManager>>,
    session_id: &str,
    db_path: &Path,
    user_id: Option<&str>,
    permission_mode: PermissionMode,
    current_mode: WorkMode,
    recovery_partial_assistant: Option<&PartialAssistantState>,
    delegated_progress_tx: Option<&mpsc::UnboundedSender<crate::agent::DelegatedProgressEvent>>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    provider_call_trace: Option<&ProviderCallTraceContext>,
    input_inbox: &mut LoopInputInbox,
    subagent_max_turns_override: Option<usize>,
    advertised_tool_names: &HashSet<String>,
    disabled_tools: Option<&[String]>,
    file_observations: Arc<FileObservationTracker>,
) -> ToolExecutionBatch {
    let mut work_mode = current_mode;
    let mut results = Vec::new();
    let tool_control = ToolControl::new(permission_mode);
    let mut parallel_calls_to_skip = 0usize;
    let configured_extension_manager = tool_registry.agent_extension_manager();
    let extension_snapshot_prepared = configured_extension_manager.is_some();
    let extension_manager =
        configured_extension_manager.filter(|manager| manager.has_tool_interceptors());

    for (call_index, original_call) in tool_calls.iter().enumerate() {
        if parallel_calls_to_skip > 0 {
            parallel_calls_to_skip -= 1;
            continue;
        }

        // Treat the frozen provider request as an execution capability. A
        // provider may hallucinate a registered tool that was not included in
        // this run's schema; reject it before extensions, approvals, recovery
        // interaction persistence, or registry execution can observe it.
        if !advertised_tool_names.contains(&original_call.name) {
            let denied = ToolResult::error_with_code(
                "tool_not_advertised",
                format!(
                    "Tool '{}' was not advertised for this agent run",
                    original_call.name
                ),
            );
            results.push(tool_control.publish_result(original_call, &denied, event_tx));
            continue;
        }

        // Extension rewrites are part of call preparation, not execution. The
        // effective call below is what policy classifies, recovery persists,
        // and the approval UI displays. ToolRegistry is told not to intercept
        // it again after approval.
        let intercepted_call;
        let call = if let Some(manager) = &extension_manager {
            let intercept_context = ToolContext {
                working_dir: working_dir.to_path_buf(),
                project_dir: project_dir.map(Path::to_path_buf),
                workspace_mode: if project_dir.is_some() {
                    WorkspaceMode::Selected
                } else {
                    WorkspaceMode::Neutral
                },
                session_id: Some(session_id.to_string()),
                db_path: Some(db_path.to_path_buf()),
                plan_mode: work_mode == WorkMode::Plan,
                current_model: Some(ai_client.config().model.clone()),
                current_model_key: Some(ai_client.resolved_model().key.clone()),
                ai_client: Some(ai_client.clone()),
                permission_mode,
                ..Default::default()
            };
            let intercept = manager
                .before_tool(
                    &original_call.name,
                    original_call.arguments.clone(),
                    &intercept_context,
                )
                .await;
            intercepted_call = AiToolCall {
                id: original_call.id.clone(),
                name: original_call.name.clone(),
                arguments: intercept.params,
            };
            if let Some(reason) = intercept.block_reason {
                let blocked = ToolResult::error_with_code("blocked_by_extension", reason);
                results.push(tool_control.publish_result(&intercepted_call, &blocked, event_tx));
                continue;
            }
            &intercepted_call
        } else {
            original_call
        };

        if extension_manager.is_none()
            && is_parallel_safe_call(call, advertised_tool_names, disabled_tools, &tool_control)
        {
            let parallel_calls = tool_calls[call_index..]
                .iter()
                .take_while(|candidate| {
                    is_parallel_safe_call(
                        candidate,
                        advertised_tool_names,
                        disabled_tools,
                        &tool_control,
                    )
                })
                .collect::<Vec<_>>();

            if parallel_calls.len() > 1 {
                for parallel_call in &parallel_calls {
                    let _ = event_tx.send(LoopEvent::ToolExecuting {
                        id: parallel_call.id.clone(),
                        name: parallel_call.name.clone(),
                    });
                }

                let executions = parallel_calls.iter().map(|parallel_call| async {
                    let mut retries_attempted = 0usize;
                    loop {
                        let result = execute_regular_tool(
                            parallel_call,
                            tool_registry,
                            ai_client,
                            working_dir,
                            project_dir,
                            process_registry,
                            skills_manager,
                            session_id,
                            db_path,
                            user_id,
                            permission_mode,
                            work_mode,
                            delegated_progress_tx,
                            event_tx,
                            provider_call_trace,
                            subagent_max_turns_override,
                            Arc::clone(&file_observations),
                            extension_snapshot_prepared,
                        )
                        .await;

                        match tool_control.retry_directive(
                            parallel_call,
                            &result,
                            retries_attempted,
                        ) {
                            RetryDirective::Stop => break result,
                            RetryDirective::RetryOnce { reason } => {
                                retries_attempted += 1;
                                tracing::warn!(
                                    tool = %parallel_call.name,
                                    tool_call_id = %parallel_call.id,
                                    retries_attempted,
                                    reason,
                                    "Retrying parallel tool execution under centralized policy"
                                );
                            }
                        }
                    }
                });
                let execution = join_all(executions);
                tokio::pin!(execution);

                let mut input_closed = false;
                let batch_results = loop {
                    tokio::select! {
                        batch_results = &mut execution => break Some(batch_results),
                        cancelled = input_inbox.recv_cancel(), if !input_closed => {
                            match cancelled {
                                Some(()) => break None,
                                None => input_closed = true,
                            }
                        }
                    }
                };

                let Some(batch_results) = batch_results else {
                    for parallel_call in &parallel_calls {
                        let cancelled = ToolResult::error_with_code(
                            "cancelled",
                            "Tool execution cancelled by user",
                        );
                        results.push(tool_control.publish_result(
                            parallel_call,
                            &cancelled,
                            event_tx,
                        ));
                    }
                    return ToolExecutionBatch {
                        results,
                        next_work_mode: work_mode,
                        cancelled: true,
                    };
                };

                for (parallel_call, result) in parallel_calls.iter().zip(batch_results.iter()) {
                    results.push(tool_control.publish_result(parallel_call, result, event_tx));
                }
                parallel_calls_to_skip = parallel_calls.len() - 1;
                continue;
            }
        }

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

        match tool_control.authorize(call, event_tx, input_inbox).await {
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
            AuthorizationDecision::Cancel => {
                let cancelled =
                    ToolResult::error_with_code("cancelled", "Tool execution cancelled by user");
                results.push(tool_control.publish_result(call, &cancelled, event_tx));
                return ToolExecutionBatch {
                    results,
                    next_work_mode: work_mode,
                    cancelled: true,
                };
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
            let execution = execute_regular_tool(
                call,
                tool_registry,
                ai_client,
                working_dir,
                project_dir,
                process_registry,
                skills_manager,
                session_id,
                db_path,
                user_id,
                permission_mode,
                work_mode,
                delegated_progress_tx,
                event_tx,
                provider_call_trace,
                subagent_max_turns_override,
                Arc::clone(&file_observations),
                extension_snapshot_prepared,
            );
            tokio::pin!(execution);

            let mut input_closed = false;
            let result = loop {
                tokio::select! {
                    result = &mut execution => break Some(result),
                    cancelled = input_inbox.recv_cancel(), if !input_closed => {
                        match cancelled {
                            Some(()) => break None,
                            None => input_closed = true,
                        }
                    }
                }
            };

            let Some(result) = result else {
                let cancelled =
                    ToolResult::error_with_code("cancelled", "Tool execution cancelled by user");
                results.push(tool_control.publish_result(call, &cancelled, event_tx));
                return ToolExecutionBatch {
                    results,
                    next_work_mode: work_mode,
                    cancelled: true,
                };
            };

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

    ToolExecutionBatch {
        results,
        next_work_mode: work_mode,
        cancelled: false,
    }
}

/// Only calls with a read-only runtime policy can share an execution batch.
/// Mutations, interactive operations, and delegated agents stay serialized so
/// approval ordering and same-path writes remain deterministic.
fn is_parallel_safe_call(
    call: &AiToolCall,
    advertised_tool_names: &HashSet<String>,
    disabled_tools: Option<&[String]>,
    tool_control: &ToolControl,
) -> bool {
    if !advertised_tool_names.contains(&call.name)
        || disabled_tools.is_some_and(|disabled| {
            disabled
                .iter()
                .any(|name| name.as_str() == call.name.as_str())
        })
        || tool_control.requires_approval(call)
    {
        return false;
    }

    tool_policy_for_call(&call.name, &call.arguments).category == ToolCategory::ReadOnly
        && call.name != "agent"
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
    use crate::tools::registry::{Tool, ToolContext};
    use async_trait::async_trait;
    use serde_json::json;
    use serde_json::Value;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;
    use tempfile::TempDir;

    struct ConcurrentReadTool {
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
    }

    struct DelayedDelegatedWriteTool {
        started_tx: mpsc::UnboundedSender<String>,
    }

    struct CapturingAgentTool {
        calls: Arc<StdMutex<Vec<Value>>>,
    }

    #[async_trait]
    impl Tool for CapturingAgentTool {
        fn name(&self) -> &str {
            "agent"
        }

        fn description(&self) -> &str {
            "capture effective delegated-agent arguments"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }

        async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(params);
            ToolResult::success("captured")
        }
    }

    #[async_trait]
    impl Tool for DelayedDelegatedWriteTool {
        fn name(&self) -> &str {
            "agent"
        }

        fn description(&self) -> &str {
            "test foreground delegated writer"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }

        async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
            let output_path = std::path::PathBuf::from(
                params["output_path"]
                    .as_str()
                    .expect("test output path should be a string"),
            );
            let delay_ms = params["delay_ms"]
                .as_u64()
                .expect("test delay should be an integer");
            let mut children = tokio::task::JoinSet::new();
            children.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                tokio::fs::write(output_path, "delegated mutation")
                    .await
                    .expect("test delegated write should succeed");
            });
            self.started_tx
                .send(
                    ctx.session_id
                        .clone()
                        .expect("test tool should receive a session id"),
                )
                .expect("test should still receive child-start notifications");

            while let Some(result) = children.join_next().await {
                result.expect("test delegated child should succeed");
            }
            ToolResult::success("delegated work complete")
        }
    }

    #[async_trait]
    impl Tool for ConcurrentReadTool {
        fn name(&self) -> &str {
            "read"
        }

        fn description(&self) -> &str {
            "test read"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }

        async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            ToolResult::success("read complete")
        }
    }

    #[tokio::test]
    async fn independent_read_only_calls_execute_concurrently_and_preserve_result_order() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let registry = Arc::new(ToolRegistry::new());
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        registry
            .register(Arc::new(ConcurrentReadTool {
                active,
                max_active: Arc::clone(&max_active),
            }))
            .await;
        let ai_client = Arc::new(AiClient::new(Default::default(), String::new()));
        let process_registry = Arc::new(ProcessRegistry::new());
        let skills_manager = Arc::new(tokio::sync::RwLock::new(SkillsManager::new(
            temp_dir.path().join("skills"),
            None,
        )));
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (_input_tx, input_rx) = mpsc::unbounded_channel();
        let mut input_inbox = LoopInputInbox::new(input_rx);
        let calls = vec![
            AiToolCall {
                id: "read-1".into(),
                name: "read".into(),
                arguments: json!({"file_path": "one"}),
            },
            AiToolCall {
                id: "read-2".into(),
                name: "read".into(),
                arguments: json!({"file_path": "two"}),
            },
        ];
        let advertised = HashSet::from(["read".to_string()]);

        let batch = execute_tools(
            &calls,
            &registry,
            &ai_client,
            temp_dir.path(),
            Some(temp_dir.path()),
            &process_registry,
            &skills_manager,
            "session",
            &temp_dir.path().join("db"),
            None,
            PermissionMode::Autonomous,
            WorkMode::Build,
            None,
            None,
            &event_tx,
            None,
            &mut input_inbox,
            None,
            &advertised,
            None,
            Arc::new(FileObservationTracker::new()),
        )
        .await;

        assert!(!batch.cancelled);
        assert_eq!(batch.results.len(), 2);
        assert_eq!(max_active.load(Ordering::SeqCst), 2);
        assert!(matches!(
            &batch.results[0],
            Content::ToolResult { tool_use_id, .. } if tool_use_id == "read-1"
        ));
        assert!(matches!(
            &batch.results[1],
            Content::ToolResult { tool_use_id, .. } if tool_use_id == "read-2"
        ));
    }

    #[tokio::test]
    async fn unadvertised_supervised_write_is_rejected_without_approval_or_execution() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let registry = Arc::new(ToolRegistry::new());
        let ai_client = Arc::new(AiClient::new(Default::default(), String::new()));
        let process_registry = Arc::new(ProcessRegistry::new());
        let skills_manager = Arc::new(tokio::sync::RwLock::new(SkillsManager::new(
            temp_dir.path().join("skills"),
            None,
        )));
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (_input_tx, input_rx) = mpsc::unbounded_channel();
        let mut input_inbox = LoopInputInbox::new(input_rx);
        let advertised = HashSet::from(["read".to_string()]);
        let call = AiToolCall {
            id: "hallucinated-bash".into(),
            name: "bash".into(),
            arguments: json!({"command": "touch should-not-exist"}),
        };

        let batch = execute_tools(
            &[call],
            &registry,
            &ai_client,
            temp_dir.path(),
            Some(temp_dir.path()),
            &process_registry,
            &skills_manager,
            "session",
            &temp_dir.path().join("db"),
            None,
            PermissionMode::Supervised,
            WorkMode::Build,
            None,
            None,
            &event_tx,
            None,
            &mut input_inbox,
            None,
            &advertised,
            None,
            Arc::new(FileObservationTracker::new()),
        )
        .await;

        assert!(!batch.cancelled);
        assert_eq!(batch.results.len(), 1);
        match &batch.results[0] {
            Content::ToolResult {
                tool_use_id,
                output,
                is_error,
            } => {
                assert_eq!(tool_use_id, "hallucinated-bash");
                assert_eq!(output["error_code"], "tool_not_advertised");
                assert_eq!(*is_error, Some(true));
            }
            other => panic!("unexpected result: {other:?}"),
        }

        let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().all(|event| !matches!(
            event,
            LoopEvent::ToolApprovalRequired { .. } | LoopEvent::ToolExecuting { .. }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            LoopEvent::ToolResult { id, is_error: true, .. } if id == "hallucinated-bash"
        )));
        assert!(!temp_dir.path().join("should-not-exist").exists());
    }

    #[tokio::test]
    async fn extension_rewrite_is_authorized_then_executed_once_without_reinterception() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let registry = Arc::new(ToolRegistry::new());
        let captured_calls = Arc::new(StdMutex::new(Vec::new()));
        registry
            .register(Arc::new(CapturingAgentTool {
                calls: Arc::clone(&captured_calls),
            }))
            .await;

        let intercept_count = Arc::new(AtomicUsize::new(0));
        let manager = crate::extensions::AgentExtensionManager::new_with_paths(
            temp_dir.path(),
            temp_dir.path().join("extension-runtime"),
            temp_dir.path().join("global-extensions"),
        );
        registry.set_agent_extension_manager(manager.clone());
        let bun_available = which::which("bun").is_ok();
        if bun_available {
            let extension_dir = temp_dir
                .path()
                .join(".krusty")
                .join("extensions")
                .join("approval-rewrite");
            fs::create_dir_all(&extension_dir).expect("extension directory should be created");
            fs::write(
                extension_dir.join("krusty-extension.json"),
                r#"{"id":"approval-rewrite","name":"Approval Rewrite","entry":"index.ts"}"#,
            )
            .expect("extension manifest should be written");
            fs::write(
                extension_dir.join("index.ts"),
                r#"
export default (krusty) => {
  let rewriteCount = 0;
  krusty.on("tool.execute.before", (input, output) => {
    if (input.tool !== "agent") return;
    rewriteCount += 1;
    output.args.agent_type = "build";
    output.args.rewrite_count = rewriteCount;
  });
};
"#,
            )
            .expect("extension entry should be written");
            manager
                .set_project_trusted(true)
                .await
                .expect("test project should be explicitly trusted");
            manager
                .refresh_and_register(&registry)
                .await
                .expect("Bun extension should load");
        } else {
            manager.set_test_tool_interceptor({
                let intercept_count = Arc::clone(&intercept_count);
                move |name, mut params| {
                    assert_eq!(name, "agent");
                    let invocation = intercept_count.fetch_add(1, Ordering::SeqCst) + 1;
                    params["agent_type"] = Value::String("build".to_string());
                    params["rewrite_count"] = json!(invocation);
                    crate::extensions::AgentExtensionToolIntercept {
                        params,
                        block_reason: None,
                    }
                }
            });
        }
        assert!(manager.has_tool_interceptors());

        let original_call = AiToolCall {
            id: "rewritten-agent".to_string(),
            name: "agent".to_string(),
            arguments: json!({"agent_type": "explore", "prompt": "inspect only"}),
        };
        assert!(!ToolControl::new(PermissionMode::Supervised).requires_approval(&original_call));

        let ai_client = Arc::new(AiClient::new(Default::default(), String::new()));
        let process_registry = Arc::new(ProcessRegistry::new());
        let skills_manager = Arc::new(tokio::sync::RwLock::new(SkillsManager::new(
            temp_dir.path().join("skills"),
            None,
        )));
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        input_tx
            .send(LoopInput::ToolApproval {
                tool_call_id: original_call.id.clone(),
                approved: true,
            })
            .expect("approval input should be queued");
        let mut input_inbox = LoopInputInbox::new(input_rx);
        let advertised = HashSet::from(["agent".to_string()]);

        let batch = execute_tools(
            &[original_call],
            &registry,
            &ai_client,
            temp_dir.path(),
            Some(temp_dir.path()),
            &process_registry,
            &skills_manager,
            "session",
            &temp_dir.path().join("db"),
            None,
            PermissionMode::Supervised,
            WorkMode::Build,
            None,
            None,
            &event_tx,
            None,
            &mut input_inbox,
            None,
            &advertised,
            None,
            Arc::new(FileObservationTracker::new()),
        )
        .await;

        assert!(!batch.cancelled);
        assert_eq!(batch.results.len(), 1);
        if !bun_available {
            assert_eq!(intercept_count.load(Ordering::SeqCst), 1);
        }
        assert_eq!(
            captured_calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[json!({
                "agent_type": "build",
                "prompt": "inspect only",
                "rewrite_count": 1
            })]
        );

        let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            LoopEvent::ToolApprovalRequired { id, name, arguments }
                if id == "rewritten-agent"
                    && name == "agent"
                    && arguments["agent_type"] == "build"
                    && arguments["rewrite_count"] == 1
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            LoopEvent::ToolApproved { id } if id == "rewritten-agent"
        )));
    }

    async fn run_delayed_delegated_write(
        registry: Arc<ToolRegistry>,
        working_dir: std::path::PathBuf,
        session_id: String,
        output_path: std::path::PathBuf,
        delay_ms: u64,
        input_rx: mpsc::UnboundedReceiver<LoopInput>,
    ) -> ToolExecutionBatch {
        let ai_client = Arc::new(AiClient::new(Default::default(), String::new()));
        let process_registry = Arc::new(ProcessRegistry::new());
        let skills_manager = Arc::new(tokio::sync::RwLock::new(SkillsManager::new(
            working_dir.join("skills"),
            None,
        )));
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let call = AiToolCall {
            id: format!("agent-{session_id}"),
            name: "agent".to_string(),
            arguments: json!({
                "agent_type": "build",
                "output_path": output_path,
                "delay_ms": delay_ms,
            }),
        };

        let mut input_inbox = LoopInputInbox::new(input_rx);
        let advertised = HashSet::from(["agent".to_string()]);
        execute_tools(
            &[call],
            &registry,
            &ai_client,
            &working_dir,
            Some(&working_dir),
            &process_registry,
            &skills_manager,
            &session_id,
            &working_dir.join(format!("{session_id}.db")),
            None,
            PermissionMode::Autonomous,
            WorkMode::Build,
            None,
            None,
            &event_tx,
            None,
            &mut input_inbox,
            None,
            &advertised,
            None,
            Arc::new(FileObservationTracker::new()),
        )
        .await
    }

    #[tokio::test]
    async fn loop_cancel_drops_foreground_children_without_cancelling_another_session() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let registry = Arc::new(ToolRegistry::new());
        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        registry
            .register(Arc::new(DelayedDelegatedWriteTool { started_tx }))
            .await;
        let cancelled_output = temp_dir.path().join("cancelled.txt");
        let unaffected_output = temp_dir.path().join("unaffected.txt");
        let (cancel_tx, cancel_rx) = mpsc::unbounded_channel();
        let (_other_tx, other_rx) = mpsc::unbounded_channel();

        let cancelled_run = tokio::spawn(run_delayed_delegated_write(
            Arc::clone(&registry),
            temp_dir.path().to_path_buf(),
            "cancelled-session".to_string(),
            cancelled_output.clone(),
            150,
            cancel_rx,
        ));
        let unaffected_run = tokio::spawn(run_delayed_delegated_write(
            registry,
            temp_dir.path().to_path_buf(),
            "other-session".to_string(),
            unaffected_output.clone(),
            80,
            other_rx,
        ));

        let mut started_sessions = std::collections::HashSet::new();
        while started_sessions.len() < 2 {
            let session_id =
                tokio::time::timeout(std::time::Duration::from_secs(1), started_rx.recv())
                    .await
                    .expect("both delegated children should start")
                    .expect("child-start channel should stay open");
            started_sessions.insert(session_id);
        }
        assert!(started_sessions.contains("cancelled-session"));
        assert!(started_sessions.contains("other-session"));

        cancel_tx
            .send(LoopInput::Cancel)
            .expect("cancelled session should accept LoopInput::Cancel");

        let cancelled_batch =
            tokio::time::timeout(std::time::Duration::from_secs(1), cancelled_run)
                .await
                .expect("cancelled run should stop promptly")
                .expect("cancelled run task should join");
        let unaffected_batch =
            tokio::time::timeout(std::time::Duration::from_secs(1), unaffected_run)
                .await
                .expect("other session should finish")
                .expect("other session task should join");
        tokio::time::sleep(std::time::Duration::from_millis(170)).await;

        assert!(cancelled_batch.cancelled);
        assert!(!unaffected_batch.cancelled);
        assert!(!cancelled_output.exists());
        assert_eq!(
            std::fs::read_to_string(unaffected_output).expect("other session output should exist"),
            "delegated mutation"
        );
    }

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
