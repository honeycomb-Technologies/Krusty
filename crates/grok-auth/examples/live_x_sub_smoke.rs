//! Live X-subscription smoke test for grok-auth.
//!
//! This intentionally requires `GROK_AUTH_LIVE=1` because it uses a real Grok
//! account/session and may consume quota. It never prints access/refresh tokens.
//!
//! Reuse existing `~/.grok/auth.json`:
//!   GROK_AUTH_LIVE=1 cargo run --example live_x_sub_smoke
//!
//! Force a fresh browser OIDC login into a temporary auth file:
//!   GROK_AUTH_LIVE=1 GROK_AUTH_FORCE_LOGIN=1 \
//!   GROK_AUTH_SMOKE_AUTH_FILE=/tmp/grok-auth-live.json \
//!   cargo run --example live_x_sub_smoke

use grok_auth::{AuthConfig, AuthEntry, AuthStore, ClientBuilder, Result};
use serde_json::json;
use std::{collections::HashMap, env, fs, path::Path};

#[tokio::main]
async fn main() -> Result<()> {
    if env::var("GROK_AUTH_LIVE").as_deref() != Ok("1") {
        return Err(grok_auth::AuthError::Config(
            "set GROK_AUTH_LIVE=1 to run the live X-subscription smoke test".into(),
        ));
    }

    let mut cfg = AuthConfig::from_env()?;

    if let Ok(path) = env::var("GROK_AUTH_SMOKE_AUTH_FILE") {
        cfg.auth_file = path.into();
    }

    infer_oidc_config_from_auth_file(&mut cfg);

    let force_login = env::var("GROK_AUTH_FORCE_LOGIN").as_deref() == Ok("1");
    let store = AuthStore::new(cfg.auth_file.clone(), cfg.clone()).await?;
    let token = if force_login {
        println!("Starting fresh browser OIDC login; complete it in the browser window...");
        store.force_login().await?
    } else {
        store.ensure_fresh().await?
    };

    if token.issuer_key == "api_key" {
        return Err(grok_auth::AuthError::Config(
            "live smoke requires X-sub/OIDC auth, not XAI_API_KEY".into(),
        ));
    }

    println!("Authenticated via issuer key: {}", token.issuer_key);
    println!("Token expires at: {:?}", token.expires_at);

    let client = ClientBuilder::new()
        .with_token(token)
        .with_client_version(&cfg.client_version)
        .with_header("x-grok-client-identifier", "mitsuro-grok-auth-smoke")
        .with_header("X-XAI-Token-Auth", "xai-grok-cli")
        .build()?;

    let base = env::var("GROK_AUTH_SMOKE_BASE_URL")
        .or_else(|_| env::var("GROK_CLI_CHAT_PROXY_BASE_URL"))
        .unwrap_or_else(|_| "https://cli-chat-proxy.grok.com/v1".to_string());
    let model = env::var("GROK_AUTH_SMOKE_MODEL").unwrap_or_else(|_| "grok-build".to_string());

    let body = json!({
        "model": model,
        "input": [{
            "role": "user",
            "content": "Reply with exactly: grok-auth live smoke ok"
        }],
        "max_output_tokens": 40,
    });

    let endpoint_base = base
        .trim_end_matches('/')
        .trim_end_matches("/chat/completions")
        .trim_end_matches("/responses");
    let resp = client
        .inner()
        .post(format!("{endpoint_base}/responses"))
        .header("x-grok-model-override", &model)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await?;
    println!("Responses status: {status}");

    if !status.is_success() {
        println!("Response body (truncated): {}", truncate(&text, 1000));
        return Err(grok_auth::AuthError::Http(format!(
            "live Responses API call failed with {status}"
        )));
    }

    let value: serde_json::Value = serde_json::from_str(&text)?;
    let response_model = value
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>");
    let content = extract_response_text(&value).unwrap_or("<missing>");
    let total_tokens = value
        .pointer("/usage/total_tokens")
        .and_then(|v| v.as_i64());

    println!("Response model: {response_model}");
    println!("Assistant content: {}", truncate(content, 300));
    println!("Total tokens: {:?}", total_tokens);

    if content == "<missing>" {
        return Err(grok_auth::AuthError::Json(
            "successful response did not include assistant content".into(),
        ));
    }

    Ok(())
}

fn infer_oidc_config_from_auth_file(cfg: &mut AuthConfig) {
    if cfg.oidc_issuer.is_some() && cfg.oidc_client_id.is_some() {
        return;
    }

    let hint_path = env::var("GROK_AUTH_OIDC_HINT_FILE")
        .map(Into::into)
        .unwrap_or_else(|_| grok_auth::default_auth_path());

    if !Path::new(&hint_path).exists() {
        return;
    }

    let Ok(contents) = fs::read_to_string(hint_path) else {
        return;
    };
    let Ok(entries) = serde_json::from_str::<HashMap<String, AuthEntry>>(&contents) else {
        return;
    };

    for (key, entry) in entries {
        if cfg.oidc_issuer.is_none() {
            cfg.oidc_issuer = entry
                .oidc_issuer
                .or_else(|| key.split("::").next().map(str::to_string));
        }
        if cfg.oidc_client_id.is_none() {
            cfg.oidc_client_id = entry.oidc_client_id.or_else(|| {
                key.split_once("::")
                    .map(|(_, client_id)| client_id.to_string())
            });
        }
        if cfg.oidc_issuer.is_some() && cfg.oidc_client_id.is_some() {
            return;
        }
    }
}

fn extract_response_text(value: &serde_json::Value) -> Option<&str> {
    value
        .get("output_text")
        .and_then(|v| v.as_str())
        .or_else(|| {
            value
                .get("output")
                .and_then(|v| v.as_array())
                .and_then(|items| {
                    items.iter().find_map(|item| {
                        item.get("content")
                            .and_then(|v| v.as_array())
                            .and_then(|parts| {
                                parts.iter().find_map(|part| {
                                    part.get("text")
                                        .or_else(|| part.get("content"))
                                        .and_then(|v| v.as_str())
                                })
                            })
                    })
                })
        })
        .or_else(|| {
            value
                .pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
        })
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
