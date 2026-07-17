//! AI Client configuration
//!
//! Provider-agnostic configuration for AI API clients.

mod ai_client;
mod call_options;
mod effort;

pub use ai_client::AiClientConfig;
pub(crate) use call_options::{
    anthropic_prompt_cache_control, normalized_prompt_cache_key, openai_prompt_cache_options,
    openai_prompt_cache_retention, OpenAiPromptCacheMode,
};
pub use call_options::{CallOptions, PromptCacheRetention};
pub use effort::{supports_openai_xhigh_reasoning, AnthropicAdaptiveEffort, CodexReasoningEffort};
