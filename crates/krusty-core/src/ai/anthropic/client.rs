//! Anthropic Claude API client
//!
//! Implements the Anthropic API with proper header handling for both
//! OAuth tokens and API keys.

use anyhow::Result;
use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::ai::sse::{
    create_streaming_channels, parse_finish_reason, spawn_buffer_processor, ServerToolAccumulator,
    SseEvent, SseParser, SseStreamProcessor, ThinkingAccumulator, ToolCallAccumulator,
};
use crate::ai::streaming::StreamPart;
use crate::ai::transform::build_provider_params;
use crate::ai::types::{
    AiTool, Citation, Content, ContextEditingMetrics, ContextManagement, FinishReason,
    ModelMessage, Role, ThinkingConfig, Usage, WebFetchConfig, WebFetchContent, WebSearchConfig,
    WebSearchResult,
};
use crate::auth::token_manager::TokenManager;
use crate::constants;

const DEFAULT_API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

/// OAuth token prefix - tokens starting with this use Bearer auth
const OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat";

/// Krusty's core philosophy and behavioral guidance
const KRUSTY_SYSTEM_PROMPT: &str = r#"You are Krusty, an AI coding assistant. You say what needs to be said, not what people want to hear. You're hard on code because bad code hurts the people who maintain it.

## Beliefs

- Every line of code is a liability. Less code means fewer bugs.
- Simplicity is mastery. A simple solution to a complex problem shows deep understanding. Clever code that "might work" loses to simple code that does work.
- Working code beats theoretical elegance. Ship it or delete it.
- No half-measures. Complete the feature or don't start it. No TODOs, no "future work", no partial implementations.

## Before Writing Code

- Does this need to exist?
- Is there a simpler way?
- Am I solving the right problem?
- What can I delete instead of add?

## You Don't

- Add defensive code against impossible states
- Build abstractions until the pattern appears 3+ times
- Write "infrastructure for later"
- Leave dead code or commented-out code
- Add features not requested

## Tool Discipline

Use specialized tools over shell commands:
- Read over cat/head/tail
- Edit over sed/awk
- Write over echo/cat redirects
- Glob over find/ls
- Grep over grep/rg commands

## File Operations

- Read existing files before modifying
- Prefer Edit over Write for existing files
- Don't create docs/READMEs unless asked

## Git Discipline

- Never force push, never skip hooks
- Commit messages explain WHY, not WHAT
- Each commit leaves codebase working

## Quality Bar

Before any commit:
- Zero compiler/linter warnings
- All tests pass
- No dead code

## Communication

You are honest. If an approach is wrong, you say so directly. No excessive praise. No flattery. Just the work."#;

use crate::ai::models::ApiFormat;
use crate::ai::providers::{AuthHeader, ProviderCapabilities, ProviderId, ReasoningFormat};

/// Configuration for the Anthropic client
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub model: String,
    pub max_tokens: usize,
    /// Optional base URL override (defaults to Anthropic API)
    pub base_url: Option<String>,
    /// How to send authentication header
    pub auth_header: AuthHeader,
    /// Which provider this config is for
    pub provider_id: ProviderId,
    /// API format for this model (for multi-format providers like OpenCode Zen)
    pub api_format: ApiFormat,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            model: constants::ai::DEFAULT_MODEL.to_string(),
            max_tokens: constants::ai::MAX_OUTPUT_TOKENS,
            base_url: None,
            auth_header: AuthHeader::XApiKey,
            provider_id: ProviderId::Anthropic,
            api_format: ApiFormat::Anthropic,
        }
    }
}

impl AnthropicConfig {
    /// Get the API URL to use
    ///
    /// For OpenCode Zen, routes to correct endpoint based on model's API format:
    /// - Anthropic format → /v1/messages
    /// - OpenAI format → /v1/chat/completions
    /// - OpenAI Responses → /v1/responses
    /// - Google format → /v1/models/{model} (not implemented yet)
    pub fn api_url(&self) -> String {
        if let Some(base) = &self.base_url {
            // For OpenCode Zen, modify the endpoint based on format
            if self.provider_id == ProviderId::OpenCodeZen {
                let base_without_endpoint = base
                    .trim_end_matches("/messages")
                    .trim_end_matches("/chat/completions")
                    .trim_end_matches("/responses");

                return match self.api_format {
                    ApiFormat::Anthropic => format!("{}/messages", base_without_endpoint),
                    ApiFormat::OpenAI => format!("{}/chat/completions", base_without_endpoint),
                    ApiFormat::OpenAIResponses => format!("{}/responses", base_without_endpoint),
                    ApiFormat::Google => format!("{}/models/{}", base_without_endpoint, self.model),
                };
            }
            base.clone()
        } else {
            DEFAULT_API_URL.to_string()
        }
    }

    /// Check if this config is for the native Anthropic API (for OAuth-specific logic)
    pub fn is_anthropic(&self) -> bool {
        self.provider_id == ProviderId::Anthropic
    }

    /// Get the provider ID
    pub fn provider_id(&self) -> ProviderId {
        self.provider_id
    }

    /// Check if this config uses OpenAI chat/completions format
    pub fn uses_openai_format(&self) -> bool {
        matches!(
            self.api_format,
            ApiFormat::OpenAI | ApiFormat::OpenAIResponses
        )
    }

    /// Check if this provider uses Anthropic-compatible API
    /// All our providers (Anthropic, OpenRouter, Z.ai, MiniMax, Kimi) use Anthropic Messages API
    /// Exception: OpenCode Zen routes some models to OpenAI format
    pub fn uses_anthropic_api(&self) -> bool {
        !self.uses_openai_format()
    }
}

/// Call options for API requests
#[derive(Debug, Clone)]
pub struct CallOptions {
    pub max_tokens: Option<usize>,
    pub temperature: Option<f32>,
    pub tools: Option<Vec<AiTool>>,
    pub system_prompt: Option<String>,
    /// Extended thinking configuration (Anthropic-style)
    pub thinking: Option<ThinkingConfig>,
    /// Universal reasoning format - determines how to encode reasoning in requests
    /// When Some, enables reasoning for the model using the appropriate format
    pub reasoning_format: Option<ReasoningFormat>,
    /// Enable prompt caching (default: true)
    pub enable_caching: bool,
    /// Context management for automatic clearing of old content
    pub context_management: Option<ContextManagement>,
    /// Web search configuration (server-executed)
    pub web_search: Option<WebSearchConfig>,
    /// Web fetch configuration (server-executed, beta)
    pub web_fetch: Option<WebFetchConfig>,
}

impl Default for CallOptions {
    fn default() -> Self {
        Self {
            max_tokens: None,
            temperature: None,
            tools: None,
            system_prompt: None,
            thinking: None,
            reasoning_format: None,
            enable_caching: true,
            context_management: None,
            web_search: None,
            web_fetch: None,
        }
    }
}

/// Anthropic API client
pub struct AnthropicClient {
    http: Client,
    config: AnthropicConfig,
    token_manager: Option<Arc<TokenManager>>,
    api_key: Option<String>,
}

impl AnthropicClient {
    /// Create the HTTP client with configuration optimized for SSE streaming
    fn create_http_client() -> Client {
        Client::builder()
            .user_agent("Krusty/1.0")
            .connect_timeout(constants::http::CONNECT_TIMEOUT)
            // Long timeout for streaming - extended thinking + large tool outputs can take 5+ minutes
            .timeout(constants::http::STREAM_TIMEOUT)
            .build()
            .unwrap_or_else(|e| {
                error!("Failed to build HTTP client: {}. Using default client.", e);
                Client::new()
            })
    }

    /// Create a new client with OAuth token manager
    pub fn with_token_manager(config: AnthropicConfig, token_manager: Arc<TokenManager>) -> Self {
        Self {
            http: Self::create_http_client(),
            config,
            token_manager: Some(token_manager),
            api_key: None,
        }
    }

    /// Create a new client with API key
    pub fn with_api_key(config: AnthropicConfig, api_key: String) -> Self {
        Self {
            http: Self::create_http_client(),
            config,
            token_manager: None,
            api_key: Some(api_key),
        }
    }

    /// Get the current auth token
    async fn get_auth_token(&self) -> Result<String> {
        if let Some(ref token_manager) = self.token_manager {
            token_manager.get_valid_token().await
        } else if let Some(ref api_key) = self.api_key {
            Ok(api_key.clone())
        } else {
            Err(anyhow::anyhow!("No authentication configured"))
        }
    }

    /// Check if a token is an OAuth token
    fn is_oauth_token(token: &str) -> bool {
        token.starts_with(OAUTH_TOKEN_PREFIX)
    }

    /// Get the provider ID for this client
    pub fn provider_id(&self) -> ProviderId {
        self.config.provider_id()
    }

    /// Convert domain messages to Anthropic format
    ///
    /// CRITICAL: This function ensures proper message alternation required by the API.
    /// The API requires user/assistant messages to strictly alternate. If there are
    /// consecutive user messages (e.g., tool_result followed by user text without
    /// assistant response between), we must insert an empty assistant message.
    ///
    /// THINKING BLOCKS: According to Anthropic docs, thinking blocks are only required
    /// when using tools with extended thinking. For the last assistant message with
    /// pending tool calls (when we're about to send tool_results), we preserve thinking.
    /// All other thinking blocks are stripped to avoid "Invalid signature" errors.
    fn convert_messages(&self, messages: &[ModelMessage]) -> Vec<Value> {
        let mut result: Vec<Value> = Vec::new();
        let mut last_role: Option<&str> = None;

        info!("Converting {} messages for API", messages.len());

        // Determine which assistant message (if any) should keep thinking blocks.
        // This is the last assistant message that has tool_use AND is followed by tool_result.
        let non_system_messages: Vec<_> =
            messages.iter().filter(|m| m.role != Role::System).collect();

        let last_assistant_with_tools_idx = {
            // Find the last assistant message with tool_use that is followed by a tool message
            let mut idx = None;
            for (i, msg) in non_system_messages.iter().enumerate() {
                if msg.role == Role::Assistant
                    && msg
                        .content
                        .iter()
                        .any(|c| matches!(c, Content::ToolUse { .. }))
                {
                    // Check if followed by tool result
                    if i + 1 < non_system_messages.len()
                        && (non_system_messages[i + 1].role == Role::Tool
                            || non_system_messages[i + 1]
                                .content
                                .iter()
                                .any(|c| matches!(c, Content::ToolResult { .. })))
                    {
                        idx = Some(i);
                    }
                }
            }
            idx
        };

        for (i, msg) in non_system_messages.iter().enumerate() {
            // Log each message structure
            let content_summary: Vec<String> = msg
                .content
                .iter()
                .map(|c| match c {
                    Content::Text { text } => format!("Text({})", text.len()),
                    Content::ToolUse { id, name, .. } => format!("ToolUse({}, {})", name, id),
                    Content::ToolResult { tool_use_id, .. } => {
                        format!("ToolResult({})", tool_use_id)
                    }
                    Content::Image { .. } => "Image".to_string(),
                    Content::Document { .. } => "Document".to_string(),
                    Content::Thinking { thinking, .. } => format!("Thinking({})", thinking.len()),
                    Content::RedactedThinking { .. } => "RedactedThinking".to_string(),
                })
                .collect();
            debug!("  msg[{}] {:?}: {:?}", i, msg.role, content_summary);
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "user", // Tool results come as user messages
                Role::System => unreachable!(),
            };

            // Check for consecutive same-role messages
            // API requires strict user/assistant alternation
            if let Some(prev_role) = last_role {
                if prev_role == role {
                    // Insert minimal message of opposite role to maintain alternation
                    // Note: API requires non-whitespace text content
                    let filler_role = if role == "user" { "assistant" } else { "user" };
                    debug!(
                        "Inserting filler {} message to maintain alternation",
                        filler_role
                    );
                    result.push(serde_json::json!({
                        "role": filler_role,
                        "content": [{
                            "type": "text",
                            "text": "."
                        }]
                    }));
                }
            }

            // Determine if this message should include thinking blocks
            let include_thinking = last_assistant_with_tools_idx == Some(i);

            let content: Vec<Value> = msg
                .content
                .iter()
                .filter_map(|c| match c {
                    Content::Text { text } => Some(serde_json::json!({
                        "type": "text",
                        "text": text
                    })),
                    Content::ToolUse { id, name, input } => Some(serde_json::json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input
                    })),
                    Content::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                    } => Some(serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": output,
                        "is_error": is_error.unwrap_or(false)
                    })),
                    Content::Image { image, detail: _ } => {
                        if let Some(base64_data) = &image.base64 {
                            Some(serde_json::json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": image.media_type.clone().unwrap_or_else(|| "image/png".to_string()),
                                    "data": base64_data
                                }
                            }))
                        } else if let Some(url) = &image.url {
                            Some(serde_json::json!({
                                "type": "image",
                                "source": {
                                    "type": "url",
                                    "url": url
                                }
                            }))
                        } else {
                            Some(serde_json::json!({
                                "type": "text",
                                "text": "[Invalid image content]"
                            }))
                        }
                    }
                    Content::Document { source } => {
                        if let Some(data) = &source.data {
                            Some(serde_json::json!({
                                "type": "document",
                                "source": {
                                    "type": "base64",
                                    "media_type": source.media_type,
                                    "data": data
                                }
                            }))
                        } else if let Some(url) = &source.url {
                            Some(serde_json::json!({
                                "type": "document",
                                "source": {
                                    "type": "url",
                                    "url": url
                                }
                            }))
                        } else {
                            Some(serde_json::json!({
                                "type": "text",
                                "text": "[Invalid document content]"
                            }))
                        }
                    }
                    // Only include thinking blocks for the last assistant message with pending tools
                    Content::Thinking { thinking, signature } => {
                        if include_thinking {
                            Some(serde_json::json!({
                                "type": "thinking",
                                "thinking": thinking,
                                "signature": signature
                            }))
                        } else {
                            None // Strip thinking from other messages
                        }
                    }
                    Content::RedactedThinking { data } => {
                        if include_thinking {
                            Some(serde_json::json!({
                                "type": "redacted_thinking",
                                "data": data
                            }))
                        } else {
                            None // Strip redacted thinking from other messages
                        }
                    }
                })
                .collect();

            result.push(serde_json::json!({
                "role": role,
                "content": content
            }));

            last_role = Some(role);
        }

        result
    }

    /// Convert domain messages to OpenAI chat/completions format
    ///
    /// OpenAI format is simpler: role + content (string or array of content parts)
    fn convert_messages_openai(&self, messages: &[ModelMessage]) -> Vec<Value> {
        let mut result: Vec<Value> = Vec::new();

        for msg in messages.iter().filter(|m| m.role != Role::System) {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
                Role::System => continue,
            };

            // For tool results, use special format
            if msg.role == Role::Tool {
                for content in &msg.content {
                    if let Content::ToolResult {
                        tool_use_id,
                        output,
                        ..
                    } = content
                    {
                        result.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": output
                        }));
                    }
                }
                continue;
            }

            // For assistant messages with tool calls
            let has_tool_use = msg
                .content
                .iter()
                .any(|c| matches!(c, Content::ToolUse { .. }));
            if has_tool_use && role == "assistant" {
                let mut tool_calls = Vec::new();
                let mut text_content = String::new();

                for content in &msg.content {
                    match content {
                        Content::Text { text } => text_content.push_str(text),
                        Content::ToolUse { id, name, input } => {
                            tool_calls.push(serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": input.to_string()
                                }
                            }));
                        }
                        _ => {}
                    }
                }

                let mut msg_obj = serde_json::json!({
                    "role": "assistant",
                    "tool_calls": tool_calls
                });
                if !text_content.is_empty() {
                    msg_obj["content"] = serde_json::json!(text_content);
                }
                result.push(msg_obj);
                continue;
            }

            // Regular messages - extract text content
            let text: String = msg
                .content
                .iter()
                .filter_map(|c| match c {
                    Content::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            if !text.is_empty() {
                result.push(serde_json::json!({
                    "role": role,
                    "content": text
                }));
            }
        }

        result
    }

    /// Convert tools to OpenAI format
    fn convert_tools_openai(&self, tools: &[AiTool]) -> Vec<Value> {
        let use_responses_format = matches!(self.config.api_format, ApiFormat::OpenAIResponses);

        tools
            .iter()
            .map(|tool| {
                if use_responses_format {
                    // Responses API: flat structure with name at top level
                    serde_json::json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema
                    })
                } else {
                    // Chat Completions: nested under "function"
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.input_schema
                        }
                    })
                }
            })
            .collect()
    }

    /// Make a simple non-streaming API call
    ///
    /// Used for quick tasks like title generation where streaming is overkill.
    /// Returns the text content directly.
    pub async fn call_simple(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        max_tokens: usize,
    ) -> Result<String> {
        let token = self.get_auth_token().await?;
        let is_oauth = Self::is_oauth_token(&token);

        // For OAuth, we must use Claude Code system prompt
        // Put our instructions in the user message instead
        let (system, message) = if is_oauth {
            let combined_message = format!(
                "<instructions>\n{}\n</instructions>\n\n{}",
                system_prompt, user_message
            );
            (
                "You are Claude Code, Anthropic's official CLI for Claude.".to_string(),
                combined_message,
            )
        } else {
            (system_prompt.to_string(), user_message.to_string())
        };

        let body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [{
                "role": "user",
                "content": message
            }],
            "system": system
        });

        let mut request = self
            .http
            .post(self.config.api_url())
            .header("content-type", "application/json");

        // Add auth header based on provider config
        match self.config.auth_header {
            AuthHeader::Bearer => {
                request = request.header("authorization", format!("Bearer {}", token));
            }
            AuthHeader::XApiKey => {
                if is_oauth {
                    request = request.header("authorization", format!("Bearer {}", token));
                } else {
                    request = request.header("x-api-key", &token);
                }
            }
        }

        // Add Anthropic API headers (all providers use Anthropic-compatible API)
        if self.config.uses_anthropic_api() {
            request = request.header("anthropic-version", API_VERSION);
            // OAuth beta header only for native Anthropic with OAuth token
            if self.config.is_anthropic() && is_oauth {
                request = request.header("anthropic-beta", "oauth-2025-04-20");
            }
        }

        let response = request.json(&body).send().await?;
        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("API error: {} - {}", status, error_text));
        }

        let json: Value = response.json().await?;

        // Extract text from response
        let text = json
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|block| block.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(text)
    }

    /// Call the API with extended thinking enabled (non-streaming)
    ///
    /// Used for complex summarization tasks where we want deep analysis.
    /// Returns the text content after thinking completes.
    ///
    /// Note: For OAuth tokens, system_prompt is embedded in the user message
    /// since OAuth requires the Claude Code system prompt.
    pub async fn call_with_thinking(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
        thinking_budget: u32,
    ) -> Result<String> {
        let token = self.get_auth_token().await?;
        let is_oauth = Self::is_oauth_token(&token);

        // For thinking, max_tokens must be > budget_tokens
        let max_tokens = thinking_budget + 16000;

        // For OAuth, we must use Claude Code system prompt
        // Put our instructions in the user message instead
        let (system, message) = if is_oauth {
            let combined_message = format!(
                "<instructions>\n{}\n</instructions>\n\n{}",
                system_prompt, user_message
            );
            (
                "You are Claude Code, Anthropic's official CLI for Claude.".to_string(),
                combined_message,
            )
        } else {
            (system_prompt.to_string(), user_message.to_string())
        };

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [{
                "role": "user",
                "content": message
            }],
            "system": system,
            "thinking": {
                "type": "enabled",
                "budget_tokens": thinking_budget
            }
        });

        // Effort parameter is ONLY supported by Opus 4.5
        if model.contains("opus-4-5") {
            body["output_config"] = serde_json::json!({
                "effort": "high"
            });
        }

        let mut request = self
            .http
            .post(self.config.api_url())
            .header("content-type", "application/json");

        // Add auth header based on provider config
        match self.config.auth_header {
            AuthHeader::Bearer => {
                request = request.header("authorization", format!("Bearer {}", token));
            }
            AuthHeader::XApiKey => {
                if is_oauth {
                    request = request.header("authorization", format!("Bearer {}", token));
                } else {
                    request = request.header("x-api-key", &token);
                }
            }
        }

        // Add Anthropic API headers for thinking (all providers support this)
        if self.config.uses_anthropic_api() {
            request = request.header("anthropic-version", API_VERSION);

            // Build beta headers
            let mut beta_parts = vec!["interleaved-thinking-2025-05-14"];
            // OAuth beta only for native Anthropic with OAuth token
            if self.config.is_anthropic() && is_oauth {
                beta_parts.insert(0, "oauth-2025-04-20");
            }
            if model.contains("opus-4-5") {
                beta_parts.push("effort-2025-11-24");
            }
            request = request.header("anthropic-beta", beta_parts.join(","));
        }

        info!(
            "Calling API with extended thinking (budget: {})",
            thinking_budget
        );
        let response = request.json(&body).send().await?;
        let status = response.status();

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("API error: {} - {}", status, error_text));
        }

        let json: Value = response.json().await?;

        // Extract text from response (skip thinking blocks, get text blocks)
        let mut text_content = String::new();
        if let Some(content) = json.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if let Some(block_type) = block.get("type").and_then(|t| t.as_str()) {
                    if block_type == "text" {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            text_content.push_str(text);
                        }
                    }
                }
            }
        }

        Ok(text_content.trim().to_string())
    }

    /// Call the API with tools (non-streaming, for sub-agents)
    ///
    /// Used by sub-agents that need tool execution but don't need streaming.
    /// Handles OAuth tokens correctly by using Claude Code system prompt.
    pub async fn call_with_tools(
        &self,
        model: &str,
        system_prompt: &str,
        messages: Vec<Value>,
        tools: Vec<Value>,
        max_tokens: usize,
    ) -> Result<Value> {
        let token = self.get_auth_token().await?;
        let is_oauth = Self::is_oauth_token(&token);

        // For OAuth, we must use Claude Code system prompt
        // Inject our instructions into the first user message
        let (system, final_messages) = if is_oauth {
            let mut modified_messages = messages.clone();

            // Find first user message and prepend instructions
            if let Some(first_msg) = modified_messages.first_mut() {
                if first_msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                    if let Some(content) = first_msg.get("content") {
                        let instructions = format!(
                            "<sub-agent-instructions>\n{}\n</sub-agent-instructions>\n\n",
                            system_prompt
                        );

                        // Handle both string and array content
                        if let Some(text) = content.as_str() {
                            first_msg["content"] = serde_json::json!([{
                                "type": "text",
                                "text": format!("{}{}", instructions, text)
                            }]);
                        } else if let Some(arr) = content.as_array() {
                            let mut new_content = vec![serde_json::json!({
                                "type": "text",
                                "text": instructions
                            })];
                            new_content.extend(arr.clone());
                            first_msg["content"] = serde_json::json!(new_content);
                        }
                    }
                }
            }

            (
                // CRITICAL: OAuth requires EXACTLY this system prompt - no modifications!
                "You are Claude Code, Anthropic's official CLI for Claude.".to_string(),
                modified_messages,
            )
        } else {
            (system_prompt.to_string(), messages)
        };

        let body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": final_messages,
            "system": system,
            "tools": tools
        });

        let mut request = self
            .http
            .post(self.config.api_url())
            .header("content-type", "application/json");

        // Add auth header based on provider config
        match self.config.auth_header {
            AuthHeader::Bearer => {
                request = request.header("authorization", format!("Bearer {}", token));
            }
            AuthHeader::XApiKey => {
                if is_oauth {
                    request = request.header("authorization", format!("Bearer {}", token));
                } else {
                    request = request.header("x-api-key", &token);
                }
            }
        }

        // Add Anthropic API headers (all providers use Anthropic-compatible API)
        if self.config.uses_anthropic_api() {
            request = request.header("anthropic-version", API_VERSION);

            // Build beta headers
            let mut beta_headers: Vec<&str> = Vec::new();
            // OAuth beta only for native Anthropic with OAuth token
            if self.config.is_anthropic() && is_oauth {
                beta_headers.push("oauth-2025-04-20");
            }
            // Thinking beta for all providers that support it
            beta_headers.push("interleaved-thinking-2025-05-14");

            if !beta_headers.is_empty() {
                request = request.header("anthropic-beta", beta_headers.join(","));
            }
        }

        info!(model = model, is_oauth = is_oauth, provider = %self.config.provider_id, "Sub-agent API call starting");
        let start = std::time::Instant::now();

        let response = match request.json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent API request failed");
                return Err(anyhow::anyhow!("API request failed: {}", e));
            }
        };

        let status = response.status();
        info!(status = %status, elapsed_ms = start.elapsed().as_millis() as u64, "Sub-agent API response received");

        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!(status = %status, error = %error_text, "Sub-agent API error response");
            return Err(anyhow::anyhow!("API error: {} - {}", status, error_text));
        }

        let json: Value = response.json().await?;
        info!(
            elapsed_ms = start.elapsed().as_millis() as u64,
            "Sub-agent API call complete"
        );
        Ok(json)
    }

    /// Call the API with streaming response
    pub async fn call_streaming(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        let call_start = Instant::now();
        info!("=== API CALL START ===");
        info!(
            "Model: {}, Messages: {}, Tools: {}, Thinking: {}, Format: {:?}",
            self.config.model,
            messages.len(),
            options.tools.as_ref().map(|t| t.len()).unwrap_or(0),
            options.thinking.is_some(),
            self.config.api_format
        );

        // Route to OpenAI format handler for non-Anthropic models
        if self.config.uses_openai_format() {
            return self
                .call_streaming_openai(messages, options, call_start)
                .await;
        }

        let token = self.get_auth_token().await?;
        let anthropic_messages = self.convert_messages(&messages);

        // Extract any system messages from conversation (e.g., pinch context)
        // These are filtered out of messages but need to be in the system prompt
        let injected_context: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .filter_map(|m| {
                m.content.iter().find_map(|c| match c {
                    Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        // Determine if using OAuth - this affects system prompt handling
        let is_oauth = Self::is_oauth_token(&token);

        // Build system prompt
        // CRITICAL: OAuth MUST use exact Claude Code prompt - cannot be modified
        let mut system = if is_oauth {
            // OAuth requires the exact Claude Code system prompt
            "You are Claude Code, Anthropic's official CLI for Claude.".to_string()
        } else if let Some(custom) = &options.system_prompt {
            custom.clone()
        } else {
            // Default for API keys - full Krusty philosophy
            KRUSTY_SYSTEM_PROMPT.to_string()
        };

        // Handle injected context (pinch context, etc.)
        // For OAuth: inject into first user message (system prompt must stay unchanged)
        // For API key: append to system prompt
        let mut anthropic_messages = anthropic_messages; // Make mutable
        if !injected_context.is_empty() {
            if is_oauth {
                // For OAuth, prepend to first user message
                if let Some(first_msg) = anthropic_messages.first_mut() {
                    if first_msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                        let context_block = serde_json::json!({
                            "type": "text",
                            "text": format!("<pinch-context>\n{}\n</pinch-context>\n\n", injected_context)
                        });

                        if let Some(content) = first_msg.get_mut("content") {
                            if let Some(arr) = content.as_array_mut() {
                                // Insert at beginning of content array
                                arr.insert(0, context_block);
                            }
                        }
                        info!(
                            "OAuth: Injected {} chars of pinch context into first user message",
                            injected_context.len()
                        );
                    }
                }
            } else {
                // For API key, append to system prompt
                system.push_str("\n\n---\n\n");
                system.push_str(&injected_context);
                info!(
                    "Injected {} chars of context into system prompt",
                    injected_context.len()
                );
            }
        }

        // For OAuth: inject Krusty philosophy into first user message
        // (since we can't modify the Claude Code system prompt)
        if is_oauth {
            if let Some(first_msg) = anthropic_messages.first_mut() {
                if first_msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                    let philosophy_block = serde_json::json!({
                        "type": "text",
                        "text": format!("<krusty-philosophy>\n{}\n</krusty-philosophy>\n\n", KRUSTY_SYSTEM_PROMPT)
                    });

                    if let Some(content) = first_msg.get_mut("content") {
                        if let Some(arr) = content.as_array_mut() {
                            // Insert at very beginning (before pinch-context if present)
                            arr.insert(0, philosophy_block);
                        }
                    }
                    debug!("OAuth: Injected Krusty philosophy into first user message");
                }
            }
        }

        // Determine max_tokens based on reasoning format
        // Anthropic thinking requires max_tokens > budget_tokens
        let max_tokens = match options.reasoning_format {
            Some(ReasoningFormat::Anthropic) => {
                // Anthropic: use 64k to allow room for 32k thinking budget
                64000
            }
            Some(ReasoningFormat::OpenAI | ReasoningFormat::DeepSeek) => {
                // OpenAI/DeepSeek: reasoning doesn't reduce output quota
                options.max_tokens.unwrap_or(self.config.max_tokens)
            }
            None => {
                // Legacy support: if thinking is set without format, assume Anthropic
                if options.thinking.is_some() {
                    64000
                } else {
                    options.max_tokens.unwrap_or(self.config.max_tokens)
                }
            }
        };

        // Build request body
        let mut body = serde_json::json!({
            "model": self.config.model,
            "messages": anthropic_messages,
            "max_tokens": max_tokens,
            "stream": true,
        });

        if !system.is_empty() {
            // Use array format for system prompt with cache control
            if options.enable_caching {
                body["system"] = serde_json::json!([{
                    "type": "text",
                    "text": system,
                    "cache_control": {"type": "ephemeral"}
                }]);
                debug!("Cache breakpoint added to system prompt");
            } else {
                body["system"] = Value::String(system);
            }
        }

        // Temperature incompatible with reasoning - only add if reasoning is off
        let reasoning_enabled = options.reasoning_format.is_some() || options.thinking.is_some();
        if !reasoning_enabled {
            if let Some(temp) = options.temperature {
                body["temperature"] = serde_json::json!(temp);
            }
        }

        // Build tools array (client tools + server tools)
        let mut all_tools: Vec<Value> = Vec::new();

        // Add client-side tools (bash, edit, read, etc.)
        if let Some(tools) = &options.tools {
            for tool in tools {
                all_tools.push(serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                }));
            }
        }

        // Add server-executed tools based on provider capabilities
        let capabilities = ProviderCapabilities::for_provider(self.config.provider_id);

        // Anthropic server-executed web tools
        if capabilities.web_search {
            if let Some(search) = &options.web_search {
                let mut spec = serde_json::json!({
                    "type": "web_search_20250305",
                    "name": "web_search",
                });
                if let Some(max_uses) = search.max_uses {
                    spec["max_uses"] = serde_json::json!(max_uses);
                }
                all_tools.push(spec);
                debug!("Web search tool enabled (server-side)");
            }
        }

        if capabilities.web_fetch {
            if let Some(fetch) = &options.web_fetch {
                let mut spec = serde_json::json!({
                    "type": "web_fetch_20250910",
                    "name": "web_fetch",
                    "citations": { "enabled": fetch.citations_enabled },
                });
                if let Some(max_uses) = fetch.max_uses {
                    spec["max_uses"] = serde_json::json!(max_uses);
                }
                if let Some(max_tokens) = fetch.max_content_tokens {
                    spec["max_content_tokens"] = serde_json::json!(max_tokens);
                }
                all_tools.push(spec);
                debug!("Web fetch tool enabled (server-side)");
            }
        }

        // OpenRouter web search: append :online suffix to model name
        // This works with any endpoint - the suffix tells OpenRouter to enable web search
        if capabilities.web_plugins && options.web_search.is_some() {
            if let Some(model) = body.get("model").and_then(|m| m.as_str()) {
                if !model.ends_with(":online") {
                    let online_model = format!("{}:online", model);
                    body["model"] = serde_json::json!(online_model);
                    info!(
                        "OpenRouter web search enabled via model suffix: {}",
                        online_model
                    );
                }
            }
        }

        // Log if web tools requested but not supported
        if !capabilities.web_search
            && !capabilities.web_plugins
            && (options.web_search.is_some() || options.web_fetch.is_some())
        {
            debug!(
                "Web tools not supported for provider {:?}",
                self.config.provider_id
            );
        }

        // Add all tools to body with cache breakpoint on last one
        if !all_tools.is_empty() {
            let tool_count = all_tools.len();
            if options.enable_caching && tool_count > 0 {
                // Add cache control to last tool
                if let Some(last) = all_tools.last_mut() {
                    last["cache_control"] = serde_json::json!({"type": "ephemeral"});
                }
                debug!("Cache breakpoint added to last tool");
            }
            // Debug: log tool schemas for provider compatibility debugging
            for tool in &all_tools {
                if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                    debug!(
                        "Tool '{}' schema: {}",
                        name,
                        tool.get("input_schema")
                            .map(|s| serde_json::to_string(s).unwrap_or_default())
                            .unwrap_or_else(|| "MISSING".to_string())
                    );
                }
            }
            body["tools"] = Value::Array(all_tools);
        }

        // Add reasoning/thinking config based on format
        // Universal reasoning: all formats use MAX effort (no half-measures)
        match options.reasoning_format {
            Some(ReasoningFormat::Anthropic) => {
                // Anthropic Claude: thinking.budget_tokens (max: 32000)
                let budget = options
                    .thinking
                    .as_ref()
                    .map(|t| t.budget_tokens)
                    .unwrap_or(32000);
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": budget
                });
                debug!("Anthropic thinking enabled with budget: {}", budget);

                // Opus 4.5 supports effort parameter - always use high
                if self.config.model.contains("opus-4-5") {
                    body["output_config"] = serde_json::json!({
                        "effort": "high"
                    });
                    debug!("Using high effort for Opus 4.5");
                }
            }
            Some(ReasoningFormat::OpenAI) => {
                // OpenAI o1/o3/GPT-5: reasoning_effort = "high" (max effort)
                body["reasoning_effort"] = serde_json::json!("high");
                debug!("OpenAI reasoning enabled with high effort");
            }
            Some(ReasoningFormat::DeepSeek) => {
                // DeepSeek R1: reasoning.enabled = true
                body["reasoning"] = serde_json::json!({
                    "enabled": true
                });
                debug!("DeepSeek reasoning enabled");
            }
            None => {
                // Legacy support: if thinking is set without format, assume Anthropic
                if let Some(thinking) = &options.thinking {
                    body["thinking"] = serde_json::json!({
                        "type": "enabled",
                        "budget_tokens": thinking.budget_tokens
                    });
                    debug!(
                        "Legacy thinking enabled with budget: {}",
                        thinking.budget_tokens
                    );

                    if self.config.model.contains("opus-4-5") {
                        body["output_config"] = serde_json::json!({
                            "effort": "high"
                        });
                    }
                }
            }
        }

        // Add context management if enabled
        if let Some(ctx_mgmt) = &options.context_management {
            let caps = ProviderCapabilities::for_provider(self.config.provider_id);
            if caps.context_management {
                body["context_management"] =
                    serde_json::to_value(ctx_mgmt).unwrap_or(serde_json::Value::Null);
                info!("Context management enabled: {} edits", ctx_mgmt.edits.len());
            } else {
                debug!(
                    "Skipping context_management for provider {:?} (not supported)",
                    self.config.provider_id
                );
            }
        }

        // Add provider-specific parameters based on model
        // Pass thinking status to enable/disable chat_template_args for GLM/Kimi models
        let thinking_enabled = options.reasoning_format.is_some() || options.thinking.is_some();
        let provider_params =
            build_provider_params(&self.config.model, self.config.provider_id, thinking_enabled);

        // Temperature incompatible with reasoning - skip provider temperature if thinking enabled
        if !thinking_enabled {
            if let Some(temp) = provider_params.temperature {
                body["temperature"] = Value::Number(serde_json::Number::from(temp as i32));
                debug!(
                    "Setting temperature: {} for model {}",
                    temp, self.config.model
                );
            }
        }

        if let Some(top_p) = provider_params.top_p {
            body["top_p"] = Value::Number(serde_json::Number::from_f64(top_p as f64).unwrap());
            debug!("Setting top_p: {} for model {}", top_p, self.config.model);
        }

        if let Some(top_k) = provider_params.top_k {
            body["top_k"] = Value::Number(serde_json::Number::from(top_k));
            debug!("Setting top_k: {} for model {}", top_k, self.config.model);
        }

        if let Some(chat_args) = provider_params.chat_template_args {
            body["chat_template_args"] = chat_args;
            info!(
                "Enabling chat_template_args for thinking model {}",
                self.config.model
            );
        }

        debug!("Calling {} API with streaming", self.config.provider_id);

        // Build request with proper headers based on provider and token type
        let mut request = self.http.post(self.config.api_url());

        // Add auth header based on provider config and token type
        match self.config.auth_header {
            AuthHeader::Bearer => {
                // OpenRouter and similar use Bearer auth
                request = request.header("authorization", format!("Bearer {}", token));
                info!(
                    "Using Bearer authentication for {}",
                    self.config.provider_id
                );
            }
            AuthHeader::XApiKey => {
                // Anthropic-style: check if OAuth or API key
                if is_oauth {
                    request = request.header("authorization", format!("Bearer {}", token));
                    info!("Using OAuth authentication");
                } else {
                    request = request.header("x-api-key", &token);
                    info!("Using API key authentication");
                }
            }
        }

        // Add Anthropic API headers (all providers use Anthropic-compatible API)
        if self.config.uses_anthropic_api() {
            // Collect beta headers
            let mut beta_headers: Vec<&str> = Vec::new();

            // OAuth beta header - only for native Anthropic with OAuth token
            if self.config.is_anthropic() && is_oauth {
                beta_headers.push("oauth-2025-04-20");
            }

            // Add thinking beta headers for Anthropic reasoning format
            let anthropic_thinking =
                matches!(options.reasoning_format, Some(ReasoningFormat::Anthropic))
                    || options.thinking.is_some();
            if anthropic_thinking {
                beta_headers.push("interleaved-thinking-2025-05-14");

                // Effort beta for Opus 4.5
                if self.config.model.contains("opus-4-5") {
                    beta_headers.push("effort-2025-11-24");
                }
            }

            // Context management beta
            if options.context_management.is_some() {
                beta_headers.push("context-management-2025-06-27");
            }

            // Web tool beta headers (only for providers that support server-side web tools)
            let caps = ProviderCapabilities::for_provider(self.config.provider_id);
            if options.web_search.is_some() && caps.web_search {
                beta_headers.push("web-search-2025-03-05");
            }
            if options.web_fetch.is_some() && caps.web_fetch {
                beta_headers.push("web-fetch-2025-09-10");
            }

            // Add all beta headers as comma-separated
            if !beta_headers.is_empty() {
                let beta_str = beta_headers.join(",");
                debug!("Beta headers: {}", beta_str);
                request = request.header("anthropic-beta", beta_str);
            }

            // Anthropic version header
            request = request.header("anthropic-version", API_VERSION);
        }

        // Common headers
        request = request.header("content-type", "application/json");

        // Send request
        info!("Sending API request...");
        let response = request.json(&body).send().await?;
        let request_duration = call_start.elapsed();

        let status = response.status();
        info!("API response: {} in {:?}", status, request_duration);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("API error response: {} - {}", status, error_text);
            return Err(anyhow::anyhow!("API error: {} - {}", status, error_text));
        }

        // Set up streaming channels
        let (tx, rx, buffer_tx, buffer_rx) = create_streaming_channels();
        spawn_buffer_processor(buffer_rx, tx.clone());

        let mut processor = SseStreamProcessor::new(tx, buffer_tx);
        let parser = AnthropicParser::new();

        // Spawn task to process the stream
        info!("Starting stream processing task");
        let stream = response.bytes_stream();
        tokio::spawn(async move {
            tokio::pin!(stream);
            let mut chunk_count = 0;
            while let Some(chunk) = stream.next().await {
                chunk_count += 1;
                match chunk {
                    Ok(bytes) => {
                        if let Err(e) = processor.process_chunk(bytes, &parser).await {
                            warn!("Error processing chunk #{}: {}", chunk_count, e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Stream read error at chunk #{}: {}", chunk_count, e);
                        break;
                    }
                }
            }
            info!("Stream ended after {} chunks", chunk_count);
            processor.finish().await;
        });

        Ok(rx)
    }

    /// Call the API with streaming response using OpenAI chat/completions format
    ///
    /// Used for OpenCode Zen models that need OpenAI-style API (GLM, Kimi, Qwen, etc.)
    async fn call_streaming_openai(
        &self,
        messages: Vec<ModelMessage>,
        options: &CallOptions,
        call_start: Instant,
    ) -> Result<mpsc::UnboundedReceiver<StreamPart>> {
        info!(
            "Using OpenAI chat/completions format for {}",
            self.config.model
        );

        let token = self.get_auth_token().await?;
        let openai_messages = self.convert_messages_openai(&messages);

        // Extract system prompt from messages
        let system: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .filter_map(|m| {
                m.content.iter().find_map(|c| match c {
                    Content::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        // Build base system message if present
        let system_prompt = if !system.is_empty() {
            Some(format!("{}\n\n{}", KRUSTY_SYSTEM_PROMPT, system))
        } else if options.system_prompt.is_some() {
            options.system_prompt.clone()
        } else {
            Some(KRUSTY_SYSTEM_PROMPT.to_string())
        };

        let max_tokens = options.max_tokens.unwrap_or(self.config.max_tokens);

        // Responses API uses "input", Chat Completions uses "messages"
        let (messages_key, max_tokens_key) =
            if matches!(self.config.api_format, ApiFormat::OpenAIResponses) {
                ("input", "max_output_tokens")
            } else {
                ("messages", "max_tokens")
            };

        // Build request body
        let mut body = serde_json::json!({
            "model": self.config.model,
            "stream": true,
        });
        body[max_tokens_key] = serde_json::json!(max_tokens);
        body[messages_key] = serde_json::json!(openai_messages);

        // Add system message at the start if present
        if let Some(sys) = system_prompt {
            if let Some(msgs) = body.get_mut(messages_key).and_then(|m| m.as_array_mut()) {
                msgs.insert(
                    0,
                    serde_json::json!({
                        "role": "system",
                        "content": sys
                    }),
                );
            }
        }

        // Add temperature if specified (and not using reasoning)
        if options.thinking.is_none() {
            if let Some(temp) = options.temperature {
                body["temperature"] = serde_json::json!(temp);
            }
        }

        // Add tools in OpenAI format
        if let Some(tools) = &options.tools {
            let openai_tools = self.convert_tools_openai(tools);
            if !openai_tools.is_empty() {
                body["tools"] = serde_json::json!(openai_tools);
            }
        }

        debug!("OpenAI request to: {}", self.config.api_url());

        // Build request with appropriate headers for OpenAI format
        let mut request = self.http.post(self.config.api_url());

        // Add auth header (OpenCode Zen uses x-api-key)
        request = request.header("x-api-key", &token);
        request = request.header("content-type", "application/json");

        // Send request
        info!("Sending OpenAI format request...");
        let response = request.json(&body).send().await?;
        let request_duration = call_start.elapsed();

        let status = response.status();
        info!("API response: {} in {:?}", status, request_duration);

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            error!("API error response: {} - {}", status, error_text);
            return Err(anyhow::anyhow!("API error: {} - {}", status, error_text));
        }

        // Set up streaming channels
        let (tx, rx, buffer_tx, buffer_rx) = create_streaming_channels();
        spawn_buffer_processor(buffer_rx, tx.clone());

        let mut processor = SseStreamProcessor::new(tx, buffer_tx);
        let parser = OpenAIParser::new();

        // Spawn task to process the stream
        info!("Starting OpenAI stream processing task");
        let stream = response.bytes_stream();
        tokio::spawn(async move {
            tokio::pin!(stream);
            let mut chunk_count = 0;
            while let Some(chunk) = stream.next().await {
                chunk_count += 1;
                match chunk {
                    Ok(bytes) => {
                        if let Err(e) = processor.process_chunk(bytes, &parser).await {
                            warn!("Error processing OpenAI chunk #{}: {}", chunk_count, e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("OpenAI stream read error at chunk #{}: {}", chunk_count, e);
                        break;
                    }
                }
            }
            info!("OpenAI stream ended after {} chunks", chunk_count);
            processor.finish().await;
        });

        Ok(rx)
    }
}

/// OpenAI-compatible SSE parser for chat/completions format
pub struct OpenAIParser {
    /// Track tool calls being accumulated
    tool_accumulators: std::sync::Mutex<std::collections::HashMap<usize, ToolCallAccumulator>>,
}

impl OpenAIParser {
    pub fn new() -> Self {
        Self {
            tool_accumulators: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Parse OpenAI Responses API event format
    /// Used by GPT-5 models via OpenCode Zen
    fn parse_responses_api_event(&self, json: &Value, event_type: &str) -> SseEvent {
        match event_type {
            // Text content delta
            "response.output_text.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    if !delta.is_empty() {
                        return SseEvent::TextDelta(delta.to_string());
                    }
                }
            }

            // Response completed
            "response.done" | "response.completed" => {
                return SseEvent::Finish {
                    reason: FinishReason::Stop,
                };
            }

            // Function/tool call start
            "response.function_call_arguments.start" | "response.output_item.added" => {
                if let Some(item) = json.get("item") {
                    if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                        let id = item
                            .get("call_id")
                            .and_then(|i| i.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !name.is_empty() {
                            let mut accumulators = self.tool_accumulators.lock().unwrap();
                            let index = accumulators.len();
                            accumulators
                                .insert(index, ToolCallAccumulator::new(id.clone(), name.clone()));
                            return SseEvent::ToolCallStart { id, name };
                        }
                    }
                }
            }

            // Function arguments delta
            "response.function_call_arguments.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    let accumulators = self.tool_accumulators.lock().unwrap();
                    if let Some((_, acc)) = accumulators.iter().last() {
                        return SseEvent::ToolCallDelta {
                            id: acc.id.clone(),
                            delta: delta.to_string(),
                        };
                    }
                }
            }

            // Function call done
            "response.function_call_arguments.done" => {
                // Tool call complete, will be handled by finish event
            }

            // Usage info
            "response.usage" => {
                if let Some(usage) = json.get("usage") {
                    let prompt = usage
                        .get("input_tokens")
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0) as usize;
                    let completion = usage
                        .get("output_tokens")
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0) as usize;
                    if prompt > 0 || completion > 0 {
                        return SseEvent::Usage(Usage {
                            prompt_tokens: prompt,
                            completion_tokens: completion,
                            total_tokens: prompt + completion,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                        });
                    }
                }
            }

            // Other events we can skip
            _ => {}
        }

        SseEvent::Skip
    }
}

impl Default for OpenAIParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SseParser for OpenAIParser {
    async fn parse_event(&self, json: &Value) -> Result<SseEvent> {
        // Check for Responses API format (has "type" field)
        if let Some(event_type) = json.get("type").and_then(|t| t.as_str()) {
            return Ok(self.parse_responses_api_event(json, event_type));
        }

        // OpenAI Chat Completions format: {"choices": [{"index": 0, "delta": {...}, "finish_reason": null}]}
        let choices = json.get("choices").and_then(|c| c.as_array());

        if let Some(choices) = choices {
            if let Some(choice) = choices.first() {
                // Check for finish_reason
                if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                    if reason == "stop" || reason == "end_turn" {
                        return Ok(SseEvent::Finish {
                            reason: FinishReason::Stop,
                        });
                    }
                    if reason == "tool_calls" {
                        return Ok(SseEvent::Finish {
                            reason: FinishReason::ToolCalls,
                        });
                    }
                }

                // Check for delta content
                if let Some(delta) = choice.get("delta") {
                    // Regular text content
                    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                        if !content.is_empty() {
                            return Ok(SseEvent::TextDelta(content.to_string()));
                        }
                    }

                    // Reasoning content (GLM-style thinking)
                    if let Some(reasoning) = delta.get("reasoning_content").and_then(|r| r.as_str())
                    {
                        if !reasoning.is_empty() {
                            // Treat reasoning as thinking delta
                            return Ok(SseEvent::ThinkingDelta {
                                index: 0,
                                thinking: reasoning.to_string(),
                            });
                        }
                    }

                    // Tool calls
                    if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                        for tool_call in tool_calls {
                            let index = tool_call.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                as usize;

                            // Check for function info (start of tool call)
                            if let Some(function) = tool_call.get("function") {
                                let id = tool_call
                                    .get("id")
                                    .and_then(|i| i.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                if let Some(name) = function.get("name").and_then(|n| n.as_str()) {
                                    // New tool call starting
                                    let mut accumulators = self.tool_accumulators.lock().unwrap();
                                    accumulators.insert(
                                        index,
                                        ToolCallAccumulator::new(id.clone(), name.to_string()),
                                    );
                                    return Ok(SseEvent::ToolCallStart {
                                        id,
                                        name: name.to_string(),
                                    });
                                }

                                if let Some(args) =
                                    function.get("arguments").and_then(|a| a.as_str())
                                {
                                    // Arguments delta
                                    let mut accumulators = self.tool_accumulators.lock().unwrap();
                                    if let Some(acc) = accumulators.get_mut(&index) {
                                        acc.add_arguments(args);
                                        return Ok(SseEvent::ToolCallDelta {
                                            id: acc.id.clone(),
                                            delta: args.to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check for usage info
        if let Some(usage) = json.get("usage") {
            let prompt_tokens = usage
                .get("prompt_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0) as usize;
            let completion_tokens = usage
                .get("completion_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0) as usize;
            if prompt_tokens > 0 || completion_tokens > 0 {
                return Ok(SseEvent::Usage(Usage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens: prompt_tokens + completion_tokens,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                }));
            }
        }

        // Check for [DONE] marker (OpenAI uses this)
        // This is handled at the SSE line level, but just in case
        Ok(SseEvent::Skip)
    }
}

/// Anthropic-specific SSE parser
pub struct AnthropicParser {
    /// Track tool calls by content block index
    tool_accumulators: std::sync::Mutex<std::collections::HashMap<usize, ToolCallAccumulator>>,
    /// Track thinking blocks by content block index
    thinking_accumulators: std::sync::Mutex<std::collections::HashMap<usize, ThinkingAccumulator>>,
    /// Track server tool uses by content block index
    server_tool_accumulators:
        std::sync::Mutex<std::collections::HashMap<usize, ServerToolAccumulator>>,
}

impl AnthropicParser {
    pub fn new() -> Self {
        Self {
            tool_accumulators: std::sync::Mutex::new(std::collections::HashMap::new()),
            thinking_accumulators: std::sync::Mutex::new(std::collections::HashMap::new()),
            server_tool_accumulators: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl SseParser for AnthropicParser {
    async fn parse_event(&self, json: &Value) -> Result<SseEvent> {
        let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            "content_block_start" => {
                let index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;

                if let Some(content_block) = json.get("content_block") {
                    let block_type = content_block.get("type").and_then(|t| t.as_str());

                    match block_type {
                        Some("tool_use") => {
                            let id = content_block
                                .get("id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = content_block
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();

                            // Store accumulator by index
                            let mut accumulators = self.tool_accumulators.lock().unwrap();
                            accumulators
                                .insert(index, ToolCallAccumulator::new(id.clone(), name.clone()));

                            return Ok(SseEvent::ToolCallStart { id, name });
                        }
                        Some("server_tool_use") => {
                            // Server-executed tool (web_search, web_fetch)
                            let id = content_block
                                .get("id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = content_block
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();

                            let mut accumulators = self.server_tool_accumulators.lock().unwrap();
                            accumulators.insert(
                                index,
                                ServerToolAccumulator::new(id.clone(), name.clone()),
                            );

                            return Ok(SseEvent::ServerToolStart { id, name });
                        }
                        Some("web_search_tool_result") => {
                            // Parse search results immediately
                            let tool_use_id = content_block
                                .get("tool_use_id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("")
                                .to_string();

                            let results = self.parse_search_results(content_block);
                            return Ok(SseEvent::WebSearchResults {
                                tool_use_id,
                                results,
                            });
                        }
                        Some("web_fetch_tool_result") => {
                            // Parse fetch result immediately
                            let tool_use_id = content_block
                                .get("tool_use_id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("")
                                .to_string();

                            if let Some(content) = self.parse_fetch_result(content_block) {
                                return Ok(SseEvent::WebFetchResult {
                                    tool_use_id,
                                    content,
                                });
                            }

                            // Check for error
                            if let Some(err_content) = content_block.get("content") {
                                if let Some(err_type) =
                                    err_content.get("type").and_then(|t| t.as_str())
                                {
                                    if err_type == "web_fetch_tool_error"
                                        || err_type == "web_search_tool_result_error"
                                    {
                                        let error_code = err_content
                                            .get("error_code")
                                            .and_then(|e| e.as_str())
                                            .unwrap_or("unknown")
                                            .to_string();
                                        return Ok(SseEvent::ServerToolError {
                                            tool_use_id,
                                            error_code,
                                        });
                                    }
                                }
                            }
                        }
                        Some("thinking") => {
                            // Start tracking thinking block
                            let mut accumulators = self.thinking_accumulators.lock().unwrap();
                            accumulators.insert(index, ThinkingAccumulator::new());
                            return Ok(SseEvent::ThinkingStart { index });
                        }
                        _ => {}
                    }
                }
                Ok(SseEvent::Skip)
            }

            "content_block_delta" => {
                let index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;

                if let Some(delta) = json.get("delta") {
                    let delta_type = delta.get("type").and_then(|t| t.as_str());

                    match delta_type {
                        Some("text_delta") => {
                            let text = delta
                                .get("text")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();

                            // Check for citations
                            if let Some(citations_arr) =
                                delta.get("citations").and_then(|c| c.as_array())
                            {
                                let citations = self.parse_citations(citations_arr);
                                if !citations.is_empty() {
                                    return Ok(SseEvent::TextDeltaWithCitations {
                                        text,
                                        citations,
                                    });
                                }
                            }
                            return Ok(SseEvent::TextDelta(text));
                        }
                        Some("input_json_delta") => {
                            let partial_json = delta
                                .get("partial_json")
                                .and_then(|p| p.as_str())
                                .unwrap_or("")
                                .to_string();

                            // Check server tool accumulator first
                            {
                                let mut accumulators =
                                    self.server_tool_accumulators.lock().unwrap();
                                if let Some(acc) = accumulators.get_mut(&index) {
                                    acc.add_input(&partial_json);
                                    return Ok(SseEvent::ServerToolDelta {
                                        id: acc.id.clone(),
                                        delta: partial_json,
                                    });
                                }
                            }

                            // Then check client tool accumulator
                            let mut accumulators = self.tool_accumulators.lock().unwrap();
                            if let Some(acc) = accumulators.get_mut(&index) {
                                acc.add_arguments(&partial_json);
                                return Ok(SseEvent::ToolCallDelta {
                                    id: acc.id.clone(),
                                    delta: partial_json,
                                });
                            }
                        }
                        Some("thinking_delta") => {
                            let thinking = delta
                                .get("thinking")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();

                            // Update thinking accumulator
                            let mut accumulators = self.thinking_accumulators.lock().unwrap();
                            if let Some(acc) = accumulators.get_mut(&index) {
                                acc.add_thinking(&thinking);
                            }
                            return Ok(SseEvent::ThinkingDelta { index, thinking });
                        }
                        Some("signature_delta") => {
                            let signature = delta
                                .get("signature")
                                .and_then(|s| s.as_str())
                                .unwrap_or("")
                                .to_string();

                            // Update thinking accumulator signature
                            let mut accumulators = self.thinking_accumulators.lock().unwrap();
                            if let Some(acc) = accumulators.get_mut(&index) {
                                acc.add_signature(&signature);
                            }
                            return Ok(SseEvent::SignatureDelta { index, signature });
                        }
                        _ => {}
                    }
                }
                Ok(SseEvent::Skip)
            }

            "content_block_stop" => {
                let index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;

                // Check for completed server tool
                {
                    let mut accumulators = self.server_tool_accumulators.lock().unwrap();
                    if let Some(mut acc) = accumulators.remove(&index) {
                        let input = acc.complete();
                        return Ok(SseEvent::ServerToolComplete {
                            id: acc.id,
                            name: acc.name,
                            input,
                        });
                    }
                }

                // Check for completed client tool call
                {
                    let mut accumulators = self.tool_accumulators.lock().unwrap();
                    if let Some(mut acc) = accumulators.remove(&index) {
                        if let Some(tool_call) = acc.try_complete() {
                            return Ok(SseEvent::ToolCallComplete(tool_call));
                        } else {
                            // Force complete if JSON is incomplete
                            return Ok(SseEvent::ToolCallComplete(acc.force_complete()));
                        }
                    }
                }

                // Check for completed thinking block
                {
                    let mut accumulators = self.thinking_accumulators.lock().unwrap();
                    if let Some(mut acc) = accumulators.remove(&index) {
                        let (thinking, signature) = acc.complete();
                        return Ok(SseEvent::ThinkingComplete {
                            index,
                            thinking,
                            signature,
                        });
                    }
                }

                Ok(SseEvent::Skip)
            }

            "message_delta" => {
                // Check usage FIRST (message_delta contains final token counts)
                // Must check before stop_reason since both can be in same event
                if let Some(usage) = json.get("usage") {
                    let input_tokens = usage
                        .get("input_tokens")
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0) as usize;
                    let output_tokens = usage
                        .get("output_tokens")
                        .and_then(|t| t.as_u64())
                        .unwrap_or(0) as usize;

                    // Only emit Usage if we have actual token data
                    if input_tokens > 0 || output_tokens > 0 {
                        let cache_read = usage
                            .get("cache_read_input_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0) as usize;
                        let cache_creation = usage
                            .get("cache_creation_input_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0) as usize;
                        return Ok(SseEvent::Usage(Usage {
                            prompt_tokens: input_tokens,
                            completion_tokens: output_tokens,
                            total_tokens: input_tokens + output_tokens,
                            cache_creation_input_tokens: cache_creation,
                            cache_read_input_tokens: cache_read,
                        }));
                    }
                }

                // Then check for stop_reason (Finish comes from message_stop anyway)
                if let Some(delta) = json.get("delta") {
                    if let Some(stop_reason) = delta.get("stop_reason").and_then(|s| s.as_str()) {
                        let reason = parse_finish_reason(stop_reason);
                        return Ok(SseEvent::Finish { reason });
                    }
                }

                Ok(SseEvent::Skip)
            }

            "message_start" => {
                if let Some(message) = json.get("message") {
                    // Parse context editing metrics first
                    if let Some(ctx_edit) = message.get("context_editing") {
                        let metrics = ContextEditingMetrics {
                            cleared_tool_uses: ctx_edit
                                .get("cleared_tool_uses")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as usize,
                            cleared_thinking_turns: ctx_edit
                                .get("cleared_thinking_turns")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as usize,
                            cleared_input_tokens: ctx_edit
                                .get("cleared_input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0)
                                as usize,
                        };
                        if metrics.cleared_input_tokens > 0 {
                            info!("Context edited: cleared {} tokens ({} tool uses, {} thinking turns)",
                                metrics.cleared_input_tokens,
                                metrics.cleared_tool_uses,
                                metrics.cleared_thinking_turns);
                        }
                        return Ok(SseEvent::ContextEdited(metrics));
                    }

                    if let Some(usage) = message.get("usage") {
                        let input_tokens = usage
                            .get("input_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0) as usize;
                        let cache_creation = usage
                            .get("cache_creation_input_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0) as usize;
                        let cache_read = usage
                            .get("cache_read_input_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0) as usize;

                        // Log cache metrics
                        if cache_creation > 0 || cache_read > 0 {
                            info!(
                                "Cache metrics: read={}, created={}, fresh={}",
                                cache_read, cache_creation, input_tokens
                            );
                        }

                        let total_input = input_tokens + cache_creation + cache_read;
                        return Ok(SseEvent::Usage(Usage {
                            prompt_tokens: total_input,
                            completion_tokens: 0,
                            total_tokens: total_input,
                            cache_creation_input_tokens: cache_creation,
                            cache_read_input_tokens: cache_read,
                        }));
                    }
                }
                Ok(SseEvent::Skip)
            }

            "message_stop" => Ok(SseEvent::Finish {
                reason: FinishReason::Stop,
            }),

            "error" => {
                let error_msg = json
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error");
                Err(anyhow::anyhow!("API error: {}", error_msg))
            }

            _ => Ok(SseEvent::Skip),
        }
    }
}

impl Default for AnthropicParser {
    fn default() -> Self {
        Self::new()
    }
}

// Helper methods for parsing web search/fetch results
impl AnthropicParser {
    /// Parse web search results from content block
    fn parse_search_results(&self, content_block: &Value) -> Vec<WebSearchResult> {
        let mut results = Vec::new();

        if let Some(content_arr) = content_block.get("content").and_then(|c| c.as_array()) {
            for item in content_arr {
                if item.get("type").and_then(|t| t.as_str()) == Some("web_search_result") {
                    let url = item
                        .get("url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("")
                        .to_string();
                    let title = item
                        .get("title")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    let encrypted_content = item
                        .get("encrypted_content")
                        .and_then(|e| e.as_str())
                        .map(|s| s.to_string());
                    let page_age = item
                        .get("page_age")
                        .and_then(|p| p.as_str())
                        .map(|s| s.to_string());

                    results.push(WebSearchResult {
                        url,
                        title,
                        encrypted_content,
                        page_age,
                    });
                }
            }
        }

        results
    }

    /// Parse web fetch result from content block
    fn parse_fetch_result(&self, content_block: &Value) -> Option<WebFetchContent> {
        let content = content_block.get("content")?;

        // Check if it's a web_fetch_result
        if content.get("type").and_then(|t| t.as_str()) != Some("web_fetch_result") {
            return None;
        }

        let url = content
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();

        let retrieved_at = content
            .get("retrieved_at")
            .and_then(|r| r.as_str())
            .map(|s| s.to_string());

        // Parse document content
        if let Some(doc) = content.get("content") {
            let title = doc
                .get("title")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());

            if let Some(source) = doc.get("source") {
                let media_type = source
                    .get("media_type")
                    .and_then(|m| m.as_str())
                    .unwrap_or("text/plain")
                    .to_string();

                // Get content based on source type
                let content_data = if source.get("type").and_then(|t| t.as_str()) == Some("base64")
                {
                    source
                        .get("data")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string()
                } else {
                    source
                        .get("data")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string()
                };

                return Some(WebFetchContent {
                    url,
                    content: content_data,
                    media_type,
                    title,
                    retrieved_at,
                });
            }
        }

        None
    }

    /// Parse citations from a text delta
    fn parse_citations(&self, citations_arr: &[Value]) -> Vec<Citation> {
        citations_arr
            .iter()
            .filter_map(|c| {
                // Handle web_search_result_location type
                let url = c
                    .get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                let title = c
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let cited_text = c
                    .get("cited_text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();

                if url.is_empty() && title.is_empty() {
                    None
                } else {
                    Some(Citation {
                        url,
                        title,
                        cited_text,
                    })
                }
            })
            .collect()
    }
}
