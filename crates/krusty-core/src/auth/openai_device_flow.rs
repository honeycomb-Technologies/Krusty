//! OpenAI-specific device authorization flow.
//!
//! OpenAI's Codex-compatible device login is not a standard RFC 8628 flow.
//! It uses custom device-auth endpoints to mint an authorization code, then
//! completes a standard OAuth token exchange against `/oauth/token`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use super::extract_openai_account_id;
use super::types::{OAuthConfig, OAuthTokenData};

/// Response returned to the caller when starting the OpenAI device flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIDeviceCodeResponse {
    pub device_auth_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

pub struct OpenAIDeviceAuthFlow {
    config: OAuthConfig,
}

impl OpenAIDeviceAuthFlow {
    pub fn new(config: OAuthConfig) -> Self {
        Self { config }
    }

    pub async fn request_code(&self) -> Result<OpenAIDeviceCodeResponse> {
        let issuer = self.issuer_origin()?;
        let auth_base_url = format!("{issuer}/api/accounts/deviceauth");
        let client = reqwest::Client::new();

        let response = client
            .post(format!("{auth_base_url}/usercode"))
            .json(&serde_json::json!({
                "client_id": self.config.client_id,
            }))
            .send()
            .await
            .context("Failed to send OpenAI device code request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "OpenAI device code request failed ({}): {}",
                status,
                body
            ));
        }

        let user_code_response: UserCodeResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI device code response")?;

        let mut verification_uri_complete = Url::parse(&format!("{issuer}/codex/device"))
            .context("Failed to build OpenAI verification URL")?;
        verification_uri_complete
            .query_pairs_mut()
            .append_pair("user_code", &user_code_response.user_code);

        Ok(OpenAIDeviceCodeResponse {
            device_auth_id: user_code_response.device_auth_id,
            user_code: user_code_response.user_code,
            verification_uri: format!("{issuer}/codex/device"),
            verification_uri_complete: Some(verification_uri_complete.to_string()),
            expires_in: expires_in_from_rfc3339(&user_code_response.expires_at),
            interval: user_code_response.interval.parse().unwrap_or(5).max(1),
        })
    }

    pub async fn poll_for_token(
        &self,
        device_auth_id: &str,
        user_code: &str,
        interval: u64,
        expires_in: u64,
    ) -> Result<OAuthTokenData> {
        let issuer = self.issuer_origin()?;
        let auth_base_url = format!("{issuer}/api/accounts/deviceauth");
        let client = reqwest::Client::new();
        let poll_interval = Duration::from_secs(interval.max(1));
        let max_wait = Duration::from_secs(expires_in.max(60));
        let started_at = std::time::Instant::now();

        loop {
            let response = client
                .post(format!("{auth_base_url}/token"))
                .json(&serde_json::json!({
                    "device_auth_id": device_auth_id,
                    "user_code": user_code,
                }))
                .send()
                .await
                .context("Failed to poll OpenAI device auth token")?;

            let status = response.status();

            if status.is_success() {
                let code_response: DeviceAuthTokenResponse = response
                    .json()
                    .await
                    .context("Failed to parse OpenAI device auth token response")?;
                return self
                    .exchange_code(
                        &code_response.authorization_code,
                        &code_response.code_verifier,
                    )
                    .await;
            }

            if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND
            {
                if started_at.elapsed() >= max_wait {
                    return Err(anyhow!(
                        "OpenAI device authorization timed out after {} seconds",
                        max_wait.as_secs()
                    ));
                }

                tokio::time::sleep(poll_interval).await;
                continue;
            }

            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("OpenAI device auth failed ({}): {}", status, body));
        }
    }

    async fn exchange_code(&self, code: &str, code_verifier: &str) -> Result<OAuthTokenData> {
        let issuer = self.issuer_origin()?;
        let redirect_uri = format!("{issuer}/deviceauth/callback");
        let client = reqwest::Client::new();

        let response = client
            .post(&self.config.token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", self.config.client_id.as_str()),
                ("code", code),
                ("redirect_uri", redirect_uri.as_str()),
                ("code_verifier", code_verifier),
            ])
            .send()
            .await
            .context("Failed to exchange OpenAI device authorization code")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "OpenAI token exchange failed ({}): {}",
                status,
                body
            ));
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI token response")?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let account_id = extract_openai_account_id(&token_response.access_token).or_else(|| {
            token_response
                .id_token
                .as_deref()
                .and_then(extract_openai_account_id)
        });

        Ok(OAuthTokenData {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            id_token: token_response.id_token,
            expires_at: token_response.expires_in.map(|secs| now + secs),
            last_refresh: now,
            account_id,
        })
    }

    fn issuer_origin(&self) -> Result<String> {
        let parsed = Url::parse(&self.config.authorization_url)
            .context("Failed to parse OpenAI authorization URL")?;

        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("OpenAI authorization URL missing host"))?;

        let mut origin = format!("{}://{}", parsed.scheme(), host);
        if let Some(port) = parsed.port() {
            origin.push(':');
            origin.push_str(&port.to_string());
        }
        Ok(origin)
    }
}

#[derive(Debug, Deserialize)]
struct UserCodeResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    interval: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthTokenResponse {
    authorization_code: String,
    code_verifier: String,
    #[allow(dead_code)]
    code_challenge: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

fn expires_in_from_rfc3339(expires_at: &str) -> u64 {
    DateTime::parse_from_rfc3339(expires_at)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .and_then(|dt| (dt - Utc::now()).to_std().ok())
        .map(|duration| duration.as_secs().max(1))
        .unwrap_or(15 * 60)
}
