use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

use super::credential_loading::extract_openai_account_id;
use super::providers::{anthropic_oauth_config, openai_oauth_config};
use super::{refresh_grok_oauth_token, OAuthTokenData, OAuthTokenStore};
use crate::ai::providers::ProviderId;

const REFRESH_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const REFRESH_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(45);

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
    // Capture the generation before waiting so callers queued behind an
    // in-flight refresh can reuse its result instead of issuing a second
    // request with a rotated or rejected credential.
    let observed = load_provider_token(provider_id)?;
    let _refresh_lock = tokio::time::timeout(
        REFRESH_LOCK_TIMEOUT,
        OAuthTokenStore::lock_provider_refresh(provider_id),
    )
    .await
    .context("Timed out waiting for provider OAuth refresh lock")?
    .context("Failed to acquire provider OAuth refresh lock")?;
    let current = load_provider_token(provider_id)?;
    if credential_generation_changed(&observed, &current) {
        tracing::info!(
            provider = %provider_id,
            "OAuth credential changed while waiting for refresh lock; reusing stored result"
        );
        return Ok(current);
    }

    tokio::time::timeout(REFRESH_EXCHANGE_TIMEOUT, async {
        match provider_id {
            ProviderId::Anthropic => refresh_anthropic_oauth_token().await,
            ProviderId::Grok => refresh_grok_oauth_token().await,
            _ => refresh_openai_oauth_token(provider_id).await,
        }
    })
    .await
    .context("OAuth refresh exchange timed out")?
}

fn load_provider_token(provider_id: ProviderId) -> Result<OAuthTokenData> {
    OAuthTokenStore::load()
        .context("Failed to load OAuth token store")?
        .get(&provider_id)
        .cloned()
        .with_context(|| format!("No OAuth token stored for {provider_id}"))
}

fn credential_generation_changed(observed: &OAuthTokenData, current: &OAuthTokenData) -> bool {
    observed.access_token != current.access_token
        || observed.refresh_token != current.refresh_token
        || observed.expires_at != current.expires_at
        || observed.last_refresh != current.last_refresh
}

async fn refresh_openai_oauth_token(provider_id: ProviderId) -> Result<OAuthTokenData> {
    let oauth_store = OAuthTokenStore::load().context("Failed to load OAuth token store")?;
    let token = oauth_store
        .get(&provider_id)
        .context("No OAuth token stored for provider")?
        .clone();
    let refresh_token = token
        .refresh_token
        .clone()
        .context("No refresh token available")?;

    let config = openai_oauth_config();

    let client = reqwest::Client::new();
    let response = client
        .post(&config.token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &config.client_id),
            ("refresh_token", &refresh_token),
        ])
        .send()
        .await
        .context("Failed to send token refresh request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        invalidate_rejected_refresh_token(provider_id, status.as_u16(), &body, &refresh_token);
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

    let refreshed = persist_refreshed_token_if_current(
        provider_id,
        &refresh_token,
        refreshed,
        "Failed to save refreshed OAuth token",
    )?;

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
        .clone()
        .context("No refresh token available")?;

    let config = anthropic_oauth_config();

    let client = reqwest::Client::new();
    let response = client
        .post(&config.token_url)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": config.client_id,
            "refresh_token": &refresh_token,
        }))
        .send()
        .await
        .context("Failed to send Anthropic token refresh request")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        invalidate_rejected_refresh_token(
            ProviderId::Anthropic,
            status.as_u16(),
            &body,
            &refresh_token,
        );
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

    let refreshed = persist_refreshed_token_if_current(
        ProviderId::Anthropic,
        &refresh_token,
        refreshed,
        "Failed to save refreshed Anthropic OAuth token",
    )?;

    tracing::info!("Successfully refreshed Anthropic OAuth token");
    Ok(refreshed)
}

fn persist_refreshed_token_if_current(
    provider_id: ProviderId,
    request_refresh_token: &str,
    refreshed: OAuthTokenData,
    error_context: &'static str,
) -> Result<OAuthTokenData> {
    if OAuthTokenStore::replace_persisted_if_refresh_token_matches(
        &provider_id,
        request_refresh_token,
        refreshed.clone(),
    )
    .context(error_context)?
    {
        return Ok(refreshed);
    }

    let current = OAuthTokenStore::load()
        .context("Failed to reload concurrently refreshed OAuth token")?
        .get(&provider_id)
        .cloned()
        .context("OAuth credential changed during refresh but no replacement is stored")?;
    tracing::info!(
        provider = %provider_id,
        "OAuth credential changed concurrently; using the newer stored credential"
    );
    Ok(current)
}

fn refresh_token_is_rejected(status: u16, body: &str) -> bool {
    if !matches!(status, 400 | 401) {
        return false;
    }

    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(str::to_ascii_lowercase)
        })
        .is_some_and(|error| matches!(error.as_str(), "invalid_grant" | "invalid_token"))
}

fn invalidate_rejected_refresh_token(
    provider_id: ProviderId,
    status: u16,
    body: &str,
    rejected_refresh_token: &str,
) {
    if !refresh_token_is_rejected(status, body) {
        return;
    }

    let result = OAuthTokenStore::remove_persisted_if_refresh_token_matches(
        &provider_id,
        rejected_refresh_token,
    )
    .context("Failed to save OAuth token invalidation");

    match result {
        Ok(true) => tracing::warn!(
            provider = %provider_id,
            "OAuth refresh credential was rejected and has been cleared; reauthentication is required"
        ),
        Ok(false) => tracing::warn!(
            provider = %provider_id,
            "OAuth refresh credential was rejected, but the stored credential changed concurrently and was preserved"
        ),
        Err(error) => tracing::warn!(
            provider = %provider_id,
            error = %error,
            "OAuth refresh credential was rejected but could not be cleared"
        ),
    }
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
    use super::{credential_generation_changed, join_refresh_thread, refresh_token_is_rejected};
    use crate::ai::providers::ProviderId;
    use crate::auth::OAuthTokenData;

    fn token(access_token: &str, refresh_token: &str, last_refresh: u64) -> OAuthTokenData {
        OAuthTokenData {
            access_token: access_token.to_string(),
            refresh_token: Some(refresh_token.to_string()),
            id_token: None,
            expires_at: Some(last_refresh + 3600),
            last_refresh,
            account_id: None,
        }
    }

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

    #[test]
    fn invalid_grant_requires_reauthentication() {
        assert!(refresh_token_is_rejected(
            400,
            r#"{"error":"invalid_grant","error_description":"Refresh token expired"}"#,
        ));
        assert!(refresh_token_is_rejected(
            401,
            r#"{"error":"invalid_token"}"#,
        ));
    }

    #[test]
    fn transient_or_unrelated_refresh_errors_keep_the_credential() {
        assert!(!refresh_token_is_rejected(
            503,
            r#"{"error":"invalid_grant"}"#,
        ));
        assert!(!refresh_token_is_rejected(
            400,
            r#"{"error":"temporarily_unavailable"}"#,
        ));
        assert!(!refresh_token_is_rejected(400, "not json"));
    }

    #[test]
    fn queued_refresh_detects_an_already_persisted_result() {
        let observed = token("old-access", "old-refresh", 10);
        assert!(!credential_generation_changed(&observed, &observed));
        assert!(credential_generation_changed(
            &observed,
            &token("new-access", "new-refresh", 20)
        ));
        assert!(credential_generation_changed(
            &observed,
            &token("new-access", "old-refresh", 20)
        ));
    }
}
