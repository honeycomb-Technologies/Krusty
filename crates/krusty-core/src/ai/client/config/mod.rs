//! AI Client configuration
//!
//! Provider-agnostic configuration for AI API clients.

mod ai_client;
mod call_options;
mod effort;

pub use ai_client::AiClientConfig;
pub use call_options::CallOptions;
pub use effort::{supports_openai_xhigh_reasoning, AnthropicAdaptiveEffort, CodexReasoningEffort};
