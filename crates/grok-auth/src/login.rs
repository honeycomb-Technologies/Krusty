//! The "whole flow" orchestrator.
//! This is the part that lets you do `grok login` style behavior from any Rust harness.

use crate::config::AuthConfig;
use crate::error::{AuthError, Result};
use crate::oidc;
use crate::token::AuthEntry;
use chrono::Utc;
use std::collections::HashMap;
use std::io::{BufRead, Read};
use std::process::{Command, Stdio};
use tokio::net::TcpListener;
use tracing::{info, warn};

/// High-level entry point for interactive or semi-interactive login.
/// Respects the same precedence and external provider contract as the official CLI.
pub async fn run_interactive_login(config: &AuthConfig) -> Result<AuthEntry> {
    // 1. External provider takes highest precedence if configured
    if let Some(cmd) = &config.auth_provider_command {
        info!("using external auth provider: {}", cmd);
        return run_external_provider(cmd, config.auth_provider_label.as_deref(), false).await;
    }

    // 2. Direct API key (user can still "login" to persist it in auth.json style)
    if let Some(key) = &config.api_key {
        info!("persisting API key as auth entry");
        return Ok(AuthEntry {
            access_token: key.clone(),
            auth_mode: Some("api_key".to_string()),
            create_time: Some(Utc::now()),
            ..Default::default()
        });
    }

    // 3. OIDC (browser or device)
    let issuer = config.oidc_issuer.as_deref().unwrap_or("https://auth.x.ai");
    let client_id = config
        .oidc_client_id
        .as_deref()
        .ok_or_else(|| AuthError::Config("oidc_client_id required for OIDC login".into()))?;

    let discovery = oidc::discover(issuer).await?;

    // Prefer device code if browser is disabled (simple heuristic; a real harness can be smarter)
    let use_device = !config.allow_browser;

    if use_device {
        if let Some(dev_ep) = &discovery.device_authorization_endpoint {
            let token_endpoint =
                discovery
                    .token_endpoint
                    .as_deref()
                    .ok_or_else(|| AuthError::DiscoveryFailed {
                        issuer: issuer.to_string(),
                        msg: "no token_endpoint in discovery".into(),
                    })?;
            return run_device_code_flow(dev_ep, token_endpoint, client_id, &config.oidc_scopes)
                .await;
        }
    }

    // Browser PKCE flow (the common happy path)
    if let (Some(auth_ep), Some(token_ep)) =
        (&discovery.authorization_endpoint, &discovery.token_endpoint)
    {
        return run_browser_pkce_flow(issuer, client_id, auth_ep, token_ep, &config.oidc_scopes)
            .await;
    }

    Err(AuthError::NoCredentials)
}

/// Run an external auth provider exactly as documented in the official 02-authentication.md.
/// stdout = token (or JSON with access_token + optional refresh_token + expires_in)
/// stderr = human messages (we surface the first https:// URL)
async fn run_external_provider(
    command: &str,
    label: Option<&str>,
    is_refresh: bool,
) -> Result<AuthEntry> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    if is_refresh {
        cmd.env("GROK_AUTH_EXPIRED", "1");
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn()?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AuthError::ExternalProvider("no stderr".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AuthError::ExternalProvider("no stdout".into()))?;

    // Surface stderr to the user (important for login URLs)
    let stderr_reader = std::io::BufReader::new(stderr);
    for l in stderr_reader.lines().map_while(std::result::Result::ok) {
        eprintln!("{}", l);
        if l.contains("https://") && label.is_some() {
            // In a real TUI you would turn the first URL into a clickable link.
        }
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(AuthError::ExternalProvider(format!(
            "provider exited with {}",
            status
        )));
    }

    let mut out = String::new();
    std::io::BufReader::new(stdout).read_to_string(&mut out)?;
    let out = out.trim();

    if out.is_empty() {
        return Err(AuthError::ExternalProvider(
            "provider printed no token to stdout".into(),
        ));
    }

    // Accept bare token or JSON
    if out.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(out)?;
        let access = v["access_token"].as_str().unwrap_or(out).to_string();
        let refresh = v["refresh_token"].as_str().map(|s| s.to_string());
        let expires_in = v["expires_in"].as_i64().unwrap_or(3600);

        Ok(AuthEntry {
            access_token: access,
            refresh_token: refresh,
            expires_at: Some(Utc::now() + chrono::Duration::seconds(expires_in)),
            auth_mode: Some("external".to_string()),
            create_time: Some(Utc::now()),
            ..Default::default()
        })
    } else {
        Ok(AuthEntry {
            access_token: out.to_string(),
            auth_mode: Some("external".to_string()),
            create_time: Some(Utc::now()),
            ..Default::default()
        })
    }
}

async fn run_device_code_flow(
    device_ep: &str,
    token_ep: &str,
    client_id: &str,
    scopes: &[String],
) -> Result<AuthEntry> {
    let start = oidc::start_device_code(device_ep, client_id, scopes).await?;
    let user_code = start["user_code"].as_str().unwrap_or_default().to_string();
    let verification_uri = start["verification_uri"].as_str().unwrap_or_default();
    let device_code = start["device_code"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let interval = start["interval"].as_u64().unwrap_or(5);

    println!(
        "\nTo sign in, open: {}\nAnd enter code: {}\n",
        verification_uri, user_code
    );

    if let Some(tokens) =
        oidc::poll_device_token(token_ep, client_id, &device_code, interval).await?
    {
        return token_response_to_entry(tokens, "device");
    }

    Err(AuthError::DeviceCode("device code flow timed out".into()))
}

async fn run_browser_pkce_flow(
    issuer: &str,
    client_id: &str,
    auth_endpoint: &str,
    token_endpoint: &str,
    scopes: &[String],
) -> Result<AuthEntry> {
    let verifier = oidc::generate_pkce_verifier();
    let challenge = oidc::pkce_challenge(&verifier);
    let state = format!("{:016x}", rand::random::<u64>());

    // Random high port loopback (official client does the same)
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_addr = listener.local_addr()?;
    let redirect_uri = format!("http://127.0.0.1:{}/callback", local_addr.port());

    let auth_url = oidc::build_auth_url(
        auth_endpoint,
        client_id,
        &redirect_uri,
        scopes,
        &state,
        &challenge,
    )?;

    println!("Opening browser for login: {}", auth_url);
    if let Err(e) = webbrowser::open(auth_url.as_str()) {
        warn!("failed to auto-open browser: {}", e);
        println!("Please open this URL manually:\n{}\n", auth_url);
    }

    // Wait for the callback (very small axum server)
    let code = wait_for_code_on_localhost(listener, &state).await?;

    let tokens =
        oidc::exchange_code(token_endpoint, client_id, &code, &redirect_uri, &verifier).await?;

    let mut entry = token_response_to_entry(tokens, "oidc")?;
    entry.oidc_issuer = Some(issuer.to_string());
    entry.oidc_client_id = Some(client_id.to_string());
    Ok(entry)
}

/// Tiny one-shot server that captures ?code=...&state=...
async fn wait_for_code_on_localhost(listener: TcpListener, expected_state: &str) -> Result<String> {
    use axum::{extract::Query, response::Html, routing::get, Router};
    use std::sync::{Arc, Mutex};
    use tokio::sync::oneshot;

    let (tx, rx) = oneshot::channel::<String>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let expected_state = Arc::new(expected_state.to_string());

    let app = Router::new().route(
        "/callback",
        get(move |Query(params): Query<HashMap<String, String>>| {
            let tx = Arc::clone(&tx);
            let expected_state = Arc::clone(&expected_state);
            async move {
                if let Some(code) = params.get("code") {
                    let state = params.get("state").map(|s| s.as_str()).unwrap_or("");
                    if state == expected_state.as_str() {
                        if let Some(tx) = tx.lock().expect("callback sender lock poisoned").take() {
                            let _ = tx.send(code.clone());
                        }
                        return Html("<h1>Login successful. You can close this window.</h1>");
                    }
                }
                Html("<h1>Login failed (state mismatch).</h1>")
            }
        }),
    );

    let server = axum::serve(listener, app);

    tokio::select! {
        res = server => {
            res.map_err(|e| AuthError::CallbackServer(e.to_string()))?;
            Err(AuthError::CallbackServer("server exited early".into()))
        }
        code = rx => {
            // best effort shutdown not implemented for brevity in this version
            code.map_err(|_| AuthError::CallbackServer("callback channel closed".into()))
        }
    }
}

fn token_response_to_entry(v: serde_json::Value, mode: &str) -> Result<AuthEntry> {
    let access = v["access_token"]
        .as_str()
        .ok_or_else(|| AuthError::CodeExchangeFailed("no access_token".into()))?
        .to_string();

    let refresh = v["refresh_token"].as_str().map(|s| s.to_string());
    let expires_in = v["expires_in"].as_i64().unwrap_or(3600);

    Ok(AuthEntry {
        access_token: access,
        refresh_token: refresh,
        expires_at: Some(Utc::now() + chrono::Duration::seconds(expires_in)),
        auth_mode: Some(mode.to_string()),
        create_time: Some(Utc::now()),
        ..Default::default()
    })
}
