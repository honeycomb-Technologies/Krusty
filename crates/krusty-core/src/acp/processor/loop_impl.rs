use agent_client_protocol::{
    Client as AcpClient, ContentBlock as AcpContent, ContentChunk, PermissionOption,
    PermissionOptionId, PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallId,
    ToolCallUpdate, ToolCallUpdateFields,
};
use anyhow::Result;

use crate::ai::client::CallOptions;
use crate::ai::streaming::StreamPart;
use crate::ai::types::{AiToolCall, Content, FinishReason};
use crate::tools::git_identity::GitIdentityMode;
use crate::tools::registry::{tool_policy, PermissionMode};
use crate::tools::{ToolContext, ToolResult};

use super::content::convert_acp_content;
use super::PromptProcessor;
use crate::acp::error::AcpError;
use crate::acp::session::SessionState;
use crate::acp::tools::{
    create_tool_call_complete, create_tool_call_failed, create_tool_call_start,
    text_to_tool_content, tool_name_to_kind,
};

impl PromptProcessor {
    /// Process a prompt and stream results via the connection.
    pub async fn process_prompt<C: AcpClient>(
        &self,
        session: &SessionState,
        prompt: Vec<AcpContent>,
        connection: &C,
    ) -> Result<StopReason, AcpError> {
        let ai_client = self.ai_client.as_ref().ok_or_else(|| {
            AcpError::NotAuthenticated("AI client not initialized - authenticate first".into())
        })?;

        let initial_content: Vec<Content> =
            prompt.into_iter().filter_map(convert_acp_content).collect();
        if !initial_content.is_empty() {
            session
                .add_user_message_content(initial_content.clone())
                .await;
        }

        let recovery_notice = session.take_recovery_notice().await;
        let tool_defs = self.tools.get_ai_tools().await;
        let max_iterations = self.agent_config.acp_max_turns();

        for iteration in 0.. {
            if session.is_cancelled() {
                tracing::info!("Session cancelled");
                return Ok(StopReason::Cancelled);
            }

            if max_iterations.is_some_and(|max| iteration >= max) {
                tracing::warn!(
                    max_iterations = ?max_iterations,
                    "ACP agentic loop hit configured turn budget"
                );
                return Ok(StopReason::EndTurn);
            }

            tracing::info!("Agentic loop iteration {}", iteration + 1);
            let mut messages = session.history().await;
            if iteration == 0 {
                if let Some(recovery_notice) = recovery_notice.clone() {
                    let insert_idx = messages
                        .last()
                        .map(|message| usize::from(message.role != crate::ai::types::Role::User))
                        .filter(|insert_before_last| *insert_before_last == 0)
                        .map(|_| messages.len().saturating_sub(1))
                        .unwrap_or(messages.len());
                    messages.insert(insert_idx, recovery_notice);
                }
            }

            let options = CallOptions {
                tools: if tool_defs.is_empty() {
                    None
                } else {
                    Some(tool_defs.clone())
                },
                enable_caching: true,
                session_id: Some(session.id.to_string()),
                codex_parallel_tool_calls: true,
                ..Default::default()
            };

            let mut rx = ai_client
                .call_streaming(messages, &options)
                .await
                .map_err(|e| AcpError::AiClientError(e.to_string()))?;

            let mut accumulated_text = String::new();
            let mut pending_tool_calls: Vec<AiToolCall> = Vec::new();
            let mut stop_reason = StopReason::EndTurn;
            let stream_timeout = self.agent_config.stream_idle_timeout();

            loop {
                let part = match tokio::time::timeout(stream_timeout, rx.recv()).await {
                    Ok(Some(part)) => part,
                    Ok(None) => break,
                    Err(_) => {
                        tracing::warn!(
                            "Stream receive timed out after {}s",
                            stream_timeout.as_secs()
                        );
                        return Err(AcpError::AiClientError(
                            "Stream timed out waiting for response".into(),
                        ));
                    }
                };

                if session.is_cancelled() {
                    tracing::info!("Session cancelled, stopping stream processing");
                    return Ok(StopReason::Cancelled);
                }

                match part {
                    StreamPart::Start { model, provider } => {
                        tracing::debug!("Stream started: model={}, provider={}", model, provider);
                    }
                    StreamPart::TextDelta { delta } => {
                        accumulated_text.push_str(&delta);
                        let chunk = ContentChunk::new(AcpContent::Text(TextContent::new(&delta)));
                        let notification = SessionNotification::new(
                            session.id.clone(),
                            SessionUpdate::AgentMessageChunk(chunk),
                        );
                        if let Err(e) = connection.session_notification(notification).await {
                            tracing::warn!("Failed to send text chunk: {}", e);
                        }
                    }
                    StreamPart::ThinkingDelta { thinking, .. } => {
                        let chunk =
                            ContentChunk::new(AcpContent::Text(TextContent::new(&thinking)));
                        let notification = SessionNotification::new(
                            session.id.clone(),
                            SessionUpdate::AgentThoughtChunk(chunk),
                        );
                        if let Err(e) = connection.session_notification(notification).await {
                            tracing::warn!("Failed to send thought chunk: {}", e);
                        }
                    }
                    StreamPart::ToolCallStart { id, name } => {
                        tracing::debug!("Tool call starting: {} ({})", name, id);
                        let kind = tool_name_to_kind(&name);
                        let title = format!("Running {}", name);
                        let tool_call =
                            ToolCall::new(ToolCallId::from(id.clone()), title).kind(kind);
                        let notification = SessionNotification::new(
                            session.id.clone(),
                            SessionUpdate::ToolCall(tool_call),
                        );
                        if let Err(e) = connection.session_notification(notification).await {
                            tracing::warn!("Failed to send tool call start: {}", e);
                        }
                    }
                    StreamPart::ToolCallComplete { tool_call } => {
                        tracing::info!("Tool call complete: {} ({})", tool_call.name, tool_call.id);
                        pending_tool_calls.push(tool_call);
                    }
                    StreamPart::Finish { reason } => {
                        tracing::debug!("Stream finished: {:?}", reason);
                        stop_reason = convert_finish_reason(reason);
                    }
                    StreamPart::Error { error } => {
                        tracing::error!("Stream error: {}", error);
                        return Err(AcpError::AiClientError(error));
                    }
                    _ => {}
                }
            }

            if !accumulated_text.is_empty() {
                session
                    .add_assistant_message(accumulated_text.clone())
                    .await;
            }

            if pending_tool_calls.is_empty() {
                tracing::info!("Agentic loop complete after {} iterations", iteration + 1);
                return Ok(stop_reason);
            }

            self.execute_tool_calls(session, pending_tool_calls, connection)
                .await?;
        }

        #[allow(unreachable_code)]
        Ok(StopReason::EndTurn)
    }

    async fn execute_tool_calls<C: AcpClient>(
        &self,
        session: &SessionState,
        tool_calls: Vec<AiToolCall>,
        connection: &C,
    ) -> Result<StopReason, AcpError> {
        let ai_client = self.ai_client.clone().ok_or_else(|| {
            AcpError::NotAuthenticated("AI client not initialized - authenticate first".into())
        })?;

        let mut ctx = ToolContext {
            working_dir: session.cwd.clone(),
            ..Default::default()
        }
        .with_permission_mode(PermissionMode::Supervised)
        .with_subagent_max_turns(self.agent_config.subagent_max_turns)
        .with_ai_client(ai_client.clone());

        if let Some(ref identity) = self.git_identity {
            if identity.mode != GitIdentityMode::Disabled {
                ctx = ctx.with_git_identity(identity.clone());
            }
        }

        for tool_call in tool_calls {
            if session.is_cancelled() {
                return Ok(StopReason::Cancelled);
            }

            tracing::info!("Executing tool: {} ({})", tool_call.name, tool_call.id);
            let start_update =
                create_tool_call_start(&tool_call.id, &tool_call.name, tool_call.arguments.clone());
            let notification = SessionNotification::new(
                session.id.clone(),
                SessionUpdate::ToolCallUpdate(start_update),
            );
            if let Err(e) = connection.session_notification(notification).await {
                tracing::warn!("Failed to send tool start: {}", e);
            }

            let result = if requires_acp_permission(&tool_call) {
                match request_tool_permission(session, &tool_call, connection).await? {
                    ToolPermissionDecision::Allow => {
                        self.tools
                            .execute(&tool_call.name, tool_call.arguments.clone(), &ctx)
                            .await
                    }
                    ToolPermissionDecision::Deny => Some(ToolResult::error_with_code(
                        "permission_denied",
                        format!("Tool '{}' was denied by the ACP client", tool_call.name),
                    )),
                    ToolPermissionDecision::Cancelled => return Ok(StopReason::Cancelled),
                }
            } else {
                self.tools
                    .execute(&tool_call.name, tool_call.arguments.clone(), &ctx)
                    .await
            };

            let (update, output_for_history, is_error_for_history) = match &result {
                Some(ToolResult { output, is_error }) if !*is_error => {
                    tracing::info!("Tool {} completed successfully", tool_call.name);
                    let content = vec![text_to_tool_content(output)];
                    (
                        create_tool_call_complete(&tool_call.id, content),
                        Some(output.clone()),
                        false,
                    )
                }
                Some(ToolResult { output, is_error }) => {
                    tracing::warn!("Tool {} failed: {}", tool_call.name, output);
                    (
                        create_tool_call_failed(&tool_call.id, output),
                        Some(output.clone()),
                        *is_error,
                    )
                }
                None => {
                    let msg = format!("Tool '{}' not found", tool_call.name);
                    tracing::warn!("{}", msg);
                    (create_tool_call_failed(&tool_call.id, &msg), None, true)
                }
            };

            let notification =
                SessionNotification::new(session.id.clone(), SessionUpdate::ToolCallUpdate(update));
            if let Err(e) = connection.session_notification(notification).await {
                tracing::warn!("Failed to send tool result: {}", e);
            }

            session
                .add_tool_call(
                    tool_call.id.clone(),
                    tool_call.name.clone(),
                    tool_call.arguments.clone(),
                )
                .await;

            if let Some(ref output) = output_for_history {
                session
                    .add_tool_result(&tool_call.id, output.clone(), is_error_for_history)
                    .await;
            }
        }

        Ok(StopReason::EndTurn)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolPermissionDecision {
    Allow,
    Deny,
    Cancelled,
}

fn requires_acp_permission(tool_call: &AiToolCall) -> bool {
    tool_policy(&tool_call.name).requires_supervised_approval
}

async fn request_tool_permission<C: AcpClient>(
    session: &SessionState,
    tool_call: &AiToolCall,
    connection: &C,
) -> Result<ToolPermissionDecision, AcpError> {
    let request = RequestPermissionRequest::new(
        session.id.clone(),
        ToolCallUpdate::new(
            ToolCallId::from(tool_call.id.clone()),
            ToolCallUpdateFields::new()
                .title(format!("Run {}", tool_call.name))
                .kind(tool_name_to_kind(&tool_call.name))
                .raw_input(tool_call.arguments.clone()),
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
        .map_err(|e| AcpError::ProtocolError(e.to_string()))?;

    match response.outcome {
        RequestPermissionOutcome::Selected(selected)
            if selected.option_id.0.as_ref().starts_with("allow") =>
        {
            Ok(ToolPermissionDecision::Allow)
        }
        RequestPermissionOutcome::Selected(_) => Ok(ToolPermissionDecision::Deny),
        RequestPermissionOutcome::Cancelled => Ok(ToolPermissionDecision::Cancelled),
        _ => Ok(ToolPermissionDecision::Deny),
    }
}

/// Convert AI finish reason to ACP stop reason.
pub(super) fn convert_finish_reason(reason: FinishReason) -> StopReason {
    match reason {
        FinishReason::Stop => StopReason::EndTurn,
        FinishReason::Length => StopReason::MaxTokens,
        FinishReason::ToolCalls => StopReason::EndTurn,
        FinishReason::ContentFilter => StopReason::EndTurn,
        FinishReason::Other(_) => StopReason::EndTurn,
    }
}
