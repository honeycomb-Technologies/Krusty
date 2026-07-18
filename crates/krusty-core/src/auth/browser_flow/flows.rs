use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use url::Url;

use super::callback_server::{open_browser, run_callback_server, CallbackResult};
use super::DEFAULT_CALLBACK_PORT;
use crate::auth::extract_openai_account_id;
use crate::auth::http::{read_auth_response, safe_oauth_callback_error};
use crate::auth::pkce::PkceVerifier;
use crate::auth::types::{OAuthConfig, OAuthTokenData};

/// Browser-based OAuth flow handler
pub struct BrowserOAuthFlow {
    config: OAuthConfig,
    port: u16,
}

impl BrowserOAuthFlow {
    /// Create a new browser OAuth flow handler
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            port: DEFAULT_CALLBACK_PORT,
        }
    }

    /// Create with a custom port
    pub fn with_port(config: OAuthConfig, port: u16) -> Self {
        Self { config, port }
    }

    /// Get the callback URL for this flow
    pub fn callback_url(&self) -> String {
        format!("http://localhost:{}/auth/callback", self.port)
    }

    /// Build the authorization URL with all required parameters
    fn build_auth_url(&self, verifier: &PkceVerifier, state: &str) -> Result<Url> {
        let challenge = verifier.challenge();

        let mut url = Url::parse(&self.config.authorization_url)
            .context("Failed to parse authorization URL")?;

        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("response_type", "code")
                .append_pair("client_id", &self.config.client_id)
                .append_pair("redirect_uri", &self.callback_url())
                .append_pair("scope", &self.config.scopes.join(" "))
                .append_pair("state", state)
                .append_pair("code_challenge", challenge.as_str())
                .append_pair("code_challenge_method", challenge.method());

            for (key, value) in &self.config.extra_auth_params {
                pairs.append_pair(key, value);
            }
        }

        Ok(url)
    }

    /// Exchange authorization code for tokens
    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &PkceVerifier,
    ) -> Result<OAuthTokenData> {
        let client = reqwest::Client::new();

        let params = [
            ("grant_type", "authorization_code"),
            ("client_id", &self.config.client_id),
            ("code", code),
            ("redirect_uri", &self.callback_url()),
            ("code_verifier", verifier.as_str()),
        ];

        let response = client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .context("Failed to send token request")?;
        let response = read_auth_response(response)
            .await
            .context("Failed to read token exchange response")?;

        if !response.status().is_success() {
            return Err(response.safe_error("Token exchange failed"));
        }

        let token_response: TokenResponse = response.parse_json("OAuth token response")?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let expires_at = token_response.expires_in.map(|secs| now + secs);

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
            expires_at,
            last_refresh: now,
            account_id,
        })
    }

    /// Run the browser OAuth flow
    pub async fn run(&self) -> Result<OAuthTokenData> {
        let verifier = PkceVerifier::new();
        let state = generate_state();
        let auth_url = self.build_auth_url(&verifier, &state)?;
        let (tx, rx) = mpsc::channel::<CallbackResult>();

        let port = self.port;
        let expected_state = state.clone();
        let server_handle = thread::spawn(move || {
            run_callback_server(port, expected_state, tx);
        });

        thread::sleep(Duration::from_millis(100));
        open_browser(auth_url.as_str())?;

        let callback_result = rx
            .recv_timeout(Duration::from_secs(300))
            .context("OAuth callback timeout - no response received within 5 minutes")?;

        let _ = server_handle.join();

        match callback_result {
            CallbackResult::Success { code } => self.exchange_code(&code, &verifier).await,
            CallbackResult::Error { error, description } => Err(safe_oauth_callback_error(
                "OAuth callback failed",
                Some(&error),
                Some(&description),
            )),
        }
    }

    /// Get the authorization URL for manual use (e.g., displaying to user)
    pub fn get_auth_url(&self) -> Result<(String, PkceVerifier, String)> {
        let verifier = PkceVerifier::new();
        let state = generate_state();
        let url = self.build_auth_url(&verifier, &state)?;
        Ok((url.to_string(), verifier, state))
    }
}

/// Paste-code OAuth flow for providers without localhost redirect support (e.g., Anthropic)
pub struct PasteCodeOAuthFlow {
    config: OAuthConfig,
}

impl PasteCodeOAuthFlow {
    pub fn new(config: OAuthConfig) -> Self {
        Self { config }
    }

    /// Get the redirect URI for paste-code flow (Anthropic's code callback)
    fn redirect_uri(&self) -> &str {
        "https://console.anthropic.com/oauth/code/callback"
    }

    /// Build the authorization URL with PKCE challenge
    fn build_auth_url(&self, verifier: &PkceVerifier, state: &str) -> Result<Url> {
        let challenge = verifier.challenge();

        let mut url = Url::parse(&self.config.authorization_url)
            .context("Failed to parse authorization URL")?;

        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("response_type", "code")
                .append_pair("client_id", &self.config.client_id)
                .append_pair("redirect_uri", self.redirect_uri())
                .append_pair("scope", &self.config.scopes.join(" "))
                .append_pair("state", state)
                .append_pair("code_challenge", challenge.as_str())
                .append_pair("code_challenge_method", challenge.method());

            for (key, value) in &self.config.extra_auth_params {
                pairs.append_pair(key, value);
            }
        }

        Ok(url)
    }

    /// Get the authorization URL, verifier, and state for the paste-code flow
    pub fn get_auth_url(&self) -> Result<(String, PkceVerifier, String)> {
        let verifier = PkceVerifier::new();
        let state = generate_state();
        let url = self.build_auth_url(&verifier, &state)?;
        Ok((url.to_string(), verifier, state))
    }

    /// Exchange the pasted authorization code for tokens
    pub async fn exchange_code(
        &self,
        code: &str,
        state: Option<&str>,
        verifier: &PkceVerifier,
    ) -> Result<OAuthTokenData> {
        let client = reqwest::Client::new();

        let mut body = serde_json::json!({
            "grant_type": "authorization_code",
            "client_id": self.config.client_id,
            "code": code,
            "redirect_uri": self.redirect_uri(),
            "code_verifier": verifier.as_str(),
        });

        if let Some(state) = state {
            body.as_object_mut().expect("JSON object").insert(
                "state".to_string(),
                serde_json::Value::String(state.to_string()),
            );
        }

        let response = client
            .post(&self.config.token_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send Anthropic token request")?;
        let response = read_auth_response(response)
            .await
            .context("Failed to read Anthropic token response")?;

        if !response.status().is_success() {
            return Err(response.safe_error("Anthropic token exchange failed"));
        }

        let token_response: TokenResponse = response.parse_json("Anthropic token response")?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Ok(OAuthTokenData {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            id_token: token_response.id_token,
            expires_at: token_response.expires_in.map(|secs| now + secs),
            last_refresh: now,
            account_id: None,
        })
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
    #[serde(rename = "token_type", default)]
    _token_type: Option<String>,
}

fn generate_state() -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use rand::RngCore;

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
            client_id: "test-client".to_string(),
            authorization_url: "https://auth.example.com/authorize".to_string(),
            token_url: "https://auth.example.com/token".to_string(),
            device_auth_url: None,
            scopes: vec!["openid".to_string(), "profile".to_string()],
            refresh_days: 28,
            extra_auth_params: vec![],
        }
    }

    #[test]
    fn callback_url() {
        let flow = BrowserOAuthFlow::new(test_config());
        assert_eq!(flow.callback_url(), "http://localhost:1455/auth/callback");

        let custom_flow = BrowserOAuthFlow::with_port(test_config(), 8080);
        assert_eq!(
            custom_flow.callback_url(),
            "http://localhost:8080/auth/callback"
        );
    }

    #[test]
    fn build_auth_url() {
        let flow = BrowserOAuthFlow::new(test_config());
        let verifier = PkceVerifier::new();
        let state = "test-state";

        let url = flow.build_auth_url(&verifier, state).unwrap();

        assert!(url
            .as_str()
            .starts_with("https://auth.example.com/authorize"));
        assert!(url.as_str().contains("response_type=code"));
        assert!(url.as_str().contains("client_id=test-client"));
        assert!(url.as_str().contains("state=test-state"));
        assert!(url.as_str().contains("code_challenge_method=S256"));
    }

    #[test]
    fn generate_state_is_random() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2, "State should be random");
        assert!(s1.len() >= 32, "State should be sufficiently long");
    }
}
