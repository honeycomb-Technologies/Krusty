//! AI Client module
//!
//! Provider-agnostic AI API client supporting multiple providers and formats:
//! - Anthropic (native)
//! - OpenAI chat/completions
//! - Google/Gemini
//!
//! Each method routes to the appropriate format handler based on the provider's API format.

pub mod config;
pub mod core;
pub mod request_builder;
pub mod simple;
pub mod streaming;
pub mod thinking;
pub mod tools;

// Re-export main types
pub use config::{
    supports_openai_xhigh_reasoning, AiClientConfig, AnthropicAdaptiveEffort, CallOptions,
    CodexReasoningEffort, PromptCacheRetention,
};
pub use core::MITSURO_SYSTEM_PROMPT;
pub use core::{AiClient, PreparedRequestDiagnostics};
pub use request_builder::{BuildOptions, RequestBuilder};
pub use simple::SimpleCallResult;
