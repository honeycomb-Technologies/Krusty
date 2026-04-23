//! Authentication for Krusty
//!
//! This module provides OAuth authentication support for providers that support it,
//! as well as the types and utilities needed for authentication flows.
//!
//! API key storage is handled by the credentials module in storage/

pub mod browser_flow;
mod credential_loading;
pub mod device_flow;
pub mod hosted_browser_flow;
pub mod openai_device_flow;
pub mod pkce;
pub mod providers;
mod refresh;
mod resolution;
pub mod storage;
pub mod types;

// Re-exports for convenience
pub use browser_flow::{
    open_browser, run_callback_server, BrowserOAuthFlow, CallbackResult, PasteCodeOAuthFlow,
    DEFAULT_CALLBACK_PORT,
};
pub use credential_loading::{extract_openai_account_id, is_anthropic_oauth_token};
pub use device_flow::{DeviceCodeFlow, DeviceCodeResponse};
pub use hosted_browser_flow::HostedBrowserOAuthFlow;
pub use openai_device_flow::{OpenAIDeviceAuthFlow, OpenAIDeviceCodeResponse};
pub use pkce::{PkceChallenge, PkceVerifier};
pub use providers::{anthropic_oauth_config, openai_oauth_config};
pub use refresh::{refresh_oauth_token, try_refresh_oauth_token_blocking};
pub use resolution::{
    detect_openai_auth_type, resolve_anthropic_auth, resolve_openai_auth, AnthropicAuthResolution,
    AnthropicAuthType, OpenAIAuthMode, OpenAIAuthResolution, OpenAIAuthType,
};
pub use storage::OAuthTokenStore;
pub use types::{AuthMethod, OAuthConfig, OAuthTokenData};
