//! AI SDK types for provider communication
//!
//! These are NOT domain types - they're specific to AI provider APIs.

mod context;
mod messages;
mod web;

pub use context::{
    ContextEdit, ContextEditingMetrics, ContextManagement, ContextTrigger, KeepConfig,
    ThinkingConfig, Usage,
};
pub use messages::{
    AiTool, AiToolCall, Content, DocumentSource, FinishReason, ImageContent, ModelMessage, Role,
};
pub use web::{Citation, WebFetchConfig, WebFetchContent, WebSearchConfig, WebSearchResult};
