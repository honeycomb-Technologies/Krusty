use crate::ai::providers::ProviderId;
use crate::storage::CredentialStore;

use super::credential_loading::{load_anthropic_oauth_credential, load_openai_oauth_credential};

/// Auth type for OpenAI - determines which API endpoint to use
///
/// ChatGPT OAuth tokens require the Responses API at chatgpt.com,
/// while API keys use the standard Chat Completions API at api.openai.com.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAIAuthType {
    /// Using OAuth token from ChatGPT - requires Responses API
    ChatGptOAuth,
    /// Using API key - uses Chat Completions API
    ApiKey,
    /// No authentication configured
    None,
}

/// OpenAI auth selection mode.
///
/// - `Auto`: Prefer OAuth for Codex models, otherwise prefer API key.
/// - `OAuth`: Require ChatGPT OAuth token.
/// - `ApiKey`: Require API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAIAuthMode {
    Auto,
    OAuth,
    ApiKey,
}

impl OpenAIAuthMode {
    /// Parse auth mode from `MITSURO_OPENAI_AUTH_MODE`.
    ///
    /// Supported values: `auto`, `oauth`, `api_key`.
    pub fn from_env() -> Self {
        match crate::identity::env_var("MITSURO_OPENAI_AUTH_MODE")
            .ok()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "oauth" => Self::OAuth,
            "api_key" => Self::ApiKey,
            _ => Self::Auto,
        }
    }
}

/// Resolved OpenAI auth information for a model.
#[derive(Debug, Clone)]
pub struct OpenAIAuthResolution {
    pub auth_type: OpenAIAuthType,
    pub credential: Option<String>,
    pub account_id: Option<String>,
}

/// Auth type for Anthropic - determines auth header format
///
/// OAuth tokens (sk-ant-oat*) use Bearer auth and require CC identity headers.
/// API keys (sk-ant-*) use x-api-key header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicAuthType {
    /// Using OAuth token from Claude - requires Bearer auth + CC identity
    OAuth,
    /// Using API key - uses x-api-key header
    ApiKey,
    /// No authentication configured
    None,
}

/// Resolved Anthropic auth information.
#[derive(Debug, Clone)]
pub struct AnthropicAuthResolution {
    pub auth_type: AnthropicAuthType,
    pub credential: Option<String>,
}

/// Resolve Anthropic auth type + credential.
///
/// Checks OAuth token store first, falls back to API key from credential store.
pub fn resolve_anthropic_auth(credentials: &CredentialStore) -> AnthropicAuthResolution {
    if let Some((access_token, _)) = load_anthropic_oauth_credential() {
        return AnthropicAuthResolution {
            auth_type: AnthropicAuthType::OAuth,
            credential: Some(access_token),
        };
    }

    if let Some(key) = credentials.get(&ProviderId::Anthropic).cloned() {
        return AnthropicAuthResolution {
            auth_type: AnthropicAuthType::ApiKey,
            credential: Some(key),
        };
    }

    AnthropicAuthResolution {
        auth_type: AnthropicAuthType::None,
        credential: None,
    }
}

/// Resolve OpenAI auth type + credential for a specific model.
pub fn resolve_openai_auth(credentials: &CredentialStore, model: &str) -> OpenAIAuthResolution {
    let mode = OpenAIAuthMode::from_env();
    let is_codex_model = model.to_ascii_lowercase().contains("codex");

    let api_key = credentials.get(&ProviderId::OpenAI).cloned();
    let oauth = load_openai_oauth_credential();

    match mode {
        OpenAIAuthMode::OAuth => {
            if let Some((access_token, account_id)) = oauth {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::ChatGptOAuth,
                    credential: Some(access_token),
                    account_id,
                }
            } else {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::None,
                    credential: None,
                    account_id: None,
                }
            }
        }
        OpenAIAuthMode::ApiKey => {
            if let Some(key) = api_key {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::ApiKey,
                    credential: Some(key),
                    account_id: None,
                }
            } else {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::None,
                    credential: None,
                    account_id: None,
                }
            }
        }
        OpenAIAuthMode::Auto => {
            if is_codex_model {
                if let Some((access_token, account_id)) = oauth.clone() {
                    return OpenAIAuthResolution {
                        auth_type: OpenAIAuthType::ChatGptOAuth,
                        credential: Some(access_token),
                        account_id,
                    };
                }
                if let Some(key) = api_key {
                    return OpenAIAuthResolution {
                        auth_type: OpenAIAuthType::ApiKey,
                        credential: Some(key),
                        account_id: None,
                    };
                }
            } else if let Some(key) = api_key {
                return OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::ApiKey,
                    credential: Some(key),
                    account_id: None,
                };
            }

            if let Some((access_token, account_id)) = oauth {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::ChatGptOAuth,
                    credential: Some(access_token),
                    account_id,
                }
            } else {
                OpenAIAuthResolution {
                    auth_type: OpenAIAuthType::None,
                    credential: None,
                    account_id: None,
                }
            }
        }
    }
}

/// Detect which type of OpenAI authentication is configured.
///
/// Uses `resolve_openai_auth` with codex-aware defaults.
pub fn detect_openai_auth_type(credentials: &CredentialStore) -> OpenAIAuthType {
    resolve_openai_auth(credentials, "gpt-5.3-codex").auth_type
}
