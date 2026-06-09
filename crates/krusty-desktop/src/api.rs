use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const STREAM_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub struct KrustyApiClient {
    base_url: String,
    http: Client,
}

impl KrustyApiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = normalize_base_url(base_url.into());
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("desktop HTTP client should build");

        Self { base_url, http }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn health(&self) -> Result<HealthResponse> {
        self.get_json_at(&self.url("/health"))
    }

    pub fn list_sessions(&self) -> Result<Vec<Value>> {
        self.get_json("/sessions")
    }

    pub fn list_models(&self) -> Result<ModelsResponse> {
        self.get_json("/models")
    }

    pub fn list_credentials(&self) -> Result<Vec<ProviderStatus>> {
        self.get_json("/credentials")
    }

    pub fn overview(&self) -> Result<ServerOverview> {
        let health = self.health()?;
        let sessions = self.list_sessions().unwrap_or_default();
        let models = self.list_models().unwrap_or_default();
        let credentials = self.list_credentials().unwrap_or_default();
        let configured_providers = credentials
            .iter()
            .filter(|provider| provider.configured)
            .count();

        Ok(ServerOverview {
            status: health.status,
            version: health.version,
            chat_enabled: health.features.get("chat").copied().unwrap_or(false),
            tools_enabled: health.features.get("tools").copied().unwrap_or(false),
            session_count: sessions.len(),
            model_count: models.models.len(),
            default_model: models.default_model,
            provider_count: credentials.len(),
            configured_provider_count: configured_providers,
        })
    }

    pub fn send_chat_collect(
        &self,
        session_id: Option<String>,
        message: String,
    ) -> Result<ChatStreamResult> {
        let response = self
            .http
            .post(self.api_url("/chat"))
            .timeout(STREAM_TIMEOUT)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .json(&ChatRequest {
                session_id,
                message,
            })
            .send()
            .context("failed to submit chat request")?;

        ensure_success(response).and_then(parse_chat_stream)
    }

    fn get_json<T>(&self, path: &str) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.get_json_at(&self.api_url(path))
    }

    fn get_json_at<T>(&self, url: &str) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .http
            .get(url)
            .header(ACCEPT, "application/json")
            .send()
            .with_context(|| format!("failed to GET {url}"))?;
        let response = ensure_success(response)?;
        response
            .json::<T>()
            .with_context(|| format!("failed to decode response from {url}"))
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/api{}", self.base_url, prefixed_path(path))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, prefixed_path(path))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerOverview {
    pub status: String,
    pub version: String,
    pub chat_enabled: bool,
    pub tools_enabled: bool,
    pub session_count: usize,
    pub model_count: usize,
    pub default_model: Option<String>,
    pub provider_count: usize,
    pub configured_provider_count: usize,
}

impl ServerOverview {
    pub fn summary(&self) -> String {
        format!(
            "{} v{} · {} sessions · {} models · {}/{} providers configured",
            self.status,
            self.version,
            self.session_count,
            self.model_count,
            self.configured_provider_count,
            self.provider_count
        )
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    #[serde(default)]
    pub features: HashMap<String, bool>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
pub struct ModelsResponse {
    #[serde(default)]
    pub models: Vec<ModelResponse>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModelResponse {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatStreamResult {
    pub session_id: Option<String>,
    pub text: String,
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    message: String,
}

fn normalize_base_url(mut base_url: String) -> String {
    if base_url.trim().is_empty() {
        base_url = "http://127.0.0.1:3000".to_owned();
    }
    base_url.trim().trim_end_matches('/').to_owned()
}

fn prefixed_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

fn ensure_success(response: Response) -> Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response
        .text()
        .unwrap_or_else(|_| "Request failed".to_owned());
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.clone());
    Err(anyhow!("API {status}: {message}"))
}

fn parse_chat_stream(response: Response) -> Result<ChatStreamResult> {
    let mut text = String::new();
    let mut session_id = None;
    let mut title = None;
    let reader = BufReader::new(response);

    for line in reader.lines() {
        let line = line.context("failed to read chat stream")?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            continue;
        }
        let Some(data) = trimmed.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        let event = serde_json::from_str::<Value>(data)
            .with_context(|| format!("failed to parse stream event: {data}"))?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event_type {
            "text_delta" | "text_delta_with_citations" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    text.push_str(delta);
                }
            }
            "title_update" => {
                title = event
                    .get("title")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            "finish" => {
                session_id = event
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            "error" => {
                let error = event
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("chat stream failed");
                return Err(anyhow!(error.to_owned()));
            }
            _ => {}
        }
    }

    Ok(ChatStreamResult {
        session_id,
        text,
        title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_empty_base_url_to_local_server() {
        let client = KrustyApiClient::new(" ");
        assert_eq!(client.base_url(), "http://127.0.0.1:3000");
    }
}
