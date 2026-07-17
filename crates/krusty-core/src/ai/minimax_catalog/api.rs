use anyhow::{bail, Result};
use reqwest::{Client, RequestBuilder};
use tracing::{debug, warn};

use crate::ai::models::ModelMetadata;

use super::mapping::parse_model;
use super::types::ModelsResponse;

const MINIMAX_MODELS_URL: &str = "https://api.minimax.io/anthropic/v1/models";

/// Fetch the live model IDs available to a MiniMax API key.
pub async fn fetch_models(api_key: &str) -> Result<Vec<ModelMetadata>> {
    fetch_models_with_client(api_key, None).await
}

/// Fetch MiniMax models with an optional shared HTTP client.
pub async fn fetch_models_with_client(
    api_key: &str,
    client: Option<&Client>,
) -> Result<Vec<ModelMetadata>> {
    if api_key.trim().is_empty() {
        bail!("MiniMax model discovery requires an API key");
    }

    let owned_client;
    let client = match client {
        Some(client) => client,
        None => {
            owned_client = Client::new();
            &owned_client
        }
    };

    let mut cursor: Option<String> = None;
    let mut models = Vec::new();
    loop {
        let response = build_request(client, api_key, cursor.as_deref())
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!("MiniMax models API error: {status} - {detail}");
        }

        let page = response.json::<ModelsResponse>().await?;
        let has_more = page.has_more;
        let next_cursor = page
            .last_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        models.extend(page.data.into_iter().filter_map(parse_model));

        if !has_more {
            break;
        }
        let Some(next_cursor) = next_cursor else {
            warn!("MiniMax model catalog indicated another page without a cursor");
            break;
        };
        if cursor.as_deref() == Some(next_cursor.as_str()) {
            warn!("MiniMax model catalog repeated its pagination cursor");
            break;
        }
        cursor = Some(next_cursor);
    }

    models.sort_by(|left, right| left.id.cmp(&right.id));
    models.dedup_by(|left, right| left.id == right.id);
    debug!(count = models.len(), "Fetched MiniMax model catalog");
    Ok(models)
}

fn build_request(client: &Client, api_key: &str, after_id: Option<&str>) -> RequestBuilder {
    let mut request = client.get(MINIMAX_MODELS_URL).header("x-api-key", api_key);
    if let Some(after_id) = after_id {
        request = request.query(&[("after_id", after_id)]);
    }
    request
}

#[cfg(test)]
mod tests {
    use super::build_request;
    use reqwest::Client;

    #[test]
    fn builds_x_api_key_catalog_request() {
        let request = build_request(&Client::new(), "minimax-test", Some("cursor"))
            .build()
            .expect("request builds");

        assert_eq!(request.headers()["x-api-key"], "minimax-test");
        assert!(request.headers().get("authorization").is_none());
        assert_eq!(request.url().query(), Some("after_id=cursor"));
    }
}
