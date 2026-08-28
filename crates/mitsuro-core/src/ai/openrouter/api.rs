use anyhow::{Context, Result};
use reqwest::Client;
use tracing::{debug, info};

use crate::ai::models::ModelMetadata;
use crate::ai::transport_policy::build_provider_http_client;

use super::mapping::{is_useful_model, parse_model};
use super::types::ModelsResponse;

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// Fetch all available models from OpenRouter.
///
/// Optionally accepts a shared HTTP client to avoid creating new connections.
pub async fn fetch_models(api_key: &str) -> Result<Vec<ModelMetadata>> {
    fetch_models_with_client(api_key, None).await
}

/// Fetch models with an optional shared HTTP client.
pub async fn fetch_models_with_client(
    api_key: &str,
    client: Option<&Client>,
) -> Result<Vec<ModelMetadata>> {
    let owned_client;
    let client = match client {
        Some(c) => c,
        None => {
            owned_client = build_provider_http_client()
                .context("failed to build OpenRouter catalog HTTP client without redirects")?;
            &owned_client
        }
    };

    fetch_models_from_url(api_key, client, OPENROUTER_MODELS_URL).await
}

async fn fetch_models_from_url(
    api_key: &str,
    client: &Client,
    models_url: &str,
) -> Result<Vec<ModelMetadata>> {
    info!("Fetching models from OpenRouter...");

    let response = client
        .get(models_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header(
            "HTTP-Referer",
            "https://github.com/honeycomb-Technologies/Mitsuro",
        )
        .send()
        .await?;

    if !response.status().is_success() {
        let error = crate::ai::retry::provider_http_error(response, "OpenRouter API error").await;
        error.log();
        return Err(error.into());
    }

    let data: ModelsResponse = response.json().await?;
    info!("OpenRouter returned {} models", data.data.len());

    let models: Vec<ModelMetadata> = data
        .data
        .into_iter()
        .filter(|m| is_useful_model(&m.id))
        .map(parse_model)
        .filter(|m| m.supports_tools)
        .collect();

    debug!("Filtered to {} models with tool support", models.len());
    Ok(models)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use tiny_http::{Header, Response, Server};

    use super::*;

    #[tokio::test]
    async fn catalog_fetch_returns_redirect_without_following_it() {
        let server = Server::http("127.0.0.1:0").expect("test server should bind");
        let base_url = format!("http://{}", server.server_addr());
        let redirect_url = format!("{base_url}/followed");
        let (request_tx, request_rx) = mpsc::channel();
        let server_thread = thread::spawn(move || {
            let request = server.recv().expect("catalog request should arrive");
            let initial_path = request.url().to_string();
            request
                .respond(
                    Response::from_string("redirect")
                        .with_status_code(307)
                        .with_header(
                            Header::from_bytes("Location", redirect_url)
                                .expect("redirect header should be valid"),
                        ),
                )
                .expect("redirect should be sent");

            let followed_path = server
                .recv_timeout(Duration::from_millis(300))
                .expect("redirect probe should succeed")
                .map(|request| {
                    let path = request.url().to_string();
                    request
                        .respond(Response::from_string(r#"{"data":[]}"#))
                        .expect("followed response should be sent");
                    path
                });
            request_tx
                .send((initial_path, followed_path))
                .expect("request evidence should be recorded");
        });

        let client = build_provider_http_client()
            .expect("OpenRouter catalog client without redirects should build");
        fetch_models_from_url("test-key", &client, &format!("{base_url}/models"))
            .await
            .expect_err("OpenRouter catalog redirect must be returned as an error");

        server_thread.join().expect("server thread should finish");
        let (initial_path, followed_path) = request_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("request evidence should arrive");
        assert_eq!(initial_path, "/models");
        assert_eq!(followed_path, None);
    }
}
