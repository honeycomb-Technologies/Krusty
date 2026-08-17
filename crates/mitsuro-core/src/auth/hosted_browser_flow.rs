//! Hosted browser OAuth flow for web callbacks.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use url::Url;

use super::extract_openai_account_id;
use super::http::read_auth_response;
use super::pkce::PkceVerifier;
use super::types::{OAuthConfig, OAuthTokenData};

/// Browser-based OAuth flow that redirects back to a hosted callback URL.
pub struct HostedBrowserOAuthFlow {
    config: OAuthConfig,
    redirect_uri: String,
}

impl HostedBrowserOAuthFlow {
    pub fn new(config: OAuthConfig, redirect_uri: String) -> Self {
        Self {
            config,
            redirect_uri,
        }
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub fn get_auth_url(&self) -> Result<(String, PkceVerifier, String)> {
        let verifier = PkceVerifier::new();
        let state = generate_state();
        let url = self.build_auth_url(&verifier, &state)?;
        Ok((url.to_string(), verifier, state))
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &PkceVerifier,
    ) -> Result<OAuthTokenData> {
        let client = reqwest::Client::new();
        let params = [
            ("grant_type", "authorization_code"),
            ("client_id", self.config.client_id.as_str()),
            ("code", code),
            ("redirect_uri", self.redirect_uri()),
            ("code_verifier", verifier.as_str()),
        ];

        let response = client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .context("Failed to send OAuth token request")?;
        let response = read_auth_response(response)
            .await
            .context("Failed to read OAuth token response")?;

        if !response.status().is_success() {
            return Err(response.safe_error("Token exchange failed"));
        }

        let token_response: TokenResponse = response.parse_json("OAuth token response")?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
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

    fn build_auth_url(&self, verifier: &PkceVerifier, state: &str) -> Result<Url> {
        crate::auth::pkce::build_pkce_authorization_url(
            &self.config,
            self.redirect_uri(),
            verifier,
            state,
        )
    }
}

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::providers::ProviderId;

    fn test_config() -> OAuthConfig {
        OAuthConfig {
            provider_id: ProviderId::OpenAI,
            client_id: "client_123".to_string(),
            authorization_url: "https://auth.example.com/oauth/authorize".to_string(),
            token_url: "https://auth.example.com/oauth/token".to_string(),
            device_auth_url: None,
            scopes: vec!["openid".to_string(), "profile".to_string()],
            refresh_days: 28,
            extra_auth_params: vec![("originator".to_string(), "codex_cli_rs".to_string())],
        }
    }

    #[test]
    fn builds_auth_url_with_redirect_uri() {
        let flow = HostedBrowserOAuthFlow::new(
            test_config(),
            "https://example.ts.net/auth/oauth/callback/openai".to_string(),
        );

        let (auth_url, _verifier, state) = flow.get_auth_url().unwrap();
        let url = Url::parse(&auth_url).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

        assert_eq!(
            query.get("redirect_uri").unwrap(),
            "https://example.ts.net/auth/oauth/callback/openai"
        );
        assert_eq!(query.get("state").unwrap(), &state);
        assert_eq!(query.get("originator").unwrap(), "codex_cli_rs");
    }
}
