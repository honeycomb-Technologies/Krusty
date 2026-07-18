use anyhow::Result;
use reqwest::Client;
use tracing::{debug, error, info};

use crate::ai::models::ModelMetadata;

use super::mapping::{is_useful_model, parse_chatgpt_model, parse_model};
use super::types::{ChatGptModelsResponse, ModelsResponse};

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";
const CHATGPT_MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";
const CHATGPT_ACCOUNT_ID_HEADER: &str = "ChatGPT-Account-ID";
// This is a Codex catalog protocol compatibility version, not Krusty's package
// version. Keep it aligned with a stable official Codex release; operators can
// override it if the catalog raises its minimum before Krusty's next release.
const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.144.4";
const CODEX_CLIENT_VERSION_ENV: &str = "KRUSTY_CODEX_CLIENT_VERSION";

/// Fetch available OpenAI models for the authenticated account.
pub async fn fetch_models(api_key: &str) -> Result<Vec<ModelMetadata>> {
    fetch_models_with_client(api_key, None).await
}

/// Fetch OpenAI models with an optional shared HTTP client.
pub async fn fetch_models_with_client(
    api_key: &str,
    client: Option<&Client>,
) -> Result<Vec<ModelMetadata>> {
    let owned_client;
    let client = match client {
        Some(client) => client,
        None => {
            owned_client = Client::new();
            &owned_client
        }
    };

    info!("Fetching models from OpenAI...");
    let response = client
        .get(OPENAI_MODELS_URL)
        .bearer_auth(api_key)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        error!("OpenAI models API error: {} - {}", status, error_text);
        return Err(anyhow::anyhow!(
            "OpenAI models API error: {} - {}",
            status,
            error_text
        ));
    }

    let mut models: Vec<ModelMetadata> = response
        .json::<ModelsResponse>()
        .await?
        .data
        .into_iter()
        .filter(|model| is_useful_model(&model.id))
        .map(parse_model)
        .collect();

    models.sort_by(|a, b| a.id.cmp(&b.id));
    debug!("Fetched {} usable OpenAI models", models.len());
    Ok(models)
}

/// Fetch the entitlement-specific ChatGPT Codex model catalog.
///
/// Unlike OpenAI's public `/v1/models` response, this endpoint advertises the
/// exact reasoning levels, service tiers, visibility, and context available to
/// the authenticated ChatGPT account.
pub async fn fetch_chatgpt_models(
    access_token: &str,
    account_id: Option<&str>,
) -> Result<Vec<ModelMetadata>> {
    fetch_chatgpt_models_with_client(access_token, account_id, None).await
}

/// Fetch the ChatGPT catalog with an optional shared HTTP client.
pub async fn fetch_chatgpt_models_with_client(
    access_token: &str,
    account_id: Option<&str>,
    client: Option<&Client>,
) -> Result<Vec<ModelMetadata>> {
    let owned_client;
    let client = match client {
        Some(client) => client,
        None => {
            owned_client = Client::new();
            &owned_client
        }
    };

    info!("Fetching models from the ChatGPT Codex catalog...");
    let response = chatgpt_models_request(client, CHATGPT_MODELS_URL, access_token, account_id)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        error!(
            "ChatGPT Codex models API error: {} - {}",
            status, error_text
        );
        return Err(anyhow::anyhow!(
            "ChatGPT Codex models API error: {} - {}",
            status,
            error_text
        ));
    }

    let response = response.json::<ChatGptModelsResponse>().await?;
    let models = response
        .models
        .into_iter()
        .filter_map(parse_chatgpt_model)
        .collect::<Vec<_>>();

    debug!("Fetched {} visible ChatGPT Codex models", models.len());
    Ok(models)
}

fn chatgpt_models_request(
    client: &Client,
    url: &str,
    access_token: &str,
    account_id: Option<&str>,
) -> reqwest::RequestBuilder {
    let client_version = codex_client_version();
    let mut request = client
        .get(url)
        .query(&[("client_version", client_version.as_str())])
        .bearer_auth(access_token);
    if let Some(account_id) = account_id.filter(|value| !value.trim().is_empty()) {
        request = request.header(CHATGPT_ACCOUNT_ID_HEADER, account_id);
    }
    request
}

fn codex_client_version() -> String {
    codex_client_version_from(std::env::var(CODEX_CLIENT_VERSION_ENV).ok())
}

fn codex_client_version_from(configured: Option<String>) -> String {
    configured
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CODEX_CLIENT_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        chatgpt_models_request, codex_client_version, codex_client_version_from,
        CHATGPT_ACCOUNT_ID_HEADER, CHATGPT_MODELS_URL, DEFAULT_CODEX_CLIENT_VERSION,
    };
    use reqwest::header::AUTHORIZATION;
    use reqwest::Client;

    #[test]
    fn chatgpt_request_uses_versioned_endpoint_and_account_scoped_bearer_auth() {
        let request = chatgpt_models_request(
            &Client::new(),
            CHATGPT_MODELS_URL,
            "oauth-token",
            Some("account-123"),
        )
        .build()
        .unwrap();

        assert_eq!(
            request.url().as_str(),
            format!(
                "{CHATGPT_MODELS_URL}?client_version={}",
                codex_client_version()
            )
        );
        assert_eq!(
            request.headers().get(AUTHORIZATION).unwrap(),
            "Bearer oauth-token"
        );
        assert_eq!(
            request.headers().get(CHATGPT_ACCOUNT_ID_HEADER).unwrap(),
            "account-123"
        );
    }

    #[test]
    fn chatgpt_request_omits_empty_account_header() {
        let request = chatgpt_models_request(
            &Client::new(),
            CHATGPT_MODELS_URL,
            "oauth-token",
            Some("  "),
        )
        .build()
        .unwrap();

        assert!(!request.headers().contains_key(CHATGPT_ACCOUNT_ID_HEADER));
    }

    #[test]
    fn codex_catalog_version_is_protocol_scoped_and_overridable() {
        assert_eq!(
            codex_client_version_from(None),
            DEFAULT_CODEX_CLIENT_VERSION
        );
        assert_eq!(
            codex_client_version_from(Some(" 0.200.1 ".to_string())),
            "0.200.1"
        );
        assert_eq!(
            codex_client_version_from(Some("  ".to_string())),
            DEFAULT_CODEX_CLIENT_VERSION
        );
        assert_ne!(DEFAULT_CODEX_CLIENT_VERSION, env!("CARGO_PKG_VERSION"));
    }
}
