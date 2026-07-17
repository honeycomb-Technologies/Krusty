//! Shared provider model catalog helpers.

use anyhow::Result;

use crate::auth::{
    resolve_anthropic_auth, resolve_grok_auth, resolve_openai_auth, AnthropicAuthType,
    OpenAIAuthMode, OpenAIAuthType,
};
use crate::storage::CredentialStore;

use super::models::ModelMetadata;
use super::providers::{get_provider, ProviderId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogAuthKind {
    ApiKey,
    OAuth,
}

/// Authentication context for account-scoped model discovery.
///
/// The credential is deliberately private so debug output cannot accidentally
/// print it. Provider adapters access it through the read-only accessor.
#[derive(Clone)]
pub struct CatalogCredential {
    credential: String,
    pub kind: CatalogAuthKind,
    pub account_id: Option<String>,
}

impl CatalogCredential {
    pub fn api_key(credential: String) -> Self {
        Self {
            credential,
            kind: CatalogAuthKind::ApiKey,
            account_id: None,
        }
    }

    pub fn oauth(credential: String, account_id: Option<String>) -> Self {
        Self {
            credential,
            kind: CatalogAuthKind::OAuth,
            account_id,
        }
    }

    pub fn credential(&self) -> &str {
        &self.credential
    }
}

/// Whether the provider supports runtime model discovery.
pub fn supports_dynamic_models(provider: ProviderId) -> bool {
    get_provider(provider)
        .map(|config| config.dynamic_models)
        .unwrap_or(false)
}

/// Providers that expose a live `/models` (or equivalent) catalog at runtime.
pub fn dynamic_model_providers() -> Vec<ProviderId> {
    ProviderId::all()
        .iter()
        .copied()
        .filter(|provider| supports_dynamic_models(*provider))
        .collect()
}

/// Resolve every usable catalog identity for a provider.
///
/// OpenAI may legitimately have both an API key and a ChatGPT OAuth account;
/// their catalogs are fetched independently and merged by model ID. Other
/// providers currently expose one effective catalog identity.
pub fn credentials_for_dynamic_models(
    provider: ProviderId,
    credentials: &CredentialStore,
) -> Vec<CatalogCredential> {
    credentials_for_dynamic_models_with_env(provider, credentials, env_credential)
}

/// Backwards-compatible single catalog identity for callers that only need to
/// decide whether a refresh is possible.
pub fn credential_for_dynamic_models(
    provider: ProviderId,
    credentials: &CredentialStore,
) -> Option<CatalogCredential> {
    credentials_for_dynamic_models(provider, credentials)
        .into_iter()
        .next()
}

fn credentials_for_dynamic_models_with_env(
    provider: ProviderId,
    credentials: &CredentialStore,
    env: impl Fn(&str) -> Option<String>,
) -> Vec<CatalogCredential> {
    match provider {
        ProviderId::OpenRouter => credentials
            .get(&ProviderId::OpenRouter)
            .cloned()
            .or_else(|| env("OPENROUTER_API_KEY"))
            .map(CatalogCredential::api_key)
            .into_iter()
            .collect(),
        ProviderId::OpenAI => {
            let mode = OpenAIAuthMode::from_env();
            let mut auth = Vec::new();
            if mode != OpenAIAuthMode::OAuth {
                if let Some(key) = credentials
                    .get(&ProviderId::OpenAI)
                    .cloned()
                    .or_else(|| env("OPENAI_API_KEY"))
                {
                    auth.push(CatalogCredential::api_key(key));
                }
            }

            if mode != OpenAIAuthMode::ApiKey {
                let resolved = resolve_openai_auth(credentials, "gpt-5.3-codex");
                if resolved.auth_type == OpenAIAuthType::ChatGptOAuth {
                    if let Some(token) = resolved.credential {
                        if !auth.iter().any(|entry| entry.credential == token) {
                            auth.push(CatalogCredential::oauth(token, resolved.account_id));
                        }
                    }
                }
            }
            auth
        }
        ProviderId::Anthropic => {
            let resolved = resolve_anthropic_auth(credentials);
            resolved
                .credential
                .or_else(|| env("ANTHROPIC_API_KEY"))
                .map(|credential| {
                    if resolved.auth_type == AnthropicAuthType::OAuth {
                        CatalogCredential::oauth(credential, None)
                    } else {
                        CatalogCredential::api_key(credential)
                    }
                })
                .into_iter()
                .collect()
        }
        ProviderId::MiniMax => credentials
            .get(&ProviderId::MiniMax)
            .cloned()
            .or_else(|| env("MINIMAX_API_KEY"))
            .map(CatalogCredential::api_key)
            .into_iter()
            .collect(),
        ProviderId::Grok => resolve_grok_auth(credentials)
            .credential
            .or_else(|| env("GROK_ACCESS_TOKEN"))
            .map(|credential| CatalogCredential::oauth(credential, None))
            .into_iter()
            .collect(),
        ProviderId::ZAi => Vec::new(),
    }
}

fn env_credential(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Fetch runtime model metadata using one catalog identity.
pub async fn fetch_dynamic_models(
    provider: ProviderId,
    auth: &CatalogCredential,
) -> Result<Vec<ModelMetadata>> {
    match provider {
        ProviderId::OpenRouter => super::openrouter::fetch_models(auth.credential()).await,
        ProviderId::OpenAI if auth.kind == CatalogAuthKind::OAuth => {
            super::openai::fetch_chatgpt_models(auth.credential(), auth.account_id.as_deref()).await
        }
        ProviderId::OpenAI => super::openai::fetch_models(auth.credential()).await,
        ProviderId::Anthropic => {
            super::anthropic_catalog::fetch_models(
                auth.credential(),
                auth.kind == CatalogAuthKind::OAuth,
            )
            .await
        }
        ProviderId::MiniMax => super::minimax_catalog::fetch_models(auth.credential()).await,
        ProviderId::Grok => super::grok::fetch_models(auth.credential()).await,
        ProviderId::ZAi => anyhow::bail!("Z.ai does not expose a model-list endpoint"),
    }
}

/// Fetch and combine every configured catalog identity for a provider.
/// A partial success wins over a failed sibling identity; all failures surface
/// an error so callers retain their last-known-good snapshot.
pub async fn fetch_dynamic_models_for_store(
    provider: ProviderId,
    credentials: &CredentialStore,
) -> Result<Vec<ModelMetadata>> {
    let auth = credentials_for_dynamic_models(provider, credentials);
    if auth.is_empty() {
        anyhow::bail!("No catalog credential configured for {provider}");
    }

    let mut merged = Vec::<ModelMetadata>::new();
    let mut last_error = None;
    let mut successes = 0usize;

    for identity in auth {
        match fetch_dynamic_models(provider, &identity).await {
            Ok(models) if !models.is_empty() => {
                successes += 1;
                for model in models {
                    if let Some(existing) = merged.iter_mut().find(|item| item.id == model.id) {
                        // Later, richer identities (ChatGPT OAuth for OpenAI)
                        // replace sparse API-key metadata without reordering.
                        *existing = model;
                    } else {
                        merged.push(model);
                    }
                }
            }
            Ok(_) => {
                last_error = Some(anyhow::anyhow!("Provider returned an empty model catalog"));
            }
            Err(error) => last_error = Some(error),
        }
    }

    if successes == 0 {
        return Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Model catalog refresh failed")));
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::{credentials_for_dynamic_models_with_env, CatalogAuthKind};
    use crate::ai::providers::ProviderId;
    use crate::storage::CredentialStore;

    #[test]
    fn openai_api_catalog_reads_stored_key() {
        let mut credentials = CredentialStore::default();
        credentials.set(ProviderId::OpenAI, "sk-openai".to_string());

        let auth = credentials_for_dynamic_models_with_env(
            ProviderId::OpenAI,
            &credentials,
            |_| None,
        );

        assert!(auth.iter().any(|entry| {
            entry.kind == CatalogAuthKind::ApiKey && entry.credential() == "sk-openai"
        }));
    }

    #[test]
    fn static_zai_provider_has_no_runtime_catalog_identity() {
        let mut credentials = CredentialStore::default();
        credentials.set(ProviderId::ZAi, "zai-key".to_string());

        assert!(credentials_for_dynamic_models_with_env(
            ProviderId::ZAi,
            &credentials,
            |_| Some("env-key".to_string()),
        )
        .is_empty());
    }

    #[test]
    fn dynamic_provider_list_includes_new_direct_catalogs() {
        let providers = super::dynamic_model_providers();
        assert!(providers.contains(&ProviderId::OpenRouter));
        assert!(providers.contains(&ProviderId::OpenAI));
        assert!(providers.contains(&ProviderId::Grok));
        assert!(providers.contains(&ProviderId::MiniMax));
        assert!(providers.contains(&ProviderId::Anthropic));
        assert!(!providers.contains(&ProviderId::ZAi));
    }
}
