use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::{
    Client as AcpClient, ContentBlock as AcpContent, ContentChunk, CurrentModeUpdate,
    PermissionOption, PermissionOptionId, PermissionOptionKind, Plan, PlanEntry, PlanEntryPriority,
    PlanEntryStatus, RequestPermissionOutcome, RequestPermissionRequest, SessionNotification,
    SessionUpdate, StopReason, TextContent, ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields,
};

use crate::agent::loop_events::{LoopEvent, LoopStopReason};
use crate::agent::{LoopInput, OrchestratorServices, RunBudget, RunProvenance, RunSpecBuilder};
use crate::ai::client::CallOptions;
use crate::ai::types::{Content, Role};
use crate::skills::SkillsManager;
use crate::storage::SessionType;

use super::content::convert_acp_content;
use super::PromptProcessor;
use crate::acp::error::AcpError;
use crate::acp::session::SessionState;
use crate::acp::tools::{
    create_tool_call_complete, create_tool_call_failed, create_tool_call_start,
    text_to_tool_content, tool_name_to_kind,
};

impl PromptProcessor {
    /// Process an ACP prompt through Krusty's canonical agentic orchestrator.
    pub async fn process_prompt<C: AcpClient>(
        &self,
        session: &SessionState,
        prompt: Vec<AcpContent>,
        connection: &C,
    ) -> Result<StopReason, AcpError> {
        let ai_client = session
            .ai_client()
            .await
            .or_else(|| self.default_ai_client())
            .ok_or_else(|| {
                AcpError::NotAuthenticated(
                    "AI client not initialized - select a model first".to_string(),
                )
            })?;

        let initial_content: Vec<Content> =
            prompt.into_iter().filter_map(convert_acp_content).collect();
        if initial_content.is_empty() {
            return Err(AcpError::InvalidRequest(
                "prompt contained no supported content".to_string(),
            ));
        }
        session
            .add_user_message_content(initial_content.clone())
            .await;

        let mut conversation = session.history().await;
        if let Some(recovery_notice) = session.take_recovery_notice().await {
            let insert_at = conversation
                .iter()
                .rposition(|message| message.role == Role::User)
                .unwrap_or(conversation.len());
            conversation.insert(insert_at, recovery_notice);
        }

        let workspace_root = canonical_acp_workspace_root(session)?;
        let tool_defs = self.tools.get_ai_tools().await;
        let options = CallOptions {
            tools: (!tool_defs.is_empty()).then_some(tool_defs),
            enable_caching: true,
            session_id: Some(session.id.to_string()),
            codex_parallel_tool_calls: true,
            ..Default::default()
        };
        let mode_aware_code_tools = options.tools.is_some();
        let services = OrchestratorServices {
            ai_client,
            tool_registry: Arc::clone(&self.tools),
            process_registry: Arc::clone(&self.process_registry),
            db_path: self.db_path.clone(),
            skills_manager: Arc::new(tokio::sync::RwLock::new(SkillsManager::with_defaults(
                &workspace_root,
            ))),
        };
        let run_spec = RunSpecBuilder::new(
            RunProvenance::Acp,
            session.id.to_string(),
            workspace_root.clone(),
            SessionType::Code,
        )
        .project_dir(Some(workspace_root))
        .permission_mode(session.permission_mode().await)
        .run_budget(
            self.agent_config
                .acp_max_turns()
                .map(RunBudget::with_max_turns),
        )
        .stream_idle_timeout(self.agent_config.stream_idle_timeout())
        .initial_work_mode(session.work_mode().await)
        .mode_aware_code_tools(mode_aware_code_tools)
        .generate_title(false)
        .call_options(options)
        .build(services.ai_client.as_ref())
        .map_err(|error| AcpError::InvalidRequest(error.to_string()))?;

        let (mut event_rx, input_tx) = run_spec.start(services, conversation);
        session.set_active_input(input_tx.clone());

        let result = self
            .bridge_loop_events(session, connection, &mut event_rx, &input_tx)
            .await;
        if result.is_err() {
            // If the ACP connection disappears or rejects an update, do not leave
            // the canonical loop working invisibly in the background.
            let _ = input_tx.send(LoopInput::Cancel);
        }
        session.clear_active_input();

        if let Some(storage_session_id) = session.get_storage_session_id().await {
            if let Err(error) = session.load_from_storage(&storage_session_id).await {
                tracing::warn!(
                    session_id = %session.id,
                    "Failed to refresh ACP history after canonical run: {}",
                    error
                );
            }
        }

        result
    }

    async fn bridge_loop_events<C: AcpClient>(
        &self,
        session: &SessionState,
        connection: &C,
        event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<LoopEvent>,
        input_tx: &tokio::sync::mpsc::UnboundedSender<LoopInput>,
    ) -> Result<StopReason, AcpError> {
        let mut tool_output = HashMap::<String, String>::new();
        let mut last_error = None::<String>;

        while let Some(event) = event_rx.recv().await {
            match event {
                LoopEvent::TextDelta { delta }
                | LoopEvent::TextDeltaWithCitations { delta, .. } => {
                    send_text_update(session, connection, false, delta).await?;
                }
                LoopEvent::ThinkingDelta { thinking } => {
                    send_text_update(session, connection, true, thinking).await?;
                }
                LoopEvent::ToolCallStart { id, name } => {
                    let tool_call =
                        ToolCall::new(ToolCallId::from(id), format!("Running {}", name))
                            .kind(tool_name_to_kind(&name));
                    send_update(session, connection, SessionUpdate::ToolCall(tool_call)).await?;
                }
                LoopEvent::ToolCallComplete {
                    id,
                    name,
                    arguments,
                } => {
                    let update = create_tool_call_start(&id, &name, arguments);
                    send_update(session, connection, SessionUpdate::ToolCallUpdate(update)).await?;
                }
                LoopEvent::ToolExecuting { id, .. } => {
                    let fields = ToolCallUpdateFields::new().status(ToolCallStatus::InProgress);
                    send_update(
                        session,
                        connection,
                        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                            ToolCallId::from(id),
                            fields,
                        )),
                    )
                    .await?;
                }
                LoopEvent::ToolOutputDelta { id, delta } => {
                    let output = tool_output.entry(id.clone()).or_default();
                    output.push_str(&delta);
                    let fields =
                        ToolCallUpdateFields::new().content(vec![text_to_tool_content(output)]);
                    send_update(
                        session,
                        connection,
                        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                            ToolCallId::from(id),
                            fields,
                        )),
                    )
                    .await?;
                }
                LoopEvent::ToolResult {
                    id,
                    output,
                    is_error,
                } => {
                    tool_output.remove(&id);
                    let update = if is_error {
                        create_tool_call_failed(&id, &output)
                    } else {
                        create_tool_call_complete(&id, vec![text_to_tool_content(&output)])
                    };
                    send_update(session, connection, SessionUpdate::ToolCallUpdate(update)).await?;
                }
                LoopEvent::ToolApprovalRequired {
                    id,
                    name,
                    arguments,
                } => {
                    let decision =
                        request_tool_permission(session, connection, &id, &name, arguments).await?;
                    let input = match decision {
                        ToolPermissionDecision::Approved => LoopInput::ToolApproval {
                            tool_call_id: id,
                            approved: true,
                        },
                        ToolPermissionDecision::Denied => LoopInput::ToolApproval {
                            tool_call_id: id,
                            approved: false,
                        },
                        ToolPermissionDecision::Cancelled => LoopInput::Cancel,
                    };
                    input_tx.send(input).map_err(|_| {
                        AcpError::InternalError(
                            "canonical agent loop closed during approval".to_string(),
                        )
                    })?;
                }
                LoopEvent::ToolApproved { id } => {
                    let fields = ToolCallUpdateFields::new().status(ToolCallStatus::InProgress);
                    send_update(
                        session,
                        connection,
                        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                            ToolCallId::from(id),
                            fields,
                        )),
                    )
                    .await?;
                }
                LoopEvent::ToolDenied { id } => {
                    let update = create_tool_call_failed(&id, "Tool execution denied by user");
                    send_update(session, connection, SessionUpdate::ToolCallUpdate(update)).await?;
                }
                LoopEvent::PlanUpdate { tasks } => {
                    let entries = tasks
                        .into_iter()
                        .map(|task| {
                            PlanEntry::new(
                                task.description,
                                PlanEntryPriority::Medium,
                                if task.completed {
                                    PlanEntryStatus::Completed
                                } else {
                                    PlanEntryStatus::Pending
                                },
                            )
                        })
                        .collect();
                    send_update(session, connection, SessionUpdate::Plan(Plan::new(entries)))
                        .await?;
                }
                LoopEvent::WorkflowUpdated { .. } => {
                    // ACP receives the projected PlanUpdate event alongside
                    // this revision signal; native clients refresh via API.
                }
                LoopEvent::ModeChange { mode, .. } => {
                    let acp_mode = canonical_mode_to_acp(&mode);
                    session.set_mode(Some(acp_mode.to_string())).await;
                    send_update(
                        session,
                        connection,
                        SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(acp_mode)),
                    )
                    .await?;
                }
                LoopEvent::ServerToolStart { id, name } => {
                    let call = ToolCall::new(ToolCallId::from(id), format!("Running {}", name))
                        .kind(tool_name_to_kind(&name));
                    send_update(session, connection, SessionUpdate::ToolCall(call)).await?;
                }
                LoopEvent::ServerToolComplete { id, .. } => {
                    let update = create_tool_call_complete(&id, Vec::new());
                    send_update(session, connection, SessionUpdate::ToolCallUpdate(update)).await?;
                }
                LoopEvent::ServerToolError {
                    tool_use_id,
                    error_code,
                } => {
                    let update = create_tool_call_failed(&tool_use_id, &error_code);
                    send_update(session, connection, SessionUpdate::ToolCallUpdate(update)).await?;
                }
                LoopEvent::UserMessage { message, .. } => {
                    send_text_update(session, connection, false, message).await?;
                }
                LoopEvent::Error { error } => {
                    last_error = Some(error);
                }
                LoopEvent::Finished { stop_reason, .. } => {
                    return convert_loop_stop_reason(stop_reason, last_error);
                }
                LoopEvent::AwaitingInput { .. }
                | LoopEvent::SteeringInjected { .. }
                | LoopEvent::PlanComplete { .. }
                | LoopEvent::Usage { .. }
                | LoopEvent::TurnComplete { .. }
                | LoopEvent::RunBudgetResolved { .. }
                | LoopEvent::ProviderRequestPrepared { .. }
                | LoopEvent::MicrocompactionApplied { .. }
                | LoopEvent::ProgressGuard { .. }
                | LoopEvent::TickInjected { .. }
                | LoopEvent::AgentSleeping { .. }
                | LoopEvent::SessionPinched { .. }
                | LoopEvent::ContextCompactionStarted { .. }
                | LoopEvent::ContextCompacted { .. }
                | LoopEvent::ThinkingComplete { .. }
                | LoopEvent::TitleGenerated { .. }
                | LoopEvent::WebSearchResults { .. }
                | LoopEvent::WebFetchResult { .. }
                | LoopEvent::AgentBackgroundStarted { .. }
                | LoopEvent::AgentBackgroundCompleted { .. }
                | LoopEvent::ClassifierDecision { .. }
                | LoopEvent::TeammateSpawned { .. }
                | LoopEvent::TeammateTaskCompleted { .. }
                | LoopEvent::TeammateTaskFailed { .. }
                | LoopEvent::TeammateCancelled { .. } => {}
            }
        }

        Err(AcpError::InternalError(
            "canonical agent loop ended without a terminal event".to_string(),
        ))
    }
}

async fn send_update<C: AcpClient>(
    session: &SessionState,
    connection: &C,
    update: SessionUpdate,
) -> Result<(), AcpError> {
    connection
        .session_notification(SessionNotification::new(session.id.clone(), update))
        .await
        .map_err(|error| AcpError::ProtocolError(error.to_string()))
}

async fn send_text_update<C: AcpClient>(
    session: &SessionState,
    connection: &C,
    thinking: bool,
    text: String,
) -> Result<(), AcpError> {
    let chunk = ContentChunk::new(AcpContent::Text(TextContent::new(text)));
    let update = if thinking {
        SessionUpdate::AgentThoughtChunk(chunk)
    } else {
        SessionUpdate::AgentMessageChunk(chunk)
    };
    send_update(session, connection, update).await
}

async fn request_tool_permission<C: AcpClient>(
    session: &SessionState,
    connection: &C,
    id: &str,
    name: &str,
    arguments: serde_json::Value,
) -> Result<ToolPermissionDecision, AcpError> {
    let request = RequestPermissionRequest::new(
        session.id.clone(),
        ToolCallUpdate::new(
            ToolCallId::from(id.to_string()),
            ToolCallUpdateFields::new()
                .title(format!("Run {}", name))
                .kind(tool_name_to_kind(name))
                .raw_input(arguments),
        ),
        vec![
            PermissionOption::new(
                PermissionOptionId::new("allow-once"),
                "Allow once",
                PermissionOptionKind::AllowOnce,
            ),
            PermissionOption::new(
                PermissionOptionId::new("reject-once"),
                "Reject",
                PermissionOptionKind::RejectOnce,
            ),
        ],
    );

    let response = connection
        .request_permission(request)
        .await
        .map_err(|error| AcpError::ProtocolError(error.to_string()))?;
    Ok(match response.outcome {
        RequestPermissionOutcome::Selected(selected)
            if selected.option_id.0.as_ref().starts_with("allow") =>
        {
            ToolPermissionDecision::Approved
        }
        RequestPermissionOutcome::Selected(_) => ToolPermissionDecision::Denied,
        RequestPermissionOutcome::Cancelled => ToolPermissionDecision::Cancelled,
        _ => ToolPermissionDecision::Denied,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolPermissionDecision {
    Approved,
    Denied,
    Cancelled,
}

pub(super) fn canonical_acp_workspace_root(session: &SessionState) -> Result<PathBuf, AcpError> {
    session.cwd.canonicalize().map_err(|error| {
        AcpError::InvalidRequest(format!(
            "session working directory '{}' is not accessible: {}",
            session.cwd.display(),
            error
        ))
    })
}

fn canonical_mode_to_acp(mode: &str) -> &'static str {
    match mode {
        "plan" => "plan",
        _ => "code",
    }
}

pub(super) fn convert_loop_stop_reason(
    reason: LoopStopReason,
    last_error: Option<String>,
) -> Result<StopReason, AcpError> {
    match reason {
        LoopStopReason::Completed
        | LoopStopReason::AwaitingInput
        | LoopStopReason::Sleeping
        | LoopStopReason::Pinched => Ok(StopReason::EndTurn),
        LoopStopReason::BudgetExhausted | LoopStopReason::LoopGuardTriggered => {
            Ok(StopReason::MaxTurnRequests)
        }
        LoopStopReason::UserAbort => Ok(StopReason::Cancelled),
        LoopStopReason::ProviderError
        | LoopStopReason::StreamIdleTimeout
        | LoopStopReason::PinchFailed => {
            Err(AcpError::AiClientError(last_error.unwrap_or_else(|| {
                format!("canonical agent loop stopped: {:?}", reason)
            })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_stop_reasons_preserve_acp_semantics() {
        assert_eq!(
            convert_loop_stop_reason(LoopStopReason::Completed, None).unwrap(),
            StopReason::EndTurn
        );
        assert_eq!(
            convert_loop_stop_reason(LoopStopReason::BudgetExhausted, None).unwrap(),
            StopReason::MaxTurnRequests
        );
        assert_eq!(
            convert_loop_stop_reason(LoopStopReason::UserAbort, None).unwrap(),
            StopReason::Cancelled
        );
        assert!(convert_loop_stop_reason(
            LoopStopReason::ProviderError,
            Some("provider failed".to_string())
        )
        .is_err());
    }
}
