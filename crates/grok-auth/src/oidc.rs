//! OIDC / OAuth2 helpers (PKCE, discovery, device code, token exchange).
//! Kept separate so the flows are easy to audit and extend.

use crate::error::{AuthError, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use url::Url;

#[derive(Debug, Deserialize)]
pub struct OidcDiscovery {
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub device_authorization_endpoint: Option<String>,
    pub issuer: Option<String>,
}

pub async fn discover(issuer: &str) -> Result<OidcDiscovery> {
    let well_known = if issuer.ends_with('/') {
        format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        )
    } else {
        format!("{}/.well-known/openid-configuration", issuer)
    };

    let client = Client::new();
    let disc: OidcDiscovery = client
        .get(&well_known)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .map_err(|e| AuthError::DiscoveryFailed {
            issuer: issuer.to_string(),
            msg: e.to_string(),
        })?;
    Ok(disc)
}

/// Generate a cryptographically random PKCE code_verifier (43-128 chars).
pub fn generate_pkce_verifier() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

/// code_challenge = BASE64URL(SHA256(verifier)) without padding.
pub fn pkce_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let digest = hasher.finalize();
    URL_SAFE_NO_PAD.encode(digest)
}

/// Build the authorization URL for the browser/device flow.
pub fn build_auth_url(
    auth_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    state: &str,
    code_challenge: &str,
) -> Result<Url> {
    let mut url = Url::parse(auth_endpoint)?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", &scopes.join(" "))
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url)
}

/// Exchange authorization code + PKCE verifier for tokens.
pub async fn exchange_code(
    token_endpoint: &str,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<serde_json::Value> {
    let client = Client::new();
    let mut params = HashMap::new();
    params.insert("grant_type", "authorization_code");
    params.insert("client_id", client_id);
    params.insert("code", code);
    params.insert("redirect_uri", redirect_uri);
    params.insert("code_verifier", code_verifier);

    let resp = client
        .post(token_endpoint)
        .form(&params)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    Ok(resp)
}

/// Device code flow start.
pub async fn start_device_code(
    device_endpoint: &str,
    client_id: &str,
    scopes: &[String],
) -> Result<serde_json::Value> {
    let client = Client::new();
    let scope = scopes.join(" ");
    let params = [("client_id", client_id), ("scope", scope.as_str())];

    let resp = client
        .post(device_endpoint)
        .form(&params)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    Ok(resp)
}

/// Poll for device code completion.
pub async fn poll_device_token(
    token_endpoint: &str,
    client_id: &str,
    device_code: &str,
    interval: u64,
) -> Result<Option<serde_json::Value>> {
    let client = Client::new();
    let mut params = HashMap::new();
    params.insert("grant_type", "urn:ietf:params:oauth:grant-type:device_code");
    params.insert("client_id", client_id);
    params.insert("device_code", device_code);

    // Simple poll loop (caller can control total timeout)
    for _ in 0..(60 / interval.max(1) + 5) {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        let resp = client.post(token_endpoint).form(&params).send().await?;

        if resp.status().is_success() {
            let val: serde_json::Value = resp.json().await?;
            if val.get("access_token").is_some() {
                return Ok(Some(val));
            }
        } else {
            // Usually "authorization_pending" or "slow_down"
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            if let Some(err_code) = err.get("error").and_then(|e| e.as_str()) {
                if err_code == "slow_down" {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    }
    Ok(None)
}
