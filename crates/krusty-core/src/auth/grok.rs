use std::future::Future;

use anyhow::{Context, Result};
use chrono::Utc;

use crate::ai::providers::ProviderId;
use crate::storage::CredentialStore;

use super::{OAuthTokenData, OAuthTokenStore};

/// Resolved Grok/X-subscription auth information.
#[derive(Debug, Clone)]
pub struct GrokAuthResolution {
    /// Bearer credential to send to the Grok chat proxy.
    pub credential: Option<String>,
    /// Source entry in `~/.grok/auth.json` when using X-subscription auth.
    pub issuer_key: Option<String>,
}

/// Resolve Grok auth from the shared Grok CLI auth store.
pub fn resolve_grok_auth(_credentials: &CredentialStore) -> GrokAuthResolution {
    match try_ensure_grok_auth_token_blocking() {
        Some(token) => GrokAuthResolution {
            credential: Some(token.access_token),
            issuer_key: Some(token.issuer_key),
        },
        None => GrokAuthResolution {
            credential: None,
            issuer_key: None,
        },
    }
}

/// Ensure a fresh Grok token using the shared `~/.grok/auth.json` store.
pub async fn ensure_grok_auth_token() -> Result<grok_auth::AuthToken> {
    let mut cfg = grok_auth::AuthConfig::from_env().context("failed to build Grok auth config")?;
    cfg.api_key = None;
    let store = grok_auth::AuthStore::new(cfg.auth_file.clone(), cfg)
        .await
        .context("failed to open Grok auth store")?;
    store
        .ensure_fresh()
        .await
        .context("failed to resolve Grok auth token")
}

/// Remove the shared Grok CLI auth file used by `~/.grok/auth.json`.
pub fn clear_grok_cli_auth() -> Result<()> {
    let cfg = grok_auth::AuthConfig::from_env().context("failed to build Grok auth config")?;
    if cfg.auth_file.exists() {
        std::fs::remove_file(&cfg.auth_file)
            .with_context(|| format!("failed to remove {}", cfg.auth_file.display()))?;
    }
    Ok(())
}

/// Run a fresh browser login and persist it to the shared Grok auth store.
pub async fn force_grok_browser_login() -> Result<grok_auth::AuthToken> {
    let mut cfg = grok_auth::AuthConfig::from_env().context("failed to build Grok auth config")?;
    cfg.api_key = None;
    cfg.allow_browser = true;
    let store = grok_auth::AuthStore::new(cfg.auth_file.clone(), cfg)
        .await
        .context("failed to open Grok auth store")?;
    store
        .force_login()
        .await
        .context("failed to complete Grok browser login")
}

/// Convert a grok-auth token to Krusty's generic OAuth token shape for UI status.
pub fn grok_auth_token_to_oauth_data(token: &grok_auth::AuthToken) -> OAuthTokenData {
    OAuthTokenData {
        access_token: token.access_token.clone(),
        refresh_token: token.refresh_token.clone(),
        id_token: None,
        expires_at: token
            .expires_at
            .map(|expires| expires.timestamp().max(0) as u64),
        last_refresh: Utc::now().timestamp().max(0) as u64,
        account_id: Some(token.issuer_key.clone()),
    }
}

/// Ensure Grok auth is fresh and mirror the status into Krusty's OAuth store.
pub async fn refresh_grok_oauth_token() -> Result<OAuthTokenData> {
    let token = ensure_grok_auth_token().await?;
    let oauth = grok_auth_token_to_oauth_data(&token);

    OAuthTokenStore::set_persisted(ProviderId::Grok, oauth.clone())
        .context("failed to save mirrored Grok OAuth token")?;

    Ok(oauth)
}

pub(crate) fn try_ensure_grok_auth_token_blocking() -> Option<grok_auth::AuthToken> {
    block_on_grok_auth(ensure_grok_auth_token(), "resolving Grok auth token")
}

fn block_on_grok_auth<T>(
    future: impl Future<Output = Result<T>> + Send + 'static,
    context: &'static str,
) -> Option<T>
where
    T: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| log_grok_auth_result(handle.block_on(future), context))
        }
        Ok(_) => join_grok_auth_thread(
            std::thread::spawn(move || {
                let runtime = build_grok_auth_runtime()?;
                log_grok_auth_result(runtime.block_on(future), context)
            })
            .join(),
        ),
        Err(_) => {
            let runtime = build_grok_auth_runtime()?;
            log_grok_auth_result(runtime.block_on(future), context)
        }
    }
}

fn build_grok_auth_runtime() -> Option<tokio::runtime::Runtime> {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            tracing::warn!(error = %error, "Failed to build temporary runtime for Grok auth");
            None
        }
    }
}

fn join_grok_auth_thread<T>(join_result: std::thread::Result<Option<T>>) -> Option<T> {
    match join_result {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!("Blocking Grok auth thread panicked");
            None
        }
    }
}

fn log_grok_auth_result<T>(result: Result<T>, context: &'static str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::debug!(context, error = %error, "Grok auth unavailable");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::join_grok_auth_thread;

    #[test]
    fn join_grok_auth_thread_returns_inner_result() {
        let result = join_grok_auth_thread(std::thread::spawn(|| Some(7)).join());

        assert_eq!(result, Some(7));
    }

    #[test]
    fn join_grok_auth_thread_returns_none_on_panic() {
        let result = join_grok_auth_thread(
            std::thread::spawn(|| -> Option<i32> {
                panic!("boom");
            })
            .join(),
        );

        assert!(result.is_none());
    }
}
