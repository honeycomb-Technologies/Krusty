//! ACP Prompt Processor
//!
//! Connects the ACP agent to Krusty's AI client and tool system.
//! Handles the core prompt processing loop:
//! 1. Convert ACP content blocks to Krusty's AI format
//! 2. Call AI provider with streaming
//! 3. Stream responses back via ACP session/update notifications
//! 4. Execute tool calls and stream their results

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::{
    Client as AcpClient, ContentBlock as AcpContent, ContentChunk, SessionId, SessionNotification,
    SessionUpdate, StopReason, TextContent, ToolCall, ToolCallId,
};
use anyhow::Result;
use tracing::{debug, error, info, warn};

use crate::ai::client::{AiClient, AiClientConfig, CallOptions};
use crate::ai::models::ApiFormat;
use crate::ai::providers::{get_provider, AuthHeader, ProviderId};
use crate::ai::streaming::StreamPart;
use crate::ai::types::{AiToolCall, Content, FinishReason, ModelMessage, Role};
use crate::tools::{ToolContext, ToolRegistry, ToolResult};

use super::error::AcpError;
use super::session::SessionState;
use super::tools::{
    create_tool_call_complete, create_tool_call_failed, create_tool_call_start,
    text_to_tool_content,
};

/// Prompt processor that connects ACP to Krusty's AI and tools
pub struct PromptProcessor {
    /// AI client for making inference calls
    ai_client: Option<AiClient>,
    /// Tool registry for executing tools
    tools: Arc<ToolRegistry>,
    /// Working directory for tool execution
    cwd: PathBuf,
}

impl PromptProcessor {
    /// Create a new prompt processor
    pub fn new(tools: Arc<ToolRegistry>, cwd: PathBuf) -> Self {
        Self {
            ai_client: None,
            tools,
            cwd,
        }
    }

    /// Initialize the AI client with an API key and optional model override
    pub fn init_ai_client(
        &mut self,
        api_key: String,
        provider: ProviderId,
        model_override: Option<String>,
    ) {
        // Get provider configuration from the registry
        let provider_config = get_provider(provider);

        let (model, base_url, auth_header) = if let Some(pc) = provider_config {
            let model = model_override.unwrap_or_else(|| pc.default_model().to_string());
            (model, Some(pc.base_url.clone()), pc.auth_header)
        } else {
            // Fallback for unknown providers
            let model = model_override.unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
            (model, None, AuthHeader::XApiKey)
        };

        let config = AiClientConfig {
            model: model.clone(),
            max_tokens: 8192,
            base_url,
            auth_header,
            provider_id: provider,
            api_format: ApiFormat::Anthropic, // All supported providers use Anthropic-compatible API
        };

        self.ai_client = Some(AiClient::new(config, api_key));
        info!(
            "AI client initialized: provider={:?}, model={}",
            provider, model
        );
    }

    /// Process a prompt and stream results via the connection
    ///
    /// Returns the stop reason when processing completes
    pub async fn process_prompt<C: AcpClient>(
        &self,
        session: &SessionState,
        prompt: Vec<AcpContent>,
        connection: &C,
    ) -> Result<StopReason, AcpError> {
        let ai_client = self.ai_client.as_ref().ok_or_else(|| {
            AcpError::NotAuthenticated("AI client not initialized - authenticate first".into())
        })?;

        // Convert ACP content to Krusty messages
        let messages =
            self.convert_prompt_to_messages(&session.id, prompt, &session.history().await);

        // Get tool definitions for the AI
        let tool_defs = self.tools.get_ai_tools().await;

        // Set up call options
        let options = CallOptions {
            tools: if tool_defs.is_empty() {
                None
            } else {
                Some(tool_defs)
            },
            enable_caching: true,
            ..Default::default()
        };

        // Call AI with streaming
        let mut rx = ai_client
            .call_streaming(messages, &options)
            .await
            .map_err(|e| AcpError::AiClientError(e.to_string()))?;

        // Process stream and send updates
        let mut accumulated_text = String::new();
        let mut pending_tool_calls: Vec<AiToolCall> = Vec::new();
        let mut stop_reason = StopReason::EndTurn;

        while let Some(part) = rx.recv().await {
            if session.is_cancelled() {
                info!("Session cancelled, stopping stream processing");
                return Ok(StopReason::Cancelled);
            }

            match part {
                StreamPart::Start { model, provider } => {
                    debug!("Stream started: model={}, provider={}", model, provider);
                }

                StreamPart::TextDelta { delta } => {
                    accumulated_text.push_str(&delta);
                    // Stream text chunk to client
                    let chunk = ContentChunk::new(AcpContent::Text(TextContent::new(&delta)));
                    let notification = SessionNotification::new(
                        session.id.clone(),
                        SessionUpdate::AgentMessageChunk(chunk),
                    );
                    if let Err(e) = connection.session_notification(notification).await {
                        warn!("Failed to send text chunk: {}", e);
                    }
                }

                StreamPart::ThinkingDelta { thinking, .. } => {
                    // Stream thinking as thought chunk
                    let chunk = ContentChunk::new(AcpContent::Text(TextContent::new(&thinking)));
                    let notification = SessionNotification::new(
                        session.id.clone(),
                        SessionUpdate::AgentThoughtChunk(chunk),
                    );
                    if let Err(e) = connection.session_notification(notification).await {
                        warn!("Failed to send thought chunk: {}", e);
                    }
                }

                StreamPart::ToolCallStart { id, name } => {
                    debug!("Tool call starting: {} ({})", name, id);
                    // Send initial tool call notification
                    let tool_call = ToolCall::new(ToolCallId::from(id.clone()), name.clone());
                    let notification = SessionNotification::new(
                        session.id.clone(),
                        SessionUpdate::ToolCall(tool_call),
                    );
                    if let Err(e) = connection.session_notification(notification).await {
                        warn!("Failed to send tool call start: {}", e);
                    }
                }

                StreamPart::ToolCallComplete { tool_call } => {
                    info!("Tool call complete: {} ({})", tool_call.name, tool_call.id);
                    pending_tool_calls.push(tool_call);
                }

                StreamPart::Finish { reason } => {
                    debug!("Stream finished: {:?}", reason);
                    stop_reason = convert_finish_reason(reason);
                }

                StreamPart::Error { error } => {
                    error!("Stream error: {}", error);
                    return Err(AcpError::AiClientError(error));
                }

                _ => {
                    // Handle other stream parts as needed
                }
            }
        }

        // Execute any pending tool calls
        if !pending_tool_calls.is_empty() {
            stop_reason = self
                .execute_tool_calls(session, pending_tool_calls, connection)
                .await?;
        }

        // Add the response to conversation history
        if !accumulated_text.is_empty() {
            session.add_assistant_message(accumulated_text).await;
        }

        Ok(stop_reason)
    }

    /// Convert ACP prompt content to Krusty ModelMessage format
    fn convert_prompt_to_messages(
        &self,
        _session_id: &SessionId,
        prompt: Vec<AcpContent>,
        history: &[ModelMessage],
    ) -> Vec<ModelMessage> {
        let mut messages = history.to_vec();

        // Convert new prompt content
        let content: Vec<Content> = prompt
            .into_iter()
            .filter_map(|block| match block {
                AcpContent::Text(text) => Some(Content::Text { text: text.text }),
                _ => {
                    // TODO: Handle images, resources, etc.
                    None
                }
            })
            .collect();

        if !content.is_empty() {
            messages.push(ModelMessage {
                role: Role::User,
                content,
            });
        }

        messages
    }

    /// Execute tool calls and stream their results
    async fn execute_tool_calls<C: AcpClient>(
        &self,
        session: &SessionState,
        tool_calls: Vec<AiToolCall>,
        connection: &C,
    ) -> Result<StopReason, AcpError> {
        let ctx = ToolContext {
            working_dir: self.cwd.clone(),
            ..Default::default()
        };

        for tool_call in tool_calls {
            if session.is_cancelled() {
                return Ok(StopReason::Cancelled);
            }

            info!("Executing tool: {} ({})", tool_call.name, tool_call.id);

            // Send tool call start update
            let start_update =
                create_tool_call_start(&tool_call.id, &tool_call.name, tool_call.arguments.clone());
            let notification = SessionNotification::new(
                session.id.clone(),
                SessionUpdate::ToolCallUpdate(start_update),
            );
            if let Err(e) = connection.session_notification(notification).await {
                warn!("Failed to send tool start: {}", e);
            }

            // Execute the tool
            let result = self
                .tools
                .execute(&tool_call.name, tool_call.arguments.clone(), &ctx)
                .await;

            // Send tool call result
            let (update, output_for_history, is_error_for_history) = match &result {
                Some(ToolResult { output, is_error }) if !*is_error => {
                    info!("Tool {} completed successfully", tool_call.name);
                    let content = vec![text_to_tool_content(output)];
                    (
                        create_tool_call_complete(&tool_call.id, content),
                        Some(output.clone()),
                        false,
                    )
                }
                Some(ToolResult { output, is_error }) => {
                    warn!("Tool {} failed: {}", tool_call.name, output);
                    (
                        create_tool_call_failed(&tool_call.id, output),
                        Some(output.clone()),
                        *is_error,
                    )
                }
                None => {
                    let msg = format!("Tool '{}' not found", tool_call.name);
                    warn!("{}", msg);
                    (create_tool_call_failed(&tool_call.id, &msg), None, true)
                }
            };

            let notification =
                SessionNotification::new(session.id.clone(), SessionUpdate::ToolCallUpdate(update));
            if let Err(e) = connection.session_notification(notification).await {
                warn!("Failed to send tool result: {}", e);
            }

            // Add tool call and result to session history
            session
                .add_tool_call(
                    tool_call.id.clone(),
                    tool_call.name.clone(),
                    tool_call.arguments.clone(),
                )
                .await;

            if let Some(output) = output_for_history {
                session
                    .add_tool_result(&tool_call.id, output, is_error_for_history)
                    .await;
            }
        }

        // Tool calls completed - model should continue
        Ok(StopReason::EndTurn)
    }
}

/// Convert AI finish reason to ACP stop reason
fn convert_finish_reason(reason: FinishReason) -> StopReason {
    match reason {
        FinishReason::Stop => StopReason::EndTurn,
        FinishReason::Length => StopReason::MaxTokens,
        FinishReason::ToolCalls => StopReason::EndTurn,
        FinishReason::ContentFilter => StopReason::EndTurn,
        FinishReason::Other(_) => StopReason::EndTurn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_finish_reason() {
        assert!(matches!(
            convert_finish_reason(FinishReason::Stop),
            StopReason::EndTurn
        ));
        assert!(matches!(
            convert_finish_reason(FinishReason::ToolCalls),
            StopReason::EndTurn
        ));
        assert!(matches!(
            convert_finish_reason(FinishReason::Length),
            StopReason::MaxTokens
        ));
    }

    #[test]
    fn test_default_model() {
        let provider = get_provider(ProviderId::Anthropic).unwrap();
        let model = provider.default_model();
        assert!(model.contains("claude"));
    }
}
