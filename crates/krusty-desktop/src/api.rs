use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::sync::mpsc::{self, Receiver};
use std::thread;
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

    pub fn set_credential(&self, provider: &str, api_key: String) -> Result<ProviderStatus> {
        let response = self
            .http
            .post(self.api_url(&format!("/credentials/{provider}")))
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .json(&SetCredentialRequest { api_key })
            .send()
            .with_context(|| format!("failed to set credential for {provider}"))?;
        ensure_success(response)?
            .json::<ProviderStatus>()
            .with_context(|| format!("failed to decode credential status for {provider}"))
    }

    pub fn delete_credential(&self, provider: &str) -> Result<ProviderStatus> {
        let response = self
            .http
            .delete(self.api_url(&format!("/credentials/{provider}")))
            .header(ACCEPT, "application/json")
            .send()
            .with_context(|| format!("failed to delete credential for {provider}"))?;
        ensure_success(response)?
            .json::<ProviderStatus>()
            .with_context(|| format!("failed to decode credential status for {provider}"))
    }

    pub fn start_oauth(
        &self,
        provider: &str,
        flow_type: Option<&str>,
    ) -> Result<OAuthStartResponse> {
        let response = self
            .http
            .post(self.api_url("/auth/oauth/start"))
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .json(&OAuthStartRequest {
                provider,
                flow_type,
            })
            .send()
            .with_context(|| format!("failed to start {provider} OAuth"))?;
        ensure_success(response)?
            .json::<OAuthStartResponse>()
            .with_context(|| format!("failed to decode OAuth start response for {provider}"))
    }

    pub fn oauth_status(&self, provider: &str) -> Result<OAuthStatusResponse> {
        self.get_json(&format!("/auth/oauth/status/{provider}"))
    }

    pub fn revoke_oauth(&self, provider: &str) -> Result<OAuthStatusResponse> {
        let response = self
            .http
            .delete(self.api_url(&format!("/auth/oauth/revoke/{provider}")))
            .header(ACCEPT, "application/json")
            .send()
            .with_context(|| format!("failed to revoke OAuth for {provider}"))?;
        ensure_success(response)?
            .json::<OAuthStatusResponse>()
            .with_context(|| format!("failed to decode OAuth revoke response for {provider}"))
    }

    pub fn exchange_oauth_code(&self, provider: &str, code: String) -> Result<()> {
        let response = self
            .http
            .post(self.api_url("/auth/oauth/exchange"))
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .json(&OAuthExchangeRequest {
                provider: provider.to_owned(),
                code,
            })
            .send()
            .with_context(|| format!("failed to exchange OAuth code for {provider}"))?;
        ensure_success(response)?;
        Ok(())
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

    pub fn approve_tool(&self, session_id: &str, tool_call_id: &str, approved: bool) -> Result<()> {
        let response = self
            .http
            .post(self.api_url(&format!("/sessions/{session_id}/approvals")))
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .json(&ToolApprovalRequest {
                tool_call_id: tool_call_id.to_owned(),
                approved,
            })
            .send()
            .with_context(|| format!("failed to submit tool approval for {tool_call_id}"))?;
        ensure_success(response)?;
        Ok(())
    }

    pub fn start_chat_stream(&self, request: ChatRequest) -> Receiver<ChatStreamEvent> {
        let (tx, rx) = mpsc::channel();
        let client = self.clone();
        thread::spawn(move || {
            let result = (|| {
                let response = client.post_chat(request)?;
                parse_chat_stream(response, |event| {
                    let _ = tx.send(event);
                })
            })();
            let _ = tx.send(ChatStreamEvent::Complete(result));
        });
        rx
    }

    fn post_chat(&self, request: ChatRequest) -> Result<Response> {
        let response = self
            .http
            .post(self.api_url("/chat"))
            .timeout(STREAM_TIMEOUT)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .json(&request)
            .send()
            .context("failed to submit chat request")?;
        ensure_success(response)
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
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub supports_thinking: bool,
    #[serde(default)]
    pub reasoning_control: Option<ReasoningControl>,
    #[serde(default)]
    pub supported_reasoning_levels: Vec<String>,
    #[serde(default)]
    pub default_reasoning_level: Option<String>,
    #[serde(default)]
    pub reasoning_is_mandatory: bool,
    #[serde(default)]
    pub supports_fast_mode: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningControl {
    OpenAiEffort,
    AnthropicAdaptive,
    AnthropicBudget,
    Boolean,
    OutputOnly,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub configured: bool,
    #[serde(default)]
    pub has_oauth: bool,
    #[serde(default)]
    pub supports_oauth: bool,
    #[serde(default)]
    pub auth_methods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveOAuthFlow {
    pub provider: String,
    pub flow_type: String,
    pub paste_code: bool,
    pub device_user_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OAuthStartResponse {
    pub auth_url: String,
    pub provider: String,
    pub flow_type: String,
    pub paste_code: bool,
    pub device_code: Option<OAuthDeviceCode>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OAuthDeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OAuthStatusResponse {
    pub has_token: bool,
    pub flow_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PlanItem {
    pub content: String,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatStreamResult {
    pub session_id: Option<String>,
    pub text: String,
    pub title: Option<String>,
}

#[derive(Debug)]
pub enum ChatStreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolExecuting {
        id: String,
        name: String,
    },
    ToolOutputDelta {
        id: String,
        delta: String,
    },
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    TitleUpdate(String),
    PlanUpdate(Vec<PlanItem>),
    ToolApprovalRequired {
        id: String,
        name: String,
    },
    Error(String),
    Complete(Result<ChatStreamResult>),
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_enabled: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct SetCredentialRequest {
    api_key: String,
}

#[derive(Debug, Serialize)]
struct OAuthStartRequest<'a> {
    provider: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    flow_type: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct OAuthExchangeRequest {
    provider: String,
    code: String,
}

#[derive(Debug, Serialize)]
struct ToolApprovalRequest {
    tool_call_id: String,
    approved: bool,
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

fn parse_chat_stream(
    response: Response,
    mut on_event: impl FnMut(ChatStreamEvent),
) -> Result<ChatStreamResult> {
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
                    on_event(ChatStreamEvent::TextDelta(delta.to_owned()));
                }
            }
            "thinking_delta" => {
                if let Some(thinking) = event.get("thinking").and_then(Value::as_str) {
                    on_event(ChatStreamEvent::ThinkingDelta(thinking.to_owned()));
                }
            }
            "tool_call_start" => {
                if let (Some(id), Some(name)) = (
                    event.get("id").and_then(Value::as_str),
                    event.get("name").and_then(Value::as_str),
                ) {
                    on_event(ChatStreamEvent::ToolCallStart {
                        id: id.to_owned(),
                        name: name.to_owned(),
                    });
                }
            }
            "tool_executing" => {
                if let (Some(id), Some(name)) = (
                    event.get("id").and_then(Value::as_str),
                    event.get("name").and_then(Value::as_str),
                ) {
                    on_event(ChatStreamEvent::ToolExecuting {
                        id: id.to_owned(),
                        name: name.to_owned(),
                    });
                }
            }
            "tool_output_delta" => {
                if let (Some(id), Some(delta)) = (
                    event.get("id").and_then(Value::as_str),
                    event.get("delta").and_then(Value::as_str),
                ) {
                    on_event(ChatStreamEvent::ToolOutputDelta {
                        id: id.to_owned(),
                        delta: delta.to_owned(),
                    });
                }
            }
            "tool_result" => {
                if let Some(id) = event.get("id").and_then(Value::as_str) {
                    let output = event
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let is_error = event
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    on_event(ChatStreamEvent::ToolResult {
                        id: id.to_owned(),
                        output,
                        is_error,
                    });
                }
            }
            "title_update" => {
                if let Some(next_title) = event.get("title").and_then(Value::as_str) {
                    title = Some(next_title.to_owned());
                    on_event(ChatStreamEvent::TitleUpdate(next_title.to_owned()));
                }
            }
            "plan_update" => {
                let items = event
                    .get("items")
                    .and_then(|value| serde_json::from_value::<Vec<PlanItem>>(value.clone()).ok())
                    .unwrap_or_default();
                on_event(ChatStreamEvent::PlanUpdate(items));
            }
            "tool_approval_required" => {
                if let (Some(id), Some(name)) = (
                    event.get("id").and_then(Value::as_str),
                    event.get("name").and_then(Value::as_str),
                ) {
                    on_event(ChatStreamEvent::ToolApprovalRequired {
                        id: id.to_owned(),
                        name: name.to_owned(),
                    });
                }
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
                    .unwrap_or("chat stream failed")
                    .to_owned();
                on_event(ChatStreamEvent::Error(error.clone()));
                return Err(anyhow!(error));
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
