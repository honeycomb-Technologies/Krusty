use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::sse::{chat_stream_from_response, ChatEventStream};
use crate::{
    BackgroundProcess, ChatRequest, CreateSessionRequest, ExtensionOverview, FileResponse,
    FileTreeResponse, HealthResponse, HiveCurrentResponse, HiveScheduleSummary, McpServer,
    ModelsResponse, OAuthExchangeRequest, OAuthExchangeResponse, OAuthStartRequest,
    OAuthStartResponse, OAuthStatusResponse, ProviderStatus, ServerAccessResponse,
    ServerStatusResponse, SessionInfo, SessionStateOptions, SessionStateResponse,
    SessionWithMessages, SetCredentialRequest, SimpleOkResponse, SkillInfo, ToolApprovalRequest,
    UpdateServerAccessRequest, UpdateSessionRequest,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const STREAM_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct MitsuroClient {
    base_url: Arc<str>,
    http: Client,
}

impl MitsuroClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        Self::build(base_url.into(), None)
    }

    /// Construct a client for an authenticated remote Mitsuro server.
    ///
    /// The token is installed as a default header and is never exposed by this
    /// type's public API. Local loopback servers normally use [`Self::local`].
    pub fn with_bearer_token(base_url: impl Into<String>, token: impl AsRef<str>) -> Result<Self> {
        Self::build(base_url.into(), Some(token.as_ref()))
    }

    fn build(base_url: String, bearer_token: Option<&str>) -> Result<Self> {
        let base_url = normalize_base_url(base_url);
        let mut headers = HeaderMap::new();
        if let Some(token) = bearer_token {
            let token = token.trim();
            if token.is_empty() {
                return Err(anyhow!("Mitsuro bearer token cannot be empty"));
            }
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .context("validating Mitsuro bearer token")?;
            headers.insert(AUTHORIZATION, value);
        }
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .default_headers(headers)
            .build()
            .context("building Mitsuro HTTP client")?;
        Ok(Self {
            base_url: Arc::from(base_url),
            http,
        })
    }

    pub fn local() -> Result<Self> {
        Self::new("http://127.0.0.1:3000")
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn health(&self) -> Result<HealthResponse> {
        self.get_json_at(&self.url("/health")).await
    }

    pub async fn list_models(&self) -> Result<ModelsResponse> {
        self.get_json("/models").await
    }

    pub async fn list_credentials(&self) -> Result<Vec<ProviderStatus>> {
        self.get_json("/credentials").await
    }

    pub async fn credential_provider(&self, provider_id: &str) -> Result<ProviderStatus> {
        self.get_json(&format!("/credentials/{provider_id}")).await
    }

    pub async fn set_credential(
        &self,
        provider_id: &str,
        api_key: impl Into<String>,
    ) -> Result<ProviderStatus> {
        let request = SetCredentialRequest {
            api_key: api_key.into(),
        };
        self.post_json(&format!("/credentials/{provider_id}"), &request)
            .await
    }

    pub async fn delete_credential(&self, provider_id: &str) -> Result<ProviderStatus> {
        self.delete_json(&format!("/credentials/{provider_id}"))
            .await
    }

    pub async fn start_oauth(&self, request: OAuthStartRequest) -> Result<OAuthStartResponse> {
        self.post_json("/auth/oauth/start", &request).await
    }

    pub async fn oauth_status(&self, provider_id: &str) -> Result<OAuthStatusResponse> {
        self.get_json(&format!("/auth/oauth/status/{provider_id}"))
            .await
    }

    pub async fn exchange_oauth_code(
        &self,
        request: OAuthExchangeRequest,
    ) -> Result<OAuthExchangeResponse> {
        self.post_json("/auth/oauth/exchange", &request).await
    }

    pub async fn revoke_oauth(&self, provider_id: &str) -> Result<OAuthStatusResponse> {
        self.delete_json(&format!("/auth/oauth/revoke/{provider_id}"))
            .await
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        self.get_json("/sessions").await
    }

    pub async fn create_session(&self, request: CreateSessionRequest) -> Result<SessionInfo> {
        self.post_json("/sessions", &request).await
    }

    pub async fn get_session(&self, session_id: &str) -> Result<SessionWithMessages> {
        self.get_json(&format!("/sessions/{session_id}")).await
    }

    pub async fn get_session_state(&self, session_id: &str) -> Result<SessionStateResponse> {
        self.get_session_state_with_options(session_id, SessionStateOptions::default())
            .await
    }

    pub async fn get_session_state_with_options(
        &self,
        session_id: &str,
        options: SessionStateOptions,
    ) -> Result<SessionStateResponse> {
        let mut query = Vec::new();
        if options.include_delegated_history {
            query.push("include_delegated_history=true".to_string());
        }
        if let Some(cursor) = options.delegation_after_cursor {
            query.push(format!("delegation_after_cursor={}", cursor.max(0)));
        }
        let suffix = if query.is_empty() {
            String::new()
        } else {
            format!("?{}", query.join("&"))
        };
        self.get_json(&format!("/sessions/{session_id}/state{suffix}"))
            .await
    }

    pub async fn update_session(
        &self,
        session_id: &str,
        request: UpdateSessionRequest,
    ) -> Result<SessionInfo> {
        self.patch_json(&format!("/sessions/{session_id}"), &request)
            .await
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<SimpleOkResponse> {
        self.delete_json(&format!("/sessions/{session_id}")).await
    }

    pub async fn cancel_session(&self, session_id: &str) -> Result<SimpleOkResponse> {
        self.post_json(
            &format!("/sessions/{session_id}/cancel"),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn read_file(&self, path: &str) -> Result<FileResponse> {
        self.get_json_with_query("/files", &[("path", path)]).await
    }

    pub async fn file_tree(&self, root: &str, depth: usize) -> Result<FileTreeResponse> {
        let depth = depth.min(10).to_string();
        self.get_json_with_query("/files/tree", &[("root", root), ("depth", depth.as_str())])
            .await
    }

    pub async fn list_skills(&self) -> Result<Vec<SkillInfo>> {
        self.get_json("/skills").await
    }

    pub async fn list_extensions(&self) -> Result<ExtensionOverview> {
        self.get_json("/extensions").await
    }

    pub async fn list_mcp_servers(&self) -> Result<Vec<McpServer>> {
        self.get_json("/mcp").await
    }

    pub async fn list_processes(&self) -> Result<Vec<BackgroundProcess>> {
        self.get_json("/processes").await
    }

    pub async fn hive_current(&self) -> Result<HiveCurrentResponse> {
        self.get_json("/hive/current").await
    }

    pub async fn list_hive_schedules(&self) -> Result<Vec<HiveScheduleSummary>> {
        self.get_json("/hive/schedules").await
    }

    pub async fn server_access(&self) -> Result<ServerAccessResponse> {
        self.get_json("/server/access").await
    }

    pub async fn update_server_access(
        &self,
        request: UpdateServerAccessRequest,
    ) -> Result<ServerAccessResponse> {
        self.patch_json("/server/access", &request).await
    }

    pub async fn server_status(&self) -> Result<ServerStatusResponse> {
        self.get_json("/server/status").await
    }

    pub async fn approve_tool(
        &self,
        session_id: &str,
        tool_call_id: &str,
        approved: bool,
    ) -> Result<SimpleOkResponse> {
        let request = ToolApprovalRequest {
            session_id: session_id.to_owned(),
            tool_call_id: tool_call_id.to_owned(),
            approved,
        };
        let response = self
            .http
            .post(self.api_url(&format!("/sessions/{session_id}/tool-approval")))
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .json(&serde_json::json!({
                "tool_call_id": tool_call_id,
                "approved": approved,
            }))
            .send()
            .await
            .with_context(|| format!("submitting approval for tool call {tool_call_id}"))?;

        match ensure_success(response).await {
            Ok(response) => response
                .json::<SimpleOkResponse>()
                .await
                .or_else(|_| Ok(SimpleOkResponse { ok: true })),
            Err(primary_error) => {
                let fallback = self
                    .http
                    .post(self.api_url("/chat/tool-approval"))
                    .header(CONTENT_TYPE, "application/json")
                    .header(ACCEPT, "application/json")
                    .json(&request)
                    .send()
                    .await
                    .context("submitting fallback chat tool approval")?;
                ensure_success(fallback)
                    .await
                    .map_err(|fallback_error| {
                        anyhow!("{primary_error:#}; fallback failed: {fallback_error:#}")
                    })?
                    .json::<SimpleOkResponse>()
                    .await
                    .or_else(|_| Ok(SimpleOkResponse { ok: true }))
            }
        }
    }

    pub async fn chat_stream(&self, request: ChatRequest) -> Result<ChatEventStream> {
        let response = self
            .http
            .post(self.api_url("/chat"))
            .timeout(STREAM_TIMEOUT)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .json(&request)
            .send()
            .await
            .context("submitting chat request")?;
        let response = ensure_success(response).await?;
        Ok(chat_stream_from_response(response))
    }

    async fn get_json<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.get_json_at(&self.api_url(path)).await
    }

    async fn get_json_at<T>(&self, url: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let response = self
            .http
            .get(url)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        decode_json_response(response, url).await
    }

    async fn get_json_with_query<T, Q>(&self, path: &str, query: &Q) -> Result<T>
    where
        T: DeserializeOwned,
        Q: serde::Serialize + ?Sized,
    {
        let url = self.api_url(path);
        let response = self
            .http
            .get(&url)
            .query(query)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        decode_json_response(response, &url).await
    }

    async fn post_json<T, B>(&self, path: &str, body: &B) -> Result<T>
    where
        T: DeserializeOwned,
        B: serde::Serialize + ?Sized,
    {
        let url = self.api_url(path);
        let response = self
            .http
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        decode_json_response(response, &url).await
    }

    async fn patch_json<T, B>(&self, path: &str, body: &B) -> Result<T>
    where
        T: DeserializeOwned,
        B: serde::Serialize + ?Sized,
    {
        let url = self.api_url(path);
        let response = self
            .http
            .patch(&url)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .json(body)
            .send()
            .await
            .with_context(|| format!("PATCH {url}"))?;
        decode_json_response(response, &url).await
    }

    async fn delete_json<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let url = self.api_url(path);
        let response = self
            .http
            .delete(&url)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .with_context(|| format!("DELETE {url}"))?;
        decode_json_response(response, &url).await
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/api{}", self.base_url, prefixed_path(path))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, prefixed_path(path))
    }
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

async fn decode_json_response<T>(response: Response, url: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    ensure_success(response)
        .await?
        .json::<T>()
        .await
        .with_context(|| format!("decoding response from {url}"))
}

async fn ensure_success(response: Response) -> Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response
        .text()
        .await
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
        .unwrap_or(body);
    Err(anyhow!("API {status}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_blank_base_url_to_local_server() {
        let client = MitsuroClient::new(" ").expect("client");
        assert_eq!(client.base_url(), "http://127.0.0.1:3000");
    }

    #[test]
    fn trims_trailing_slashes() {
        let client = MitsuroClient::new("http://localhost:3000///").expect("client");
        assert_eq!(client.base_url(), "http://localhost:3000");
    }

    #[test]
    fn rejects_empty_remote_bearer_token() {
        let error = MitsuroClient::with_bearer_token("https://mitsuro.example", "  ")
            .expect_err("blank token must fail");
        assert!(error.to_string().contains("cannot be empty"));
    }
}
