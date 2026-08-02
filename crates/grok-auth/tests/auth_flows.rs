use axum::{
    extract::Form,
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration, Utc};
use grok_auth::{authenticate, AuthConfig, AuthEntry, AuthStore, AuthToken, ClientBuilder};
use serde_json::json;
use std::{collections::HashMap, error::Error, fs};
use tempfile::tempdir;
use tokio::net::TcpListener;

async fn spawn_server(app: Router) -> Result<String, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            panic!("test server failed: {err}");
        }
    });
    Ok(format!("http://{addr}"))
}

#[tokio::test]
async fn api_key_fallback_persists_to_custom_auth_file() -> Result<(), Box<dyn Error>> {
    let dir = tempdir()?;
    let auth_file = dir.path().join("auth.json");
    let cfg = AuthConfig {
        auth_file: auth_file.clone(),
        api_key: Some("xai-test-key".to_string()),
        ..AuthConfig::default()
    };

    let store = AuthStore::new(auth_file.clone(), cfg).await?;
    let token = store.ensure_fresh().await?;

    assert_eq!(token.access_token, "xai-test-key");
    assert_eq!(token.issuer_key, "api_key");

    let saved: HashMap<String, AuthEntry> = serde_json::from_str(&fs::read_to_string(auth_file)?)?;
    let api_key_entry = saved
        .get("api_key")
        .expect("api_key entry should be persisted");
    assert_eq!(api_key_entry.access_token, "xai-test-key");
    assert_eq!(api_key_entry.auth_mode.as_deref(), Some("api_key"));

    Ok(())
}

#[tokio::test]
async fn load_prefers_configured_oidc_entry_from_disk() -> Result<(), Box<dyn Error>> {
    let dir = tempdir()?;
    let auth_file = dir.path().join("auth.json");
    let issuer = "https://issuer.example";
    let client_id = "mitsuro-client";
    let preferred_key = format!("{issuer}::{client_id}");

    let mut entries = HashMap::new();
    entries.insert(
        "https://auth.x.ai::other".to_string(),
        AuthEntry {
            access_token: "fallback-token".to_string(),
            refresh_token: Some("fallback-refresh".to_string()),
            ..AuthEntry::default()
        },
    );
    entries.insert(
        preferred_key.clone(),
        AuthEntry {
            access_token: "preferred-token".to_string(),
            expires_at: Some(Utc::now() + Duration::hours(1)),
            oidc_issuer: Some(issuer.to_string()),
            oidc_client_id: Some(client_id.to_string()),
            ..AuthEntry::default()
        },
    );
    fs::write(&auth_file, serde_json::to_string_pretty(&entries)?)?;

    let cfg = AuthConfig {
        auth_file: auth_file.clone(),
        oidc_issuer: Some(issuer.to_string()),
        oidc_client_id: Some(client_id.to_string()),
        ..AuthConfig::default()
    };

    let store = AuthStore::new(auth_file, cfg).await?;
    let token = store.ensure_fresh().await?;

    assert_eq!(token.access_token, "preferred-token");
    assert_eq!(token.issuer_key, preferred_key);

    Ok(())
}

#[tokio::test]
async fn expired_oidc_entry_refreshes_and_updates_auth_file() -> Result<(), Box<dyn Error>> {
    async fn discovery(base: String) -> impl IntoResponse {
        Json(json!({
            "issuer": base,
            "token_endpoint": format!("{base}/token")
        }))
    }

    async fn token(Form(params): Form<HashMap<String, String>>) -> impl IntoResponse {
        assert_eq!(
            params.get("grant_type").map(String::as_str),
            Some("refresh_token")
        );
        assert_eq!(
            params.get("refresh_token").map(String::as_str),
            Some("old-refresh")
        );
        assert_eq!(
            params.get("client_id").map(String::as_str),
            Some("mitsuro-client")
        );

        Json(json!({
            "access_token": "new-access-token",
            "refresh_token": "new-refresh-token",
            "expires_in": 3600
        }))
    }

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let issuer = format!("http://{addr}");
    let discovery_issuer = issuer.clone();
    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || discovery(discovery_issuer.clone())),
        )
        .route("/token", post(token));
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            panic!("test server failed: {err}");
        }
    });

    let dir = tempdir()?;
    let auth_file = dir.path().join("auth.json");
    let client_id = "mitsuro-client";
    let issuer_key = format!("{issuer}::{client_id}");
    let mut entries = HashMap::new();
    entries.insert(
        issuer_key.clone(),
        AuthEntry {
            access_token: "old-access-token".to_string(),
            refresh_token: Some("old-refresh".to_string()),
            expires_at: Some(Utc::now() - Duration::minutes(1)),
            oidc_issuer: Some(issuer.clone()),
            oidc_client_id: Some(client_id.to_string()),
            ..AuthEntry::default()
        },
    );
    fs::write(&auth_file, serde_json::to_string_pretty(&entries)?)?;

    let cfg = AuthConfig {
        auth_file: auth_file.clone(),
        oidc_issuer: Some(issuer),
        oidc_client_id: Some(client_id.to_string()),
        ..AuthConfig::default()
    };

    let store = AuthStore::new(auth_file.clone(), cfg).await?;
    let token = store.ensure_fresh().await?;

    assert_eq!(token.access_token, "new-access-token");
    assert_eq!(token.refresh_token.as_deref(), Some("new-refresh-token"));
    assert_eq!(token.issuer_key, issuer_key);

    let saved: HashMap<String, AuthEntry> = serde_json::from_str(&fs::read_to_string(auth_file)?)?;
    let refreshed = saved
        .get(&issuer_key)
        .expect("refreshed entry should be saved");
    assert_eq!(refreshed.access_token, "new-access-token");
    assert_eq!(
        refreshed.refresh_token.as_deref(),
        Some("new-refresh-token")
    );
    assert!(
        refreshed
            .expires_at
            .expect("expires_at should be refreshed")
            > Utc::now()
    );

    Ok(())
}

#[tokio::test]
async fn client_builder_sends_auth_and_client_version_headers() -> Result<(), Box<dyn Error>> {
    async fn echo_headers(headers: HeaderMap) -> impl IntoResponse {
        Json(json!({
            "authorization": headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            "client_version": headers
                .get("x-grok-client-version")
                .and_then(|value| value.to_str().ok()),
            "user_agent": headers
                .get("user-agent")
                .and_then(|value| value.to_str().ok())
        }))
    }

    let base = spawn_server(Router::new().route("/headers", get(echo_headers))).await?;
    let token = AuthToken {
        access_token: "secret-token".to_string(),
        refresh_token: None,
        expires_at: None,
        issuer_key: "api_key".to_string(),
    };
    let client = ClientBuilder::new()
        .with_token(token)
        .with_client_version("mitsuro-test")
        .build()?;

    let body: serde_json::Value = client
        .inner()
        .get(format!("{base}/headers"))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(body["authorization"], "Bearer secret-token");
    assert_eq!(body["client_version"], "mitsuro-test");
    assert!(body["user_agent"]
        .as_str()
        .unwrap_or_default()
        .contains("grok-agent"));

    Ok(())
}

#[tokio::test]
async fn external_provider_json_output_is_accepted() -> Result<(), Box<dyn Error>> {
    let cfg = AuthConfig {
        auth_provider_command: Some(
            "printf '%s' '{\"access_token\":\"external-token\",\"refresh_token\":\"external-refresh\",\"expires_in\":60}'"
                .to_string(),
        ),
        ..AuthConfig::default()
    };

    let entry = authenticate(&cfg).await?;

    assert_eq!(entry.access_token, "external-token");
    assert_eq!(entry.refresh_token.as_deref(), Some("external-refresh"));
    assert_eq!(entry.auth_mode.as_deref(), Some("external"));
    assert!(entry.expires_at.expect("expires_at should be set") > Utc::now());

    Ok(())
}

#[test]
fn merge_toml_and_auth_url_helpers_work() -> Result<(), Box<dyn Error>> {
    let mut cfg = AuthConfig::default();
    cfg.merge_toml(
        r#"
        [grok_com_config.oidc]
        issuer = "https://issuer.example"
        client_id = "mitsuro-client"

        [auth]
        auth_provider_command = "/usr/bin/provider"
        auth_provider_label = "Mitsuro Corp"
        "#,
    )?;

    assert_eq!(cfg.oidc_issuer.as_deref(), Some("https://issuer.example"));
    assert_eq!(cfg.oidc_client_id.as_deref(), Some("mitsuro-client"));
    assert_eq!(
        cfg.auth_provider_command.as_deref(),
        Some("/usr/bin/provider")
    );
    assert_eq!(cfg.auth_provider_label.as_deref(), Some("Mitsuro Corp"));

    let challenge = grok_auth::oidc::pkce_challenge("test-verifier");
    assert_eq!(challenge, "JBbiqONGWPaAmwXk_8bT6UnlPfrn65D32eZlJS-zGG0");

    let auth_url = grok_auth::oidc::build_auth_url(
        "https://issuer.example/authorize",
        "mitsuro-client",
        "http://127.0.0.1:1234/callback",
        &["openid".to_string(), "api:access".to_string()],
        "state-123",
        &challenge,
    )?;
    let query: HashMap<_, _> = auth_url.query_pairs().into_owned().collect();

    assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
    assert_eq!(
        query.get("client_id").map(String::as_str),
        Some("mitsuro-client")
    );
    assert_eq!(
        query.get("redirect_uri").map(String::as_str),
        Some("http://127.0.0.1:1234/callback")
    );
    assert_eq!(
        query.get("scope").map(String::as_str),
        Some("openid api:access")
    );
    assert_eq!(query.get("state").map(String::as_str), Some("state-123"));
    assert_eq!(
        query.get("code_challenge").map(String::as_str),
        Some(challenge.as_str())
    );
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );

    Ok(())
}
