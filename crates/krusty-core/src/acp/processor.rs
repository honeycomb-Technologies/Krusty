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
    Client as AcpClient, ContentBlock as AcpContent, ContentChunk, EmbeddedResourceResource,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallId,
};
use anyhow::Result;
use tracing::{debug, error, info, warn};

use crate::ai::client::{AiClient, AiClientConfig, CallOptions};
use crate::ai::format_detection::detect_api_format;
use crate::ai::providers::{get_provider, AuthHeader, ProviderId};
use crate::ai::streaming::StreamPart;
use crate::ai::types::{AiToolCall, Content, FinishReason};
use crate::tools::git_identity::{GitIdentity, GitIdentityMode};
use crate::tools::{ToolContext, ToolRegistry, ToolResult};

use super::error::AcpError;
use super::session::SessionState;
use super::tools::{
    create_tool_call_complete, create_tool_call_failed, create_tool_call_start,
    text_to_tool_content, tool_name_to_kind,
};

/// Prompt processor that connects ACP to Krusty's AI and tools
pub struct PromptProcessor {
    /// AI client for making inference calls
    ai_client: Option<Arc<AiClient>>,
    /// Tool registry for executing tools
    tools: Arc<ToolRegistry>,
    /// Working directory for tool execution
    cwd: PathBuf,
    /// Git identity for commit attribution
    git_identity: Option<GitIdentity>,
}

impl PromptProcessor {
    /// Create a new prompt processor
    pub fn new(tools: Arc<ToolRegistry>, cwd: PathBuf) -> Self {
        Self {
            ai_client: None,
            tools,
            cwd,
            git_identity: Some(GitIdentity::default()),
        }
    }

    /// Update the working directory (called when session cwd changes)
    pub fn set_cwd(&mut self, cwd: PathBuf) {
        self.cwd = cwd;
    }

    /// Set git identity for commit attribution
    pub fn set_git_identity(&mut self, identity: GitIdentity) {
        self.git_identity = Some(identity);
    }

    /// Initialize the AI client with an API key and optional model override
    pub fn init_ai_client(
        &mut self,
        api_key: String,
        provider: ProviderId,
        model_override: Option<String>,
    ) {
        use std::collections::HashMap;

        // Get provider configuration from the registry
        let provider_config = get_provider(provider);

        let (model, base_url, auth_header, custom_headers) = if let Some(pc) = provider_config {
            let model = model_override.unwrap_or_else(|| pc.default_model().to_string());
            (
                model,
                Some(pc.base_url.clone()),
                pc.auth_header,
                pc.custom_headers.clone(),
            )
        } else {
            // Fallback for unknown providers
            let model = model_override.unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
            (model, None, AuthHeader::XApiKey, HashMap::new())
        };

        // Determine API format based on provider and model
        let api_format = detect_api_format(provider, &model);

        let config = AiClientConfig {
            model: model.clone(),
            max_tokens: 8192,
            base_url,
            auth_header,
            provider_id: provider,
            api_format,
            custom_headers,
        };

        let client = Arc::new(AiClient::new(config, api_key));
        self.ai_client = Some(client);

        info!(
            "AI client initialized: provider={:?}, model={}",
            provider, model
        );
    }

    /// Process a prompt and stream results via the connection
    ///
    /// Implements an agentic loop: after tool execution, continues calling the AI
    /// with tool results until the AI responds without requesting more tools.
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

        // Convert initial ACP content to Krusty messages and add to history
        // Handle Text, Resource (embedded files), and ResourceLink (file references)
        let initial_content: Vec<Content> =
            prompt.into_iter().filter_map(convert_acp_content).collect();

        if !initial_content.is_empty() {
            session
                .add_user_message_content(initial_content.clone())
                .await;
        }

        // Get tool definitions for the AI
        let tool_defs = self.tools.get_ai_tools().await;

        // Agentic loop - continue until AI stops requesting tools
        const MAX_ITERATIONS: usize = 50; // Safety limit
        for iteration in 0..MAX_ITERATIONS {
            if session.is_cancelled() {
                info!("Session cancelled");
                return Ok(StopReason::Cancelled);
            }

            info!("Agentic loop iteration {}", iteration + 1);

            // Get current conversation history
            let messages = session.history().await;

            // Set up call options
            let options = CallOptions {
                tools: if tool_defs.is_empty() {
                    None
                } else {
                    Some(tool_defs.clone())
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
                        let chunk =
                            ContentChunk::new(AcpContent::Text(TextContent::new(&thinking)));
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
                        // Send initial tool call notification with proper kind
                        let kind = tool_name_to_kind(&name);
                        let title = format!("Running {}", name);
                        let tool_call =
                            ToolCall::new(ToolCallId::from(id.clone()), title).kind(kind);
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

            // Add the assistant's text response to conversation history
            if !accumulated_text.is_empty() {
                session
                    .add_assistant_message(accumulated_text.clone())
                    .await;
            }

            // If no tool calls, we're done
            if pending_tool_calls.is_empty() {
                info!("Agentic loop complete after {} iterations", iteration + 1);
                return Ok(stop_reason);
            }

            // Execute tool calls and add results to history
            self.execute_tool_calls(session, pending_tool_calls, connection)
                .await?;

            // Loop continues - AI will be called again with tool results in history
        }

        warn!("Agentic loop hit maximum iterations ({})", MAX_ITERATIONS);
        Ok(StopReason::EndTurn)
    }

    /// Execute tool calls and stream their results
    async fn execute_tool_calls<C: AcpClient>(
        &self,
        session: &SessionState,
        tool_calls: Vec<AiToolCall>,
        connection: &C,
    ) -> Result<StopReason, AcpError> {
        let mut ctx = ToolContext {
            working_dir: self.cwd.clone(),
            ..Default::default()
        };

        if let Some(ref identity) = self.git_identity {
            if identity.mode != GitIdentityMode::Disabled {
                ctx = ctx.with_git_identity(identity.clone());
            }
        }

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

            if let Some(ref output) = output_for_history {
                session
                    .add_tool_result(&tool_call.id, output.clone(), is_error_for_history)
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

/// Convert ACP content block to Krusty's Content type
///
/// Handles:
/// - Text: Direct text content
/// - Resource: Embedded file content (from @-file mentions in Zed)
/// - ResourceLink: Reference to a file (formatted as context)
/// - Image/Audio: Logged but not yet supported
fn convert_acp_content(block: AcpContent) -> Option<Content> {
    match block {
        AcpContent::Text(text) => Some(Content::Text { text: text.text }),

        // Embedded resource - file content directly included
        // This is what Zed sends when user @-mentions a file
        AcpContent::Resource(embedded) => {
            match embedded.resource {
                EmbeddedResourceResource::TextResourceContents(text_resource) => {
                    // Format as a code block with file path
                    let formatted = format!(
                        "File: {}\n```\n{}\n```",
                        text_resource.uri, text_resource.text
                    );
                    debug!(
                        "Embedded resource: {} ({} bytes)",
                        text_resource.uri,
                        text_resource.text.len()
                    );
                    Some(Content::Text { text: formatted })
                }
                EmbeddedResourceResource::BlobResourceContents(blob) => {
                    // Binary content - note its presence but don't include raw data
                    let formatted = format!(
                        "[Binary file: {} ({})]",
                        blob.uri,
                        blob.mime_type.as_deref().unwrap_or("unknown type")
                    );
                    debug!("Binary resource: {}", blob.uri);
                    Some(Content::Text { text: formatted })
                }
                // Handle future resource types
                _ => {
                    warn!("Unknown embedded resource type, skipping");
                    None
                }
            }
        }

        // Resource link - reference to a file the agent could read
        AcpContent::ResourceLink(link) => {
            // Format as a file reference for context
            let formatted = if let Some(desc) = link.description {
                format!("[File reference: {} - {}]", link.uri, desc)
            } else {
                format!("[File reference: {}]", link.uri)
            };
            debug!("Resource link: {}", link.uri);
            Some(Content::Text { text: formatted })
        }

        // Image content - not yet supported
        AcpContent::Image(_) => {
            warn!("Image content not yet supported, skipping");
            None
        }

        // Audio content - not yet supported
        AcpContent::Audio(_) => {
            warn!("Audio content not yet supported, skipping");
            None
        }

        // Handle future content block types
        _ => {
            warn!("Unknown content block type, skipping");
            None
        }
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
        let provider = get_provider(ProviderId::MiniMax).unwrap();
        let model = provider.default_model();
        assert!(model.contains("MiniMax"));
    }
}
