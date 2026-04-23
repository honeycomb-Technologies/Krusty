//! Provider identity and routing policy.

mod routing;
#[cfg(test)]
mod tests;
mod types;

pub use self::types::{
    AuthHeader, ModelInfo, ProviderConfig, ProviderId, ReasoningFormat, CHATGPT_RESPONSES_API,
    OPENAI_CHAT_API, OPENAI_RESPONSES_API,
};
