use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, IF_MATCH};
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::sse::{chat_stream_from_response, ChatEventStream};
use crate::{
    BackgroundProcess, ChatRequest, CreateSessionRequest, ExtensionOverview, FileResponse,
    FileTreeResponse, HealthResponse, HiveCurrentResponse, HiveScheduleMutationResponse,
    HiveScheduleSummary, McpServer, ModelsResponse, OAuthExchangeRequest, OAuthExchangeResponse,
    OAuthStartRequest, OAuthStartResponse, OAuthStatusResponse, ProviderStatus,
    ServerAccessResponse, ServerStatusResponse, SessionInfo, SessionStateOptions,
    SessionStateResponse, SessionWithMessages, SetCredentialRequest, SimpleOkResponse, SkillInfo,
    SteerRequest, SteerResponse, ToolApprovalRequest, UpdateServerAccessRequest,
    UpdateSessionRequest,
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
        let url = self.api_url(&format!("/sessions/{session_id}"));
        let response = self
            .http
            .delete(&url)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .with_context(|| format!("DELETE {url}"))?;
        ensure_success(response).await?;
        Ok(SimpleOkResponse { ok: true })
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

    pub async fn kill_process(&self, process_id: &str) -> Result<()> {
        let mut url = reqwest::Url::parse(&self.api_url("/processes"))
            .context("building Mitsuro process kill URL")?;
        url.path_segments_mut()
            .map_err(|_| anyhow!("Mitsuro process endpoint cannot be a base URL"))?
            .push(process_id)
            .push("kill");
        let url = url.to_string();
        let response = self
            .http
            .post(&url)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        ensure_success(response).await?;
        Ok(())
    }

    pub async fn hive_current(&self) -> Result<HiveCurrentResponse> {
        self.get_json("/hive/current").await
    }

    pub async fn list_hive_schedules(&self) -> Result<Vec<HiveScheduleSummary>> {
        self.get_json("/hive/schedules").await
    }

    pub async fn pause_hive_schedule(
        &self,
        session_id: &str,
        schedule_id: &str,
        revision: u64,
        idempotency_key: Option<&str>,
    ) -> Result<HiveScheduleMutationResponse> {
        self.mutate_hive_schedule_status(
            reqwest::Method::POST,
            session_id,
            schedule_id,
            Some("pause"),
            revision,
            idempotency_key,
        )
        .await
    }

    pub async fn resume_hive_schedule(
        &self,
        session_id: &str,
        schedule_id: &str,
        revision: u64,
        idempotency_key: Option<&str>,
    ) -> Result<HiveScheduleMutationResponse> {
        self.mutate_hive_schedule_status(
            reqwest::Method::POST,
            session_id,
            schedule_id,
            Some("resume"),
            revision,
            idempotency_key,
        )
        .await
    }

    pub async fn cancel_hive_schedule(
        &self,
        session_id: &str,
        schedule_id: &str,
        revision: u64,
        idempotency_key: Option<&str>,
    ) -> Result<HiveScheduleMutationResponse> {
        self.mutate_hive_schedule_status(
            reqwest::Method::DELETE,
            session_id,
            schedule_id,
            None,
            revision,
            idempotency_key,
        )
        .await
    }

    async fn mutate_hive_schedule_status(
        &self,
        method: reqwest::Method,
        session_id: &str,
        schedule_id: &str,
        action: Option<&str>,
        revision: u64,
        idempotency_key: Option<&str>,
    ) -> Result<HiveScheduleMutationResponse> {
        let mut url = reqwest::Url::parse(&self.api_url("/hive/sessions"))
            .context("building Mitsuro Hive schedule mutation URL")?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow!("Mitsuro Hive endpoint cannot be a base URL"))?;
            segments
                .push(session_id)
                .push("schedules")
                .push(schedule_id);
            if let Some(action) = action {
                segments.push(action);
            }
        }
        let url = url.to_string();
        let mut request = self
            .http
            .request(method, &url)
            .header(ACCEPT, "application/json")
            .header(IF_MATCH, format!("\"{revision}\""));
        if let Some(key) = idempotency_key.filter(|key| !key.trim().is_empty()) {
            request = request.header("Idempotency-Key", key);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("mutating Hive schedule at {url}"))?;
        decode_json_response(response, &url).await
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

    /// Inject user input into the active run for an existing session.
    pub async fn steer(&self, request: SteerRequest) -> Result<SteerResponse> {
        self.post_json("/chat/steer", &request).await
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

    #[tokio::test]
    async fn delete_session_accepts_successful_empty_response() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 2048];
            let size = socket.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.starts_with("DELETE /api/sessions/session-1 "));
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .expect("write response");
        });

        let client = MitsuroClient::new(format!("http://{address}")).expect("client");
        let response = client
            .delete_session("session-1")
            .await
            .expect("empty successful delete response");
        assert!(response.ok);
        server.join().expect("test server join");
    }

    #[tokio::test]
    async fn kill_process_posts_to_scoped_process_endpoint_and_accepts_no_content() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 2048];
            let size = socket.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.starts_with("POST /api/processes/process-1/kill "));
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .expect("write response");
        });

        let client = MitsuroClient::new(format!("http://{address}")).expect("client");
        client
            .kill_process("process-1")
            .await
            .expect("empty successful process kill response");
        server.join().expect("test server join");
    }

    #[tokio::test]
    async fn hive_schedule_status_mutations_are_scoped_revisioned_and_idempotent() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let expected = [
                ("POST", "pause", "paused", 8_u64),
                ("POST", "resume", "enabled", 9_u64),
                ("DELETE", "", "cancelled", 10_u64),
            ];
            for (method, action, status, revision) in expected {
                let (mut socket, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 4096];
                let size = socket.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..size]);
                let suffix = if action.is_empty() {
                    String::new()
                } else {
                    format!("/{action}")
                };
                assert!(request.starts_with(&format!(
                    "{method} /api/hive/sessions/session%20one/schedules/schedule%2Fone{suffix} "
                )));
                let headers = request.to_ascii_lowercase();
                assert!(headers.contains(&format!("if-match: \"{}\"", revision - 1)));
                assert!(headers.contains("idempotency-key: mutation-key"));
                let body = format!(
                    r#"{{"schedule_id":"schedule/one","revision":{revision},"status":"{status}"}}"#
                );
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .expect("write response");
            }
        });

        let client = MitsuroClient::new(format!("http://{address}")).expect("client");
        let paused = client
            .pause_hive_schedule("session one", "schedule/one", 7, Some("mutation-key"))
            .await
            .expect("pause response");
        assert_eq!(paused.status, "paused");
        assert_eq!(paused.revision, 8);

        let resumed = client
            .resume_hive_schedule("session one", "schedule/one", 8, Some("mutation-key"))
            .await
            .expect("resume response");
        assert_eq!(resumed.status, "enabled");
        assert_eq!(resumed.revision, 9);

        let cancelled = client
            .cancel_hive_schedule("session one", "schedule/one", 9, Some("mutation-key"))
            .await
            .expect("cancel response");
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.revision, 10);
        server.join().expect("test server join");
    }

    #[tokio::test]
    async fn steer_posts_to_the_durable_chat_endpoint() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let size = socket.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.starts_with("POST /api/chat/steer "));
            assert!(request.contains("\"session_id\":\"session-1\""));
            assert!(request.contains("\"message\":\"focus on tests\""));
            let body = r#"{"status":"accepted","pending_id":"pending-1"}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .expect("write response");
        });

        let client = MitsuroClient::new(format!("http://{address}")).expect("client");
        let response = client
            .steer(SteerRequest {
                session_id: "session-1".to_owned(),
                message: "focus on tests".to_owned(),
                content: Vec::new(),
            })
            .await
            .expect("steer response");
        assert_eq!(response.status, "accepted");
        assert_eq!(response.pending_id, "pending-1");
        server.join().expect("test server join");
    }

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
