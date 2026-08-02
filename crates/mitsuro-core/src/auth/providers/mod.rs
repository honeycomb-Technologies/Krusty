//! Provider-specific OAuth configurations

mod anthropic;
mod openai;

pub use anthropic::anthropic_oauth_config;
pub use openai::openai_oauth_config;
