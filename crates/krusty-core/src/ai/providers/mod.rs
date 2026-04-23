//! AI provider configuration.
//!
//! Public facade for provider identity, config, capability flags, model
//! translation, and the built-in provider registry.

mod capabilities;
mod config;
mod mapping;
mod registry;

pub use self::capabilities::ProviderCapabilities;
pub use self::config::{
    AuthHeader, ModelInfo, ProviderConfig, ProviderId, ReasoningFormat, CHATGPT_RESPONSES_API,
    OPENAI_CHAT_API, OPENAI_RESPONSES_API,
};
pub use self::mapping::{
    get_model_family, toggle_fast_model, translate_model_id, translate_model_or_default,
    ModelFamily,
};
pub use self::registry::{builtin_providers, get_provider};
