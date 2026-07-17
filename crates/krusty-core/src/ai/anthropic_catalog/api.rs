use anyhow::{bail, Result};
use reqwest::{Client, RequestBuilder};
use tracing::{debug, warn};

use crate::ai::models::ModelMetadata;

use super::mapping::parse_model;
use super::types::ModelsResponse;

const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const ANTHROPIC_OAUTH_BETA: &str = "claude-code-20250219,oauth-2025-04-20";

/// Fetch the models available to an Anthropic credential.
///
/// `oauth` selects Bearer authentication for Claude OAuth credentials. API
/// keys use Anthropic's `x-api-key` header.
pub async fn fetch_models(credential: &str, oauth: bool) -> Result<Vec<ModelMetadata>> {
    fetch_models_with_client(credential, oauth, None).await
}

/// Fetch Anthropic models with an optional shared HTTP client.
pub async fn fetch_models_with_client(
    credential: &str,
    oauth: bool,
    client: Option<&Client>,
) -> Result<Vec<ModelMetadata>> {
    if credential.trim().is_empty() {
        bail!("Anthropic model discovery requires a credential");
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
        let response = build_request(client, credential, oauth, cursor.as_deref())
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!("Anthropic models API error: {status} - {detail}");
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
            warn!("Anthropic model catalog indicated another page without a cursor");
            break;
        };
        if cursor.as_deref() == Some(next_cursor.as_str()) {
            warn!("Anthropic model catalog repeated its pagination cursor");
            break;
        }
        cursor = Some(next_cursor);
    }

    models.sort_by(|left, right| left.id.cmp(&right.id));
    models.dedup_by(|left, right| left.id == right.id);
    debug!(count = models.len(), "Fetched Anthropic model catalog");
    Ok(models)
}

fn build_request(
    client: &Client,
    credential: &str,
    oauth: bool,
    after_id: Option<&str>,
) -> RequestBuilder {
    let mut request = client
        .get(ANTHROPIC_MODELS_URL)
        .header("anthropic-version", ANTHROPIC_API_VERSION);
    request = if oauth {
        request
            .bearer_auth(credential)
            .header("anthropic-beta", ANTHROPIC_OAUTH_BETA)
    } else {
        request.header("x-api-key", credential)
    };
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
    fn builds_api_key_catalog_request() {
        let request = build_request(&Client::new(), "sk-ant-test", false, Some("cursor"))
            .build()
            .expect("request builds");

        assert_eq!(request.headers()["x-api-key"], "sk-ant-test");
        assert_eq!(request.headers()["anthropic-version"], "2023-06-01");
        assert!(request.headers().get("authorization").is_none());
        assert_eq!(request.url().query(), Some("after_id=cursor"));
    }

    #[test]
    fn builds_oauth_catalog_request() {
        let request = build_request(&Client::new(), "oauth-token", true, None)
            .build()
            .expect("request builds");

        assert_eq!(request.headers()["authorization"], "Bearer oauth-token");
        assert_eq!(
            request.headers()["anthropic-beta"],
            "claude-code-20250219,oauth-2025-04-20"
        );
        assert!(request.headers().get("x-api-key").is_none());
    }
}
