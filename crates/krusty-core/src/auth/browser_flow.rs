//! Browser-based OAuth flow
//!
//! Implements the authorization code flow with PKCE:
//! 1. Generate PKCE verifier and challenge
//! 2. Start local HTTP server for callback
//! 3. Open browser to authorization URL
//! 4. Wait for callback with authorization code
//! 5. Exchange code for tokens

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use url::Url;

use super::extract_openai_account_id;
use super::pkce::PkceVerifier;
use super::types::{OAuthConfig, OAuthTokenData};

/// Default port for the local OAuth callback server (matches Codex CLI)
pub const DEFAULT_CALLBACK_PORT: u16 = 1455;

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

            // Add any provider-specific extra parameters
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

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(anyhow!("Token exchange failed ({}): {}", status, body));
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("Failed to parse token response")?;

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
    ///
    /// This will:
    /// 1. Start a local HTTP server
    /// 2. Open the browser to the authorization URL
    /// 3. Wait for the callback with the authorization code
    /// 4. Exchange the code for tokens
    pub async fn run(&self) -> Result<OAuthTokenData> {
        let verifier = PkceVerifier::new();
        let state = generate_state();

        // Build the authorization URL
        let auth_url = self.build_auth_url(&verifier, &state)?;

        // Channel to receive the authorization code
        let (tx, rx) = mpsc::channel::<CallbackResult>();

        // Start the callback server in a separate thread
        let port = self.port;
        let expected_state = state.clone();
        let server_handle = thread::spawn(move || {
            run_callback_server(port, expected_state, tx);
        });

        // Give the server a moment to start
        thread::sleep(Duration::from_millis(100));

        // Open the browser
        open_browser(auth_url.as_str())?;

        // Wait for the callback (with timeout)
        let callback_result = rx
            .recv_timeout(Duration::from_secs(300))
            .context("OAuth callback timeout - no response received within 5 minutes")?;

        // Wait for server thread to finish
        let _ = server_handle.join();

        // Handle the callback result
        match callback_result {
            CallbackResult::Success { code } => self.exchange_code(&code, &verifier).await,
            CallbackResult::Error { error, description } => {
                Err(anyhow!("OAuth error: {} - {}", error, description))
            }
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
///
/// Instead of starting a local server, this flow:
/// 1. Builds the auth URL with PKCE challenge
/// 2. Opens the browser to the auth URL
/// 3. User completes auth and receives a code
/// 4. User pastes the code back into the TUI
/// 5. Code is exchanged for tokens
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
    ///
    /// The caller opens the browser and waits for the user to paste the code.
    pub fn get_auth_url(&self) -> Result<(String, PkceVerifier, String)> {
        let verifier = PkceVerifier::new();
        let state = generate_state();
        let url = self.build_auth_url(&verifier, &state)?;
        Ok((url.to_string(), verifier, state))
    }

    /// Exchange the pasted authorization code for tokens
    ///
    /// Anthropic uses JSON body (not form-encoded) for token exchange,
    /// and requires the `anthropic-beta` header.
    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &PkceVerifier,
    ) -> Result<OAuthTokenData> {
        let client = reqwest::Client::new();

        let body = serde_json::json!({
            "grant_type": "authorization_code",
            "client_id": self.config.client_id,
            "code": code,
            "redirect_uri": self.redirect_uri(),
            "code_verifier": verifier.as_str(),
        });

        let response = client
            .post(&self.config.token_url)
            .header("anthropic-beta", "oauth-2025-04-20")
            .json(&body)
            .send()
            .await
            .context("Failed to send Anthropic token request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(anyhow!(
                "Anthropic token exchange failed ({}): {}",
                status,
                body
            ));
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("Failed to parse Anthropic token response")?;

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

/// Token response from the OAuth server
#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: Option<String>,
}

/// Result from the OAuth callback
pub enum CallbackResult {
    Success { code: String },
    Error { error: String, description: String },
}

/// Generate a random state parameter for CSRF protection
fn generate_state() -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use rand::RngCore;

    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Parse query parameters from a URL path
fn parse_query_params(path: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(query) = path.split('?').nth(1) {
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            params.insert(key.to_string(), value.to_string());
        }
    }
    params
}

/// Run the local callback server
pub fn run_callback_server(port: u16, expected_state: String, tx: mpsc::Sender<CallbackResult>) {
    let addr = format!("127.0.0.1:{}", port);
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(CallbackResult::Error {
                error: "server_error".to_string(),
                description: format!("Failed to start callback server: {}", e),
            });
            return;
        }
    };

    // Wait for exactly one request
    if let Some(request) = server.recv_timeout(Duration::from_secs(300)).ok().flatten() {
        let path = request.url().to_string();
        let params = parse_query_params(&path);

        // Check state parameter
        let state = params.get("state").map(|s| s.as_str()).unwrap_or("");
        if state != expected_state {
            let _ = tx.send(CallbackResult::Error {
                error: "state_mismatch".to_string(),
                description: "State parameter does not match".to_string(),
            });
            respond_with_error(request, "State mismatch - possible CSRF attack");
            return;
        }

        // Check for error
        if let Some(error) = params.get("error") {
            let description = params
                .get("error_description")
                .map(|s| s.as_str())
                .unwrap_or("Unknown error");
            let _ = tx.send(CallbackResult::Error {
                error: error.clone(),
                description: description.to_string(),
            });
            respond_with_error(request, description);
            return;
        }

        // Get the authorization code
        if let Some(code) = params.get("code") {
            let _ = tx.send(CallbackResult::Success { code: code.clone() });
            respond_with_success(request);
        } else {
            let _ = tx.send(CallbackResult::Error {
                error: "missing_code".to_string(),
                description: "No authorization code received".to_string(),
            });
            respond_with_error(request, "No authorization code received");
        }
    }
}

/// Send a success response to the browser (consumes the request)
fn respond_with_success(request: tiny_http::Request) {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>Authentication Successful</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
        }
        .container {
            text-align: center;
            padding: 2rem;
            background: rgba(255, 255, 255, 0.1);
            border-radius: 1rem;
            backdrop-filter: blur(10px);
        }
        h1 { font-size: 2rem; margin-bottom: 1rem; }
        p { opacity: 0.9; }
        .checkmark {
            font-size: 4rem;
            margin-bottom: 1rem;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="checkmark">✓</div>
        <h1>Authentication Successful!</h1>
        <p>You can close this window and return to Krusty.</p>
    </div>
</body>
</html>"#;

    let response = tiny_http::Response::from_string(html)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                .unwrap(),
        )
        .with_status_code(200);

    let _ = request.respond(response);
}

/// Send an error response to the browser (consumes the request)
fn respond_with_error(request: tiny_http::Request, message: &str) {
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Authentication Failed</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #e74c3c 0%, #c0392b 100%);
            color: white;
        }}
        .container {{
            text-align: center;
            padding: 2rem;
            background: rgba(255, 255, 255, 0.1);
            border-radius: 1rem;
            backdrop-filter: blur(10px);
        }}
        h1 {{ font-size: 2rem; margin-bottom: 1rem; }}
        p {{ opacity: 0.9; }}
        .error-icon {{
            font-size: 4rem;
            margin-bottom: 1rem;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="error-icon">✗</div>
        <h1>Authentication Failed</h1>
        <p>{}</p>
        <p>Please close this window and try again.</p>
    </div>
</body>
</html>"#,
        html_escape(message)
    );

    let response = tiny_http::Response::from_string(html)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                .unwrap(),
        )
        .with_status_code(400);

    let _ = request.respond(response);
}

/// Simple HTML escape
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Open a URL in the default browser
pub fn open_browser(url: &str) -> Result<()> {
    use std::process::Stdio;

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to open browser with xdg-open")?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to open browser with open")?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to open browser")?;
    }

    Ok(())
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
    fn test_callback_url() {
        let flow = BrowserOAuthFlow::new(test_config());
        assert_eq!(flow.callback_url(), "http://localhost:1455/auth/callback");

        let custom_flow = BrowserOAuthFlow::with_port(test_config(), 8080);
        assert_eq!(
            custom_flow.callback_url(),
            "http://localhost:8080/auth/callback"
        );
    }

    #[test]
    fn test_build_auth_url() {
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
    fn test_parse_query_params() {
        let params = parse_query_params("/callback?code=abc123&state=xyz789");
        assert_eq!(params.get("code"), Some(&"abc123".to_string()));
        assert_eq!(params.get("state"), Some(&"xyz789".to_string()));
    }

    #[test]
    fn test_generate_state() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2, "State should be random");
        assert!(s1.len() >= 32, "State should be sufficiently long");
    }
}
