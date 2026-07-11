//! Shared provider model catalog helpers.

use anyhow::Result;

use crate::storage::CredentialStore;

use super::models::ModelMetadata;
use super::providers::{get_provider, ProviderId};

/// Whether the provider supports runtime model discovery.
pub fn supports_dynamic_models(provider: ProviderId) -> bool {
    get_provider(provider)
        .map(|config| config.dynamic_models)
        .unwrap_or(false)
}

/// Providers that expose a live `/models` (or equivalent) catalog at runtime.
///
/// Used by CLI and server so catalog bootstrap, cache restore, and refresh
/// stay aligned across surfaces.
pub fn dynamic_model_providers() -> Vec<ProviderId> {
    ProviderId::all()
        .iter()
        .copied()
        .filter(|provider| supports_dynamic_models(*provider))
        .collect()
}

/// Resolve the credential that is valid for runtime model discovery.
///
/// This intentionally differs from chat auth resolution. For example, OpenAI's
/// `/v1/models` endpoint accepts OpenAI API keys, but ChatGPT/Codex OAuth
/// tokens are for `chatgpt.com/backend-api/codex/*` inference calls and return
/// 403 when reused for catalog refreshes.
pub fn credential_for_dynamic_models(
    provider: ProviderId,
    credentials: &CredentialStore,
) -> Option<String> {
    credential_for_dynamic_models_with_env(provider, credentials, env_credential)
}

fn credential_for_dynamic_models_with_env(
    provider: ProviderId,
    credentials: &CredentialStore,
    env: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    match provider {
        ProviderId::OpenRouter => credentials
            .get(&ProviderId::OpenRouter)
            .cloned()
            .or_else(|| env("OPENROUTER_API_KEY")),
        ProviderId::OpenAI => credentials
            .get(&ProviderId::OpenAI)
            .cloned()
            .or_else(|| env("OPENAI_API_KEY")),
        ProviderId::Grok => crate::auth::resolve_grok_auth(credentials).credential,
        _ => None,
    }
}

fn env_credential(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Fetch runtime model metadata for a provider.
pub async fn fetch_dynamic_models(
    provider: ProviderId,
    credential: &str,
) -> Result<Vec<ModelMetadata>> {
    match provider {
        ProviderId::OpenRouter => super::openrouter::fetch_models(credential).await,
        ProviderId::OpenAI => super::openai::fetch_models(credential).await,
        ProviderId::Grok => super::grok::fetch_models(credential).await,
        _ => anyhow::bail!(
            "Provider {:?} does not support dynamic model discovery",
            provider
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::ai::catalog::credential_for_dynamic_models_with_env;
    use crate::ai::providers::ProviderId;
    use crate::storage::CredentialStore;

    #[test]
    fn openai_dynamic_catalog_uses_api_key_not_oauth_fallback() {
        let credentials = CredentialStore::default();

        let credential =
            credential_for_dynamic_models_with_env(ProviderId::OpenAI, &credentials, |_| None);

        assert!(credential.is_none());
    }

    #[test]
    fn openai_dynamic_catalog_reads_stored_api_key() {
        let mut credentials = CredentialStore::default();
        credentials.set(ProviderId::OpenAI, "sk-openai".to_string());

        let credential =
            credential_for_dynamic_models_with_env(ProviderId::OpenAI, &credentials, |_| None);

        assert_eq!(credential.as_deref(), Some("sk-openai"));
    }

    #[test]
    fn dynamic_catalog_ignores_static_providers() {
        let mut credentials = CredentialStore::default();
        credentials.set(ProviderId::MiniMax, "minimax-key".to_string());

        let credential =
            credential_for_dynamic_models_with_env(ProviderId::MiniMax, &credentials, |_| {
                Some("env-key".to_string())
            });

        assert!(credential.is_none());
    }

    #[test]
    fn dynamic_model_providers_includes_openrouter_openai_and_grok() {
        let providers = crate::ai::catalog::dynamic_model_providers();
        assert!(providers.contains(&ProviderId::OpenRouter));
        assert!(providers.contains(&ProviderId::OpenAI));
        assert!(providers.contains(&ProviderId::Grok));
        assert!(!providers.contains(&ProviderId::MiniMax));
        assert!(!providers.contains(&ProviderId::Anthropic));
    }
}
