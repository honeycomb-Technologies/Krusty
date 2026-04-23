use anyhow::{Context, Result};
use serde::Deserialize;

use super::credential_loading::extract_openai_account_id;
use super::providers::{anthropic_oauth_config, openai_oauth_config};
use super::{OAuthTokenData, OAuthTokenStore};
use crate::ai::providers::ProviderId;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// Refresh an expired OAuth token using the stored refresh token
pub async fn refresh_oauth_token(provider_id: ProviderId) -> Result<OAuthTokenData> {
    match provider_id {
        ProviderId::Anthropic => refresh_anthropic_oauth_token().await,
        _ => refresh_openai_oauth_token(provider_id).await,
    }
}

async fn refresh_openai_oauth_token(provider_id: ProviderId) -> Result<OAuthTokenData> {
    let oauth_store = OAuthTokenStore::load().context("Failed to load OAuth token store")?;
    let token = oauth_store
        .get(&provider_id)
        .context("No OAuth token stored for provider")?
        .clone();
    let refresh_token = token
        .refresh_token
        .as_ref()
        .context("No refresh token available")?;

    let config = openai_oauth_config();

    let client = reqwest::Client::new();
    let response = client
        .post(&config.token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &config.client_id),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .context("Failed to send token refresh request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Token refresh failed ({}): {}", status, body);
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .context("Failed to parse token refresh response")?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let account_id = extract_openai_account_id(&token_response.access_token)
        .or_else(|| {
            token_response
                .id_token
                .as_deref()
                .and_then(extract_openai_account_id)
        })
        .or(token.account_id.clone());

    let refreshed = OAuthTokenData {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token.or(token.refresh_token.clone()),
        id_token: token_response.id_token.or(token.id_token),
        expires_at: token_response.expires_in.map(|secs| now + secs),
        last_refresh: now,
        account_id,
    };

    let mut store = OAuthTokenStore::load().context("Failed to reload OAuth token store")?;
    store.set(provider_id, refreshed.clone());
    store
        .save()
        .context("Failed to save refreshed OAuth token")?;

    tracing::info!("Successfully refreshed OAuth token for {}", provider_id);
    Ok(refreshed)
}

/// Refresh an Anthropic OAuth token
///
/// Anthropic uses JSON body (not form-encoded) for token requests.
async fn refresh_anthropic_oauth_token() -> Result<OAuthTokenData> {
    let oauth_store = OAuthTokenStore::load().context("Failed to load OAuth token store")?;
    let token = oauth_store
        .get(&ProviderId::Anthropic)
        .context("No OAuth token stored for Anthropic")?
        .clone();
    let refresh_token = token
        .refresh_token
        .as_ref()
        .context("No refresh token available")?;

    let config = anthropic_oauth_config();

    let client = reqwest::Client::new();
    let response = client
        .post(&config.token_url)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": config.client_id,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .context("Failed to send Anthropic token refresh request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic token refresh failed ({}): {}", status, body);
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .context("Failed to parse Anthropic token refresh response")?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let refreshed = OAuthTokenData {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token.or(token.refresh_token.clone()),
        id_token: token_response.id_token.or(token.id_token),
        expires_at: token_response.expires_in.map(|secs| now + secs),
        last_refresh: now,
        account_id: token.account_id.clone(),
    };

    let mut store = OAuthTokenStore::load().context("Failed to reload OAuth token store")?;
    store.set(ProviderId::Anthropic, refreshed.clone());
    store
        .save()
        .context("Failed to save refreshed Anthropic OAuth token")?;

    tracing::info!("Successfully refreshed Anthropic OAuth token");
    Ok(refreshed)
}

fn log_refresh_failure(provider_id: ProviderId, context: &'static str, error: &anyhow::Error) {
    tracing::warn!(provider = %provider_id, context, error = %error, "OAuth token refresh failed");
}

fn refresh_with_runtime(
    provider_id: ProviderId,
    runtime: tokio::runtime::Runtime,
    context: &'static str,
) -> Option<OAuthTokenData> {
    match runtime.block_on(refresh_oauth_token(provider_id)) {
        Ok(token) => Some(token),
        Err(error) => {
            log_refresh_failure(provider_id, context, &error);
            None
        }
    }
}

fn build_blocking_refresh_runtime(provider_id: ProviderId) -> Option<tokio::runtime::Runtime> {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            tracing::warn!(
                provider = %provider_id,
                error = %error,
                "Failed to build temporary runtime for blocking OAuth refresh"
            );
            None
        }
    }
}

fn join_refresh_thread(
    provider_id: ProviderId,
    join_result: std::thread::Result<Option<OAuthTokenData>>,
) -> Option<OAuthTokenData> {
    match join_result {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!(
                provider = %provider_id,
                "Blocking OAuth refresh thread panicked"
            );
            None
        }
    }
}

/// Sync wrapper for refreshing an OAuth token from non-async code paths
pub fn try_refresh_oauth_token_blocking(provider_id: ProviderId) -> Option<OAuthTokenData> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| {
                match handle.block_on(refresh_oauth_token(provider_id)) {
                    Ok(token) => Some(token),
                    Err(error) => {
                        log_refresh_failure(
                            provider_id,
                            "refreshing OAuth token on current multithread runtime",
                            &error,
                        );
                        None
                    }
                }
            })
        }
        Ok(_) => join_refresh_thread(
            provider_id,
            std::thread::spawn(move || {
                let runtime = build_blocking_refresh_runtime(provider_id)?;
                refresh_with_runtime(
                    provider_id,
                    runtime,
                    "refreshing OAuth token on worker thread from current-thread runtime",
                )
            })
            .join(),
        ),
        Err(_) => {
            let runtime = build_blocking_refresh_runtime(provider_id)?;
            refresh_with_runtime(
                provider_id,
                runtime,
                "refreshing OAuth token without an active runtime",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::join_refresh_thread;
    use crate::ai::providers::ProviderId;

    #[test]
    fn join_refresh_thread_returns_inner_result() {
        let result = join_refresh_thread(
            ProviderId::OpenAI,
            std::thread::spawn(|| None::<crate::auth::OAuthTokenData>).join(),
        );

        assert!(result.is_none());
    }

    #[test]
    fn join_refresh_thread_returns_none_on_panic() {
        let result = join_refresh_thread(
            ProviderId::OpenAI,
            std::thread::spawn(|| -> Option<crate::auth::OAuthTokenData> {
                panic!("boom");
            })
            .join(),
        );

        assert!(result.is_none());
    }
}
