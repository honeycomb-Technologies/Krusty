//! AI SDK types for provider communication
//!
//! These are NOT domain types - they're specific to AI provider APIs

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::reasoning::DEFAULT_THINKING_BUDGET;

/// AI SDK Tool definition (for provider communication only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Extended prompt for system prompt injection (internal only, not sent to providers)
    #[serde(skip)]
    pub prompt: Option<String>,
}

/// AI SDK Tool call (for provider communication only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Message role in a conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Content types that can be in a message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Content {
    #[serde(rename = "text")]
    Text { text: String },

    #[serde(rename = "image")]
    Image {
        image: ImageContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },

    /// Document content (PDF)
    #[serde(rename = "document")]
    Document { source: DocumentSource },

    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        output: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },

    /// Extended thinking content block
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },

    /// Redacted thinking (when thinking contains sensitive content)
    #[serde(rename = "redacted_thinking")]
    RedactedThinking { data: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

/// Document source for PDF content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSource {
    /// Source type: "base64" or "url"
    #[serde(rename = "type")]
    pub source_type: String,
    /// MIME type (e.g., "application/pdf")
    pub media_type: String,
    /// Base64-encoded content (when source_type is "base64")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// URL to fetch (when source_type is "url")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Unified message format for provider communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMessage {
    pub role: Role,
    pub content: Vec<Content>,
}

/// Finish reasons for model generation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other(String),
}

/// Usage information with cache metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
    /// Tokens written to cache (25% extra cost)
    #[serde(default)]
    pub cache_creation_input_tokens: usize,
    /// Tokens read from cache (10% cost vs 100%)
    #[serde(default)]
    pub cache_read_input_tokens: usize,
}

/// Context management configuration for automatic context editing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManagement {
    pub edits: Vec<ContextEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContextEdit {
    #[serde(rename = "clear_tool_uses_20250919")]
    ClearToolUses {
        #[serde(skip_serializing_if = "Option::is_none")]
        trigger: Option<ContextTrigger>,
        #[serde(skip_serializing_if = "Option::is_none")]
        keep: Option<KeepConfig>,
    },
    #[serde(rename = "clear_thinking_20251015")]
    ClearThinking {
        #[serde(skip_serializing_if = "Option::is_none")]
        keep: Option<KeepConfig>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContextTrigger {
    #[serde(rename = "input_tokens")]
    InputTokens { value: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum KeepConfig {
    #[serde(rename = "tool_uses")]
    ToolUses { value: usize },
    #[serde(rename = "thinking_turns")]
    ThinkingTurns { value: usize },
}

/// Metrics from context editing operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextEditingMetrics {
    pub cleared_tool_uses: usize,
    pub cleared_thinking_turns: usize,
    pub cleared_input_tokens: usize,
}

impl ContextManagement {
    /// Default for extended thinking + tools (Krusty's main use case)
    /// Note: clear_thinking must come before clear_tool_uses per API requirement
    pub fn default_for_thinking_and_tools() -> Self {
        Self {
            edits: vec![
                // Thinking clearing - keep last 2 turns
                ContextEdit::ClearThinking {
                    keep: Some(KeepConfig::ThinkingTurns { value: 2 }),
                },
                // Tool clearing - trigger at 100k tokens, keep last 5
                ContextEdit::ClearToolUses {
                    trigger: Some(ContextTrigger::InputTokens { value: 100_000 }),
                    keep: Some(KeepConfig::ToolUses { value: 5 }),
                },
            ],
        }
    }

    /// For tools without thinking
    pub fn default_tools_only() -> Self {
        Self {
            edits: vec![ContextEdit::ClearToolUses {
                trigger: Some(ContextTrigger::InputTokens { value: 100_000 }),
                keep: Some(KeepConfig::ToolUses { value: 5 }),
            }],
        }
    }
}

/// Extended thinking configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    pub budget_tokens: u32,
}

impl Default for ThinkingConfig {
    fn default() -> Self {
        Self {
            budget_tokens: DEFAULT_THINKING_BUDGET,
        }
    }
}

// ============================================================================
// Server-Executed Tools (Web Search, Web Fetch)
// ============================================================================

/// Web search tool configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebSearchConfig {
    /// Maximum number of searches per request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
}

/// Web fetch tool configuration (beta)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchConfig {
    /// Maximum number of fetches per request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
    /// Enable citations for fetched content
    pub citations_enabled: bool,
    /// Maximum content length in tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_content_tokens: Option<u32>,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            max_uses: Some(10),
            citations_enabled: true,
            max_content_tokens: Some(100_000),
        }
    }
}

/// A single web search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResult {
    pub url: String,
    pub title: String,
    /// Encrypted content (must be passed back for citations)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
    /// When the page was last updated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_age: Option<String>,
}

/// Web fetch result content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchContent {
    pub url: String,
    /// The fetched content (text or base64 for PDFs)
    pub content: String,
    /// Media type (text/plain, application/pdf, etc.)
    pub media_type: String,
    /// Document title if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// When content was retrieved
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieved_at: Option<String>,
}

/// Citation from web search or fetch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub url: String,
    pub title: String,
    /// The cited text (up to 150 chars for search)
    pub cited_text: String,
}
