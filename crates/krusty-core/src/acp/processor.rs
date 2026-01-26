//! ACP Prompt Processor
//!
//! Connects the ACP agent to Krusty's AI client and tool system.
//! Handles the core prompt processing loop:
//! 1. Convert ACP content blocks to Krusty's AI format
//! 2. Call AI provider with streaming
//! 3. Stream responses back via ACP session/update notifications
//! 4. Execute tool calls and stream their results
//!
//! ## Dual-Mind Integration
//! When enabled, Little Claw reviews Big Claw's actions:
//! - Pre-review: Before executing tool calls
//! - Post-review: After tool results
//! - Dialogue streamed as thought chunks

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::{
    Client as AcpClient, ContentBlock as AcpContent, ContentChunk, EmbeddedResourceResource,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallId,
};
use anyhow::Result;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::agent::dual_mind::{
    DialogueResult, DialogueTurn, DualMind, DualMindConfig, Observation,
};
use crate::agent::AgentCancellation;
use crate::ai::client::{AiClient, AiClientConfig, CallOptions};
use crate::ai::format_detection::detect_api_format;
use crate::ai::providers::{get_provider, AuthHeader, ProviderId};
use crate::ai::streaming::StreamPart;
use crate::ai::types::{AiToolCall, Content, FinishReason};
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
    /// Dual-mind system (Big Claw / Little Claw) - wrapped in RwLock for interior mutability
    dual_mind: Option<Arc<RwLock<DualMind>>>,
    /// Dual-mind configuration
    dual_mind_config: DualMindConfig,
}

impl PromptProcessor {
    /// Create a new prompt processor
    pub fn new(tools: Arc<ToolRegistry>, cwd: PathBuf) -> Self {
        Self {
            ai_client: None,
            tools,
            cwd,
            dual_mind: None,
            dual_mind_config: DualMindConfig::default(),
        }
    }

    /// Create with dual-mind disabled
    pub fn without_dual_mind(tools: Arc<ToolRegistry>, cwd: PathBuf) -> Self {
        let mut processor = Self::new(tools, cwd);
        processor.dual_mind_config.enabled = false;
        processor
    }

    /// Enable or disable dual-mind
    pub async fn set_dual_mind_enabled(&self, enabled: bool) {
        if let Some(dm) = &self.dual_mind {
            let mut dm = dm.write().await;
            if enabled {
                dm.enable();
            } else {
                dm.disable();
            }
        }
    }

    /// Update the working directory (called when session cwd changes)
    pub fn set_cwd(&mut self, cwd: PathBuf) {
        self.cwd = cwd;
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
        self.ai_client = Some(client.clone());

        // Initialize dual-mind if enabled
        if self.dual_mind_config.enabled {
            let cancellation = AgentCancellation::new();
            let dual_mind = DualMind::with_tools(
                client,
                cancellation,
                self.dual_mind_config.clone(),
                self.tools.clone(),
                self.cwd.clone(),
            );
            self.dual_mind = Some(Arc::new(RwLock::new(dual_mind)));
            info!("Dual-mind system initialized (Little Claw active with research tools)");
        }

        info!(
            "AI client initialized: provider={:?}, model={}, dual_mind={}",
            provider, model, self.dual_mind_config.enabled
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

            // Little Claw pre-review: Question the intent before execution
            if let Some(dual_mind) = &self.dual_mind {
                let intent = format_tool_calls_intent(&pending_tool_calls);

                let (review_result, dialogue) = {
                    let mut dm = dual_mind.write().await;
                    let result = dm.pre_review(&intent).await;
                    let dialogue = dm.take_dialogue();
                    (result, dialogue)
                };

                // Stream the dialogue as thought chunks
                stream_dialogue_turns(session, connection, &dialogue).await;

                // Handle review result
                match review_result {
                    DialogueResult::NeedsEnhancement { critique, .. } => {
                        info!("Little Claw raised pre-action concerns");

                        // Inject Little Claw's concerns into the conversation
                        // Big Claw will see this before proceeding
                        let concern_prompt = format!(
                            "[Quality Review - Pre-Action Concern]\n\n\
                            Before executing, Little Claw has raised a concern:\n\n\
                            {}\n\n\
                            Consider this feedback. If you believe your approach is correct, \
                            explain briefly why and proceed. If the concern is valid, \
                            adjust your approach.",
                            critique
                        );

                        session.add_system_context(concern_prompt).await;

                        // Stream thought notification
                        let chunk = ContentChunk::new(AcpContent::Text(TextContent::new(format!(
                            "\n[Little Claw: {}]\n",
                            critique
                        ))));
                        let notification = SessionNotification::new(
                            session.id.clone(),
                            SessionUpdate::AgentThoughtChunk(chunk),
                        );
                        let _ = connection.session_notification(notification).await;

                        // Don't execute these tool calls - loop back for Big Claw to respond
                        continue;
                    }
                    DialogueResult::Consensus { .. } => {
                        debug!("Little Claw approved the action");
                    }
                    _ => {}
                }
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

            if let Some(ref output) = output_for_history {
                session
                    .add_tool_result(&tool_call.id, output.clone(), is_error_for_history)
                    .await;

                // Sync observation to Little Claw
                if let Some(dual_mind) = &self.dual_mind {
                    let observation =
                        create_observation(&tool_call.name, output, !is_error_for_history);
                    let dm = dual_mind.read().await;
                    dm.sync_observation(observation).await;
                }

                // Little Claw post-review: Validate the output
                if let Some(dual_mind) = &self.dual_mind {
                    // Only review significant outputs (not trivial reads)
                    if should_post_review(&tool_call.name, output) {
                        let (review_result, dialogue) = {
                            let mut dm = dual_mind.write().await;
                            let result = dm.post_review(output).await;
                            let dialogue = dm.take_dialogue();
                            (result, dialogue)
                        };

                        // Stream the dialogue as thought chunks
                        stream_dialogue_turns(session, connection, &dialogue).await;

                        // Handle review result - trigger enhancement sweep if needed
                        if let DialogueResult::NeedsEnhancement { critique, .. } = review_result {
                            info!("Little Claw found issues, triggering enhancement sweep");

                            // Inject the critique into the conversation as a system message
                            // Big Claw will see this and address the issues
                            let enhancement_prompt = format!(
                                "[Quality Review - Enhancement Required]\n\n\
                                Little Claw has identified issues with the recent output:\n\n\
                                {}\n\n\
                                Please address these concerns and enhance the code accordingly. \
                                Focus on the specific issues mentioned.",
                                critique
                            );

                            session.add_system_context(enhancement_prompt).await;

                            // Stream notification to user that enhancement is happening
                            let chunk = ContentChunk::new(AcpContent::Text(TextContent::new(
                                "\n[Enhancing based on quality review...]\n",
                            )));
                            let notification = SessionNotification::new(
                                session.id.clone(),
                                SessionUpdate::AgentMessageChunk(chunk),
                            );
                            let _ = connection.session_notification(notification).await;
                        }
                    }
                }
            }
        }

        // Tool calls completed - model should continue
        Ok(StopReason::EndTurn)
    }
}

/// Stream dialogue turns as thought chunks
async fn stream_dialogue_turns<C: AcpClient>(
    session: &SessionState,
    connection: &C,
    dialogue: &[DialogueTurn],
) {
    if dialogue.is_empty() {
        return;
    }

    // Format dialogue for display
    let mut formatted = String::new();
    for turn in dialogue {
        formatted.push_str(&format!(
            "[{}] {}\n\n",
            turn.speaker.display_name(),
            turn.content
        ));
    }

    // Stream as thought chunk
    let chunk = ContentChunk::new(AcpContent::Text(TextContent::new(&formatted)));
    let notification =
        SessionNotification::new(session.id.clone(), SessionUpdate::AgentThoughtChunk(chunk));
    if let Err(e) = connection.session_notification(notification).await {
        warn!("Failed to send dual-mind dialogue: {}", e);
    }

    debug!("Streamed {} dialogue turns", dialogue.len());
}

/// Format tool calls into a human-readable intent description
fn format_tool_calls_intent(tool_calls: &[AiToolCall]) -> String {
    if tool_calls.len() == 1 {
        let tc = &tool_calls[0];
        format!(
            "Execute {} with arguments: {}",
            tc.name,
            serde_json::to_string_pretty(&tc.arguments)
                .unwrap_or_else(|_| tc.arguments.to_string())
        )
    } else {
        let names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
        format!("Execute {} tools: {}", tool_calls.len(), names.join(", "))
    }
}

/// Create an observation from a tool result
fn create_observation(tool_name: &str, output: &str, success: bool) -> Observation {
    match tool_name {
        "Edit" | "edit" => {
            // Extract file path from output if possible
            Observation::file_edit("unknown", "File edited", output)
        }
        "Write" | "write" => Observation::file_write("unknown", "File written"),
        "Bash" | "bash" => Observation::bash("command", output, success),
        _ => Observation::tool_result(tool_name, output, success),
    }
}

/// Determine if a tool output warrants post-review
fn should_post_review(tool_name: &str, output: &str) -> bool {
    // Skip review for read-only operations with small output
    match tool_name {
        "Read" | "read" | "Glob" | "glob" | "Grep" | "grep" => {
            // Only review if output is substantial (might indicate complexity)
            output.len() > 2000
        }
        "Edit" | "edit" | "Write" | "write" => {
            // Always review file modifications
            true
        }
        "Bash" | "bash" => {
            // Review bash commands that produced output
            !output.trim().is_empty()
        }
        _ => {
            // Default: review if output is non-trivial
            output.len() > 500
        }
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
        let provider = get_provider(ProviderId::Anthropic).unwrap();
        let model = provider.default_model();
        assert!(model.contains("claude"));
    }

    #[test]
    fn test_should_post_review() {
        // Read operations with small output should not trigger review
        assert!(!should_post_review("Read", "small content"));

        // Edit operations should always trigger review
        assert!(should_post_review("Edit", "any content"));
        assert!(should_post_review("Write", ""));

        // Bash with output should trigger review
        assert!(should_post_review("Bash", "command output"));
        assert!(!should_post_review("Bash", ""));
    }

    #[test]
    fn test_format_tool_calls_intent() {
        let calls = vec![AiToolCall {
            id: "1".to_string(),
            name: "Edit".to_string(),
            arguments: serde_json::json!({"file": "test.rs"}),
        }];

        let intent = format_tool_calls_intent(&calls);
        assert!(intent.contains("Edit"));
        assert!(intent.contains("test.rs"));
    }
}
