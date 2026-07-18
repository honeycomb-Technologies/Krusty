use anyhow::{bail, Result};
use reqwest::{Client, RequestBuilder};
use tracing::debug;

use crate::ai::catalog::{next_catalog_cursor, MAX_CATALOG_PAGES};
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
    let mut seen_cursors = std::collections::HashSet::new();
    let mut models = Vec::new();
    let mut page_count = 0usize;
    loop {
        page_count += 1;
        if page_count > MAX_CATALOG_PAGES {
            bail!("MiniMax model catalog exceeded {MAX_CATALOG_PAGES} pages");
        }
        let response = build_request(client, api_key, cursor.as_deref())
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!("MiniMax models API error: {status} - {detail}");
        }

        let page = response.json::<ModelsResponse>().await?;
        let next_cursor = next_catalog_cursor(
            "MiniMax",
            page.has_more,
            page.last_id.as_deref(),
            &mut seen_cursors,
        )?;
        models.extend(page.data.into_iter().filter_map(parse_model));

        match next_cursor {
            Some(next_cursor) => cursor = Some(next_cursor),
            None => break,
        }
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
