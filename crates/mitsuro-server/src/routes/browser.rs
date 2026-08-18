//! Atlas: authenticated, server-owned Chromium sessions powered by agent-browser.
//!
//! The native stream and CDP endpoints stay loopback-only. Clients receive the
//! Mitsuro WebSocket proxy and a deliberately small semantic action surface.

use std::{
    collections::HashMap,
    env,
    path::{Path as FsPath, PathBuf},
    process::Stdio,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use axum::{
    extract::{
        ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
        Path, Query,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::AsyncWriteExt,
    process::Command,
    sync::{Mutex, RwLock},
    time::timeout,
};
use tokio_tungstenite::{connect_async, tungstenite::Message as UpstreamMessage};
use uuid::Uuid;

use mitsuro_core::tools::{registry::Tool, ToolContext, ToolRegistry, ToolResult};

use crate::{auth::CurrentUser, AppState};

const MAX_SESSIONS_PER_OWNER: usize = 6;
const MAX_ACTIONS: usize = 32;
const MAX_TEXT_BYTES: usize = 32 * 1024;
const PRESENCE_TTL: Duration = Duration::from_secs(20);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const AGENT_TIMEOUT: Duration = Duration::from_secs(180);

static ATLAS: OnceLock<AtlasRuntime> = OnceLock::new();

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_sessions).post(create_session))
        .route("/:id", get(get_session))
        .route("/:id/stop", post(stop_session))
        .route("/:id/heartbeat", post(heartbeat))
        .route("/:id/actions", post(run_actions))
        .route("/:id/stream", get(stream_session))
}

pub(crate) async fn register_tool(registry: &ToolRegistry) {
    registry.register(Arc::new(AtlasBrowserTool)).await;
}

struct AtlasBrowserTool;

#[derive(Debug, Deserialize)]
struct BrowserToolRequest {
    #[serde(default)]
    list_tabs: bool,
    tab_id: Option<String>,
    #[serde(default)]
    actions: Vec<BrowserAction>,
}

#[async_trait::async_trait]
impl Tool for AtlasBrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Inspect and control the user's visible Mitsuro Chromium tabs through semantic browser actions."
    }

    fn prompt(&self) -> Option<&str> {
        Some(
            "Use browser with list_tabs=true to discover the user's open visible tabs. Use a compact interactive snapshot before clicking or filling, then act only on references returned by that snapshot. This tool controls the same browser surface the user sees; do not start a separate browser agent chat.",
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "list_tabs": {
                    "type": "boolean",
                    "description": "List the user's visible browser tabs without performing actions."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Visible browser tab id. Omit to use the most recently active runnable tab."
                },
                "actions": {
                    "type": "array",
                    "maxItems": MAX_ACTIONS,
                    "description": "Ordered semantic browser actions.",
                    "items": {
                        "oneOf": [
                            {"type":"object","properties":{"type":{"const":"navigate"},"url":{"type":"string"}},"required":["type","url"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"const":"snapshot"},"interactive":{"type":"boolean"},"compact":{"type":"boolean"},"depth":{"type":"integer","minimum":1,"maximum":20}},"required":["type"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"const":"click"},"target":{"type":"string"}},"required":["type","target"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"enum":["fill","type"]},"target":{"type":"string"},"value":{"type":"string"}},"required":["type","target","value"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"const":"press"},"key":{"type":"string"}},"required":["type","key"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"const":"hover"},"target":{"type":"string"}},"required":["type","target"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"const":"select"},"target":{"type":"string"},"values":{"type":"array","items":{"type":"string"}}},"required":["type","target","values"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"const":"scroll"},"direction":{"enum":["up","down","left","right"]},"amount":{"type":"integer","minimum":1}},"required":["type","direction"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"enum":["back","forward","reload"]}},"required":["type"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"const":"wait"},"ms":{"type":"integer","minimum":0,"maximum":30000}},"required":["type","ms"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"const":"get"},"property":{"enum":["text","html","value","title","url","count"]},"target":{"type":"string"}},"required":["type","property"],"additionalProperties":false},
                            {"type":"object","properties":{"type":{"const":"attribute"},"target":{"type":"string"},"name":{"type":"string"}},"required":["type","target","name"],"additionalProperties":false}
                            ,{"type":"object","properties":{"type":{"const":"viewport"},"mode":{"enum":["mobile","desktop"]}},"required":["type","mode"],"additionalProperties":false}
                        ]
                    }
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let request = match serde_json::from_value::<BrowserToolRequest>(params) {
            Ok(request) => request,
            Err(error) => return ToolResult::invalid_parameters(error),
        };
        let owner = owner_key(ctx.user_id.as_deref());
        let records: Vec<_> = runtime().sessions.read().await.values().cloned().collect();
        let mut owned = Vec::new();
        for record in records.into_iter().filter(|record| record.owner == owner) {
            let updated_at = record.state.read().await.public.updated_at.clone();
            owned.push((updated_at, record));
        }
        owned.sort_by(|left, right| right.0.cmp(&left.0));

        if request.list_tabs {
            let mut tabs = Vec::new();
            for (_, record) in owned {
                tabs.push(public_session(&record).await);
            }
            return ToolResult::success_data(json!({ "tabs": tabs }));
        }
        if request.actions.is_empty() || request.actions.len() > MAX_ACTIONS {
            return ToolResult::invalid_parameters(format!(
                "browser actions must contain between 1 and {MAX_ACTIONS} items"
            ));
        }
        let record = if let Some(tab_id) = request.tab_id.as_deref() {
            let mut found = None;
            for (_, record) in owned {
                if record.state.read().await.public.id == tab_id {
                    found = Some(record);
                    break;
                }
            }
            found
        } else {
            let mut found = None;
            for (_, record) in owned {
                if matches!(
                    record.state.read().await.public.status,
                    BrowserSessionStatus::Ready | BrowserSessionStatus::Running
                ) {
                    found = Some(record);
                    break;
                }
            }
            found
        };
        let Some(record) = record else {
            return ToolResult::error_with_code(
                "browser_tab_not_found",
                "No runnable visible browser tab was found. Ask the user to open the Browser surface.",
            );
        };
        if let Err((_, Json(body))) = ensure_runnable(&record).await {
            return ToolResult::error_with_code(
                "browser_tab_not_runnable",
                body.get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Browser tab is not runnable"),
            );
        }
        let commands = match action_commands(&request.actions) {
            Ok(commands) => commands,
            Err((_, Json(body))) => {
                return ToolResult::invalid_parameters(
                    body.get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Invalid browser action"),
                )
            }
        };
        let input = match serde_json::to_vec(&commands) {
            Ok(input) => input,
            Err(error) => return ToolResult::error_with_code("browser_encode_failed", error),
        };
        {
            let mut state = record.state.write().await;
            state.public.status = BrowserSessionStatus::Running;
            state.public.updated_at = chrono::Utc::now().to_rfc3339();
        }
        match run_cli(
            &record,
            vec!["batch".into(), "--bail".into()],
            Some(input),
            AGENT_TIMEOUT,
        )
        .await
        {
            Ok(results) => {
                let mut state = record.state.write().await;
                state.public.status = BrowserSessionStatus::Ready;
                state.public.updated_at = chrono::Utc::now().to_rfc3339();
                if let Some(url) = latest_url(&results) {
                    state.public.url = Some(url);
                }
                if let Some(mode) = latest_viewport_mode(&request.actions) {
                    state.public.viewport_mode = mode;
                }
                ToolResult::success_data(json!({
                    "tab": state.public,
                    "results": results
                }))
            }
            Err(error) => {
                let mut state = record.state.write().await;
                state.public.status = BrowserSessionStatus::Ready;
                state.public.updated_at = chrono::Utc::now().to_rfc3339();
                state.public.last_error = Some(error.clone());
                ToolResult::error_with_code("browser_action_failed", error)
            }
        }
    }
}

fn runtime() -> &'static AtlasRuntime {
    ATLAS.get_or_init(AtlasRuntime::default)
}

#[derive(Default)]
struct AtlasRuntime {
    sessions: RwLock<HashMap<String, Arc<SessionRecord>>>,
}

struct SessionRecord {
    owner: String,
    session_name: String,
    state: RwLock<SessionState>,
    command_lock: Mutex<()>,
}

struct SessionState {
    public: BrowserSession,
    stream_port: Option<u16>,
    viewers: HashMap<String, Instant>,
    controller: Option<(String, Instant)>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserCapability {
    available: bool,
    runtime: &'static str,
    version: &'static str,
    executable: Option<String>,
    live_stream: bool,
    semantic_actions: bool,
    agent_chat: bool,
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserListResponse {
    sessions: Vec<BrowserSession>,
    capability: BrowserCapability,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserSession {
    id: String,
    title: String,
    kind: BrowserSessionKind,
    status: BrowserSessionStatus,
    url: Option<String>,
    /// Intentionally always absent: raw CDP is never a client capability.
    cdp_url: Option<String>,
    debug_port: Option<u16>,
    stream_url: Option<String>,
    viewers: usize,
    controllers: usize,
    last_error: Option<String>,
    created_at: String,
    updated_at: String,
    viewport_mode: ViewportMode,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ViewportMode {
    Mobile,
    Desktop,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum BrowserSessionKind {
    Interactive,
    Agent,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BrowserSessionStatus {
    Starting,
    Ready,
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Deserialize)]
struct CreateBrowserSession {
    title: Option<String>,
    #[serde(default = "interactive_kind")]
    kind: BrowserSessionKind,
    url: Option<String>,
    #[allow(dead_code)]
    launch_local: Option<bool>,
}

fn interactive_kind() -> BrowserSessionKind {
    BrowserSessionKind::Interactive
}

#[derive(Debug, Deserialize)]
struct HeartbeatRequest {
    #[serde(default)]
    capability: PresenceCapability,
    client_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PresenceCapability {
    #[default]
    Viewer,
    Controller,
}

#[derive(Debug, Deserialize)]
struct StreamQuery {
    #[serde(default)]
    capability: PresenceCapability,
    client_id: Option<String>,
    #[allow(dead_code)]
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ActionBatch {
    actions: Vec<BrowserAction>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrowserAction {
    Navigate {
        url: String,
    },
    Snapshot {
        #[serde(default)]
        interactive: bool,
        #[serde(default)]
        compact: bool,
        depth: Option<u16>,
    },
    Click {
        target: String,
    },
    Fill {
        target: String,
        value: String,
    },
    Type {
        target: String,
        value: String,
    },
    Press {
        key: String,
    },
    Hover {
        target: String,
    },
    Select {
        target: String,
        values: Vec<String>,
    },
    Scroll {
        direction: ScrollDirection,
        amount: Option<u32>,
    },
    Back,
    Forward,
    Reload,
    Wait {
        ms: u32,
    },
    Get {
        property: GetProperty,
        target: Option<String>,
    },
    Attribute {
        target: String,
        name: String,
    },
    Viewport {
        mode: ViewportMode,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GetProperty {
    Text,
    Html,
    Value,
    Title,
    Url,
    Count,
}

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

async fn list_sessions(CurrentUser(user): CurrentUser) -> Json<BrowserListResponse> {
    let owner = owner_key(user.user_id.as_deref());
    let records: Vec<_> = runtime().sessions.read().await.values().cloned().collect();
    let mut sessions = Vec::new();
    for record in records {
        if record.owner == owner {
            sessions.push(public_session(&record).await);
        }
    }
    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(BrowserListResponse {
        sessions,
        capability: browser_capability(),
    })
}

async fn create_session(
    CurrentUser(user): CurrentUser,
    Json(request): Json<CreateBrowserSession>,
) -> ApiResult<Json<BrowserSession>> {
    let owner = owner_key(user.user_id.as_deref());
    let url = normalize_url(request.url.as_deref().unwrap_or("about:blank"))?;
    let title = bounded_text(request.title.as_deref().unwrap_or("Browser"), 120)?;

    let owned_count = runtime()
        .sessions
        .read()
        .await
        .values()
        .filter(|record| record.owner == owner)
        .count();
    if owned_count >= MAX_SESSIONS_PER_OWNER {
        return Err(api_error(
            StatusCode::CONFLICT,
            "Browser tab limit reached; close an existing tab first",
        ));
    }

    let Some(_) = discover_agent_browser() else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            missing_runtime_message(),
        ));
    };

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let record = Arc::new(SessionRecord {
        owner,
        session_name: format!("atlas-{}", id.replace('-', "")),
        state: RwLock::new(SessionState {
            public: BrowserSession {
                id: id.clone(),
                title,
                kind: request.kind,
                status: BrowserSessionStatus::Starting,
                url: Some(url.clone()),
                cdp_url: None,
                debug_port: None,
                stream_url: None,
                viewers: 0,
                controllers: 0,
                last_error: None,
                created_at: now.clone(),
                updated_at: now,
                viewport_mode: ViewportMode::Mobile,
            },
            stream_port: None,
            viewers: HashMap::new(),
            controller: None,
        }),
        command_lock: Mutex::new(()),
    });
    runtime()
        .sessions
        .write()
        .await
        .insert(id.clone(), Arc::clone(&record));

    let launch_commands = serde_json::to_vec(&vec![
        vec!["open", url.as_str()],
        vec!["set", "device", "iPhone 12"],
        vec!["set", "viewport", "390", "700", "3"],
    ])
    .map_err(internal_error)?;
    let launch_result = run_cli(
        &record,
        vec!["batch".into(), "--bail".into()],
        Some(launch_commands),
        COMMAND_TIMEOUT,
    )
    .await;
    match launch_result {
        Ok(_) => {
            // agent-browser 0.34 starts its session-scoped stream with the
            // daemon. Query the assigned loopback port instead of trying to
            // enable an already-enabled server.
            let stream = run_cli(
                &record,
                vec!["stream".into(), "status".into()],
                None,
                COMMAND_TIMEOUT,
            )
            .await;
            let mut state = record.state.write().await;
            state.public.status = BrowserSessionStatus::Ready;
            state.public.updated_at = chrono::Utc::now().to_rfc3339();
            match stream.and_then(|value| {
                find_stream_port(&value)
                    .ok_or_else(|| "stream port missing from runtime response".into())
            }) {
                Ok(port) => {
                    state.stream_port = Some(port);
                    state.public.stream_url = Some(format!("/api/browser/{id}/stream"));
                }
                Err(error) => {
                    state.public.last_error = Some(format!("Live stream unavailable: {error}"))
                }
            }
        }
        Err(error) => {
            let mut state = record.state.write().await;
            state.public.status = BrowserSessionStatus::Error;
            state.public.last_error = Some(error);
            state.public.updated_at = chrono::Utc::now().to_rfc3339();
        }
    }

    Ok(Json(public_session(&record).await))
}

async fn get_session(
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<BrowserSession>> {
    let record = owned_session(&id, user.user_id.as_deref()).await?;
    Ok(Json(public_session(&record).await))
}

async fn stop_session(
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<BrowserSession>> {
    let record = owned_session(&id, user.user_id.as_deref()).await?;
    let result = run_cli(&record, vec!["close".into()], None, COMMAND_TIMEOUT).await;
    let mut state = record.state.write().await;
    state.public.updated_at = chrono::Utc::now().to_rfc3339();
    state.viewers.clear();
    state.controller = None;
    state.public.viewers = 0;
    state.public.controllers = 0;
    state.stream_port = None;
    state.public.stream_url = None;
    match result {
        Ok(_) => {
            state.public.status = BrowserSessionStatus::Stopped;
            state.public.last_error = None;
        }
        Err(error) => {
            state.public.status = BrowserSessionStatus::Error;
            state.public.last_error = Some(error);
        }
    }
    let response = state.public.clone();
    drop(state);
    runtime().sessions.write().await.remove(&id);
    Ok(Json(response))
}

async fn heartbeat(
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(request): Json<HeartbeatRequest>,
) -> ApiResult<Json<BrowserSession>> {
    let record = owned_session(&id, user.user_id.as_deref()).await?;
    claim_presence(
        &record,
        request.capability,
        request.client_id.as_deref(),
        user.user_id.as_deref(),
    )
    .await?;
    Ok(Json(public_session(&record).await))
}

async fn run_actions(
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Json(request): Json<ActionBatch>,
) -> ApiResult<Json<Value>> {
    if request.actions.is_empty() || request.actions.len() > MAX_ACTIONS {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "actions must contain between 1 and 32 entries",
        ));
    }
    let record = owned_session(&id, user.user_id.as_deref()).await?;
    ensure_runnable(&record).await?;
    let commands = action_commands(&request.actions)?;
    let input = serde_json::to_vec(&commands).map_err(internal_error)?;
    {
        let mut state = record.state.write().await;
        state.public.status = BrowserSessionStatus::Running;
        state.public.updated_at = chrono::Utc::now().to_rfc3339();
    }
    let output = run_cli(
        &record,
        vec!["batch".into(), "--bail".into()],
        Some(input),
        COMMAND_TIMEOUT,
    )
    .await
    .map_err(runtime_error)?;

    let mut state = record.state.write().await;
    state.public.status = BrowserSessionStatus::Ready;
    state.public.updated_at = chrono::Utc::now().to_rfc3339();
    if let Some(url) = latest_url(&output) {
        state.public.url = Some(url);
    }
    if let Some(mode) = latest_viewport_mode(&request.actions) {
        state.public.viewport_mode = mode;
    }
    Ok(Json(json!({ "ok": true, "results": output })))
}

async fn stream_session(
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<StreamQuery>,
    upgrade: WebSocketUpgrade,
) -> ApiResult<impl IntoResponse> {
    let record = owned_session(&id, user.user_id.as_deref()).await?;
    claim_presence(
        &record,
        query.capability,
        query.client_id.as_deref(),
        user.user_id.as_deref(),
    )
    .await?;
    let stream_port =
        record.state.read().await.stream_port.ok_or_else(|| {
            api_error(StatusCode::CONFLICT, "Browser live stream is not available")
        })?;
    let controller = matches!(query.capability, PresenceCapability::Controller);
    Ok(upgrade.on_upgrade(move |socket| proxy_stream(socket, stream_port, controller)))
}

async fn proxy_stream(client: WebSocket, stream_port: u16, controller: bool) {
    let upstream_url = format!("ws://127.0.0.1:{stream_port}/?pacing=ack&maxFps=30");
    let Ok((upstream, _)) = connect_async(&upstream_url).await else {
        tracing::warn!(
            port = stream_port,
            "Browser stream proxy could not reach runtime"
        );
        return;
    };
    let (mut client_tx, mut client_rx) = client.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();

    loop {
        tokio::select! {
            from_runtime = upstream_rx.next() => {
                let Some(Ok(message)) = from_runtime else { break };
                let mapped = match message {
                    UpstreamMessage::Text(value) => AxumMessage::Text(value),
                    UpstreamMessage::Binary(value) => AxumMessage::Binary(value),
                    UpstreamMessage::Ping(value) => AxumMessage::Ping(value),
                    UpstreamMessage::Pong(value) => AxumMessage::Pong(value),
                    UpstreamMessage::Close(_) => AxumMessage::Close(None),
                    UpstreamMessage::Frame(_) => continue,
                };
                if client_tx.send(mapped).await.is_err() { break; }
            }
            from_client = client_rx.next() => {
                let Some(Ok(message)) = from_client else { break };
                let mapped = match message {
                    AxumMessage::Text(value) if stream_input_allowed(&value, controller) => UpstreamMessage::Text(value),
                    AxumMessage::Binary(value) if controller => UpstreamMessage::Binary(value),
                    AxumMessage::Ping(value) => UpstreamMessage::Ping(value),
                    AxumMessage::Pong(value) => UpstreamMessage::Pong(value),
                    AxumMessage::Close(_) => UpstreamMessage::Close(None),
                    _ => continue,
                };
                if upstream_tx.send(mapped).await.is_err() { break; }
            }
        }
    }
}

fn stream_input_allowed(text: &str, controller: bool) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("config" | "ack") => true,
        Some("input_mouse" | "input_keyboard" | "input_touch") => controller,
        _ => false,
    }
}

async fn owned_session(id: &str, user_id: Option<&str>) -> ApiResult<Arc<SessionRecord>> {
    let record = runtime()
        .sessions
        .read()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Browser tab not found"))?;
    if record.owner != owner_key(user_id) {
        return Err(api_error(StatusCode::NOT_FOUND, "Browser tab not found"));
    }
    Ok(record)
}

async fn public_session(record: &SessionRecord) -> BrowserSession {
    let now = Instant::now();
    let mut state = record.state.write().await;
    state
        .viewers
        .retain(|_, seen| now.duration_since(*seen) <= PRESENCE_TTL);
    if state
        .controller
        .as_ref()
        .is_some_and(|(_, seen)| now.duration_since(*seen) > PRESENCE_TTL)
    {
        state.controller = None;
    }
    state.public.viewers = state.viewers.len();
    state.public.controllers = usize::from(state.controller.is_some());
    state.public.clone()
}

async fn claim_presence(
    record: &SessionRecord,
    capability: PresenceCapability,
    client_id: Option<&str>,
    user_id: Option<&str>,
) -> ApiResult<()> {
    let fallback = owner_key(user_id);
    let client_id = bounded_text(client_id.unwrap_or(&fallback), 128)?;
    let now = Instant::now();
    let mut state = record.state.write().await;
    state
        .viewers
        .retain(|_, seen| now.duration_since(*seen) <= PRESENCE_TTL);
    if state
        .controller
        .as_ref()
        .is_some_and(|(_, seen)| now.duration_since(*seen) > PRESENCE_TTL)
    {
        state.controller = None;
    }
    match capability {
        PresenceCapability::Viewer => {
            state.viewers.insert(client_id, now);
        }
        PresenceCapability::Controller => {
            if let Some((current, _)) = &state.controller {
                if current != &client_id {
                    return Err(api_error(
                        StatusCode::CONFLICT,
                        "Another Browser controller currently holds the lease",
                    ));
                }
            }
            state.controller = Some((client_id, now));
        }
    }
    Ok(())
}

async fn ensure_runnable(record: &SessionRecord) -> ApiResult<()> {
    let status = record.state.read().await.public.status;
    if matches!(
        status,
        BrowserSessionStatus::Stopped | BrowserSessionStatus::Error
    ) {
        Err(api_error(
            StatusCode::CONFLICT,
            "Browser tab is not running",
        ))
    } else {
        Ok(())
    }
}

async fn run_cli(
    record: &SessionRecord,
    args: Vec<String>,
    stdin: Option<Vec<u8>>,
    deadline: Duration,
) -> Result<Value, String> {
    let _guard = record.command_lock.lock().await;
    let executable =
        discover_agent_browser().ok_or_else(|| "agent-browser runtime not found".to_string())?;
    let mut command = Command::new(executable);
    command
        .arg("--namespace")
        .arg("mitsuro-atlas")
        .arg("--session")
        .arg(&record.session_name)
        .arg("--json")
        .arg("--content-boundaries")
        .arg("--max-output")
        .arg("200000")
        .arg("--idle-timeout")
        .arg("30m")
        .arg("--confirm-actions")
        .arg("eval,download")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start Browser runtime: {error}"))?;
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .ok_or_else(|| "Browser runtime stdin unavailable".to_string())?
            .write_all(&input)
            .await
            .map_err(|error| format!("could not write Browser command batch: {error}"))?;
    }
    let output = timeout(deadline, child.wait_with_output())
        .await
        .map_err(|_| "Browser runtime command timed out".to_string())?
        .map_err(|error| format!("Browser runtime failed: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let parsed =
        serde_json::from_str::<Value>(&stdout).unwrap_or_else(|_| json!({ "text": stdout }));
    if !output.status.success() || parsed.get("success") == Some(&Value::Bool(false)) {
        let message = parsed
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| (!stderr.is_empty()).then_some(stderr.as_str()))
            .unwrap_or("Browser runtime command failed");
        return Err(message.to_string());
    }
    Ok(parsed)
}

fn action_command(action: &BrowserAction) -> ApiResult<Vec<String>> {
    let command = match action {
        BrowserAction::Navigate { url } => vec!["open".into(), normalize_url(url)?],
        BrowserAction::Snapshot {
            interactive,
            compact,
            depth,
        } => {
            let mut args = vec!["snapshot".into()];
            if *interactive {
                args.push("-i".into());
            }
            if *compact {
                args.push("-c".into());
            }
            if let Some(depth) = depth {
                args.extend(["-d".into(), depth.min(&64).to_string()]);
            }
            args
        }
        BrowserAction::Click { target } => vec!["click".into(), safe_target(target)?],
        BrowserAction::Fill { target, value } => vec![
            "fill".into(),
            safe_target(target)?,
            bounded_text(value, MAX_TEXT_BYTES)?,
        ],
        BrowserAction::Type { target, value } => vec![
            "type".into(),
            safe_target(target)?,
            bounded_text(value, MAX_TEXT_BYTES)?,
        ],
        BrowserAction::Press { key } => vec!["press".into(), safe_argument(key, 100)?],
        BrowserAction::Hover { target } => vec!["hover".into(), safe_target(target)?],
        BrowserAction::Select { target, values } => {
            if values.is_empty() || values.len() > 32 {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "select requires 1 to 32 values",
                ));
            }
            let mut args = vec!["select".into(), safe_target(target)?];
            for value in values {
                args.push(safe_argument(value, 1024)?);
            }
            args
        }
        BrowserAction::Scroll { direction, amount } => {
            let direction = match direction {
                ScrollDirection::Up => "up",
                ScrollDirection::Down => "down",
                ScrollDirection::Left => "left",
                ScrollDirection::Right => "right",
            };
            let mut args = vec!["scroll".into(), direction.into()];
            if let Some(amount) = amount {
                args.push(amount.min(&100_000).to_string());
            }
            args
        }
        BrowserAction::Back => vec!["back".into()],
        BrowserAction::Forward => vec!["forward".into()],
        BrowserAction::Reload => vec!["reload".into()],
        BrowserAction::Wait { ms } => vec!["wait".into(), ms.min(&30_000).to_string()],
        BrowserAction::Get { property, target } => {
            let page_property = matches!(property, GetProperty::Title | GetProperty::Url);
            if !page_property && target.is_none() {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "element queries require a target",
                ));
            }
            let property = match property {
                GetProperty::Text => "text",
                GetProperty::Html => "html",
                GetProperty::Value => "value",
                GetProperty::Title => "title",
                GetProperty::Url => "url",
                GetProperty::Count => "count",
            };
            let mut args = vec!["get".into(), property.into()];
            if let Some(target) = target {
                args.push(safe_target(target)?);
            }
            args
        }
        BrowserAction::Attribute { target, name } => vec![
            "get".into(),
            "attr".into(),
            safe_target(target)?,
            safe_argument(name, 256)?,
        ],
        BrowserAction::Viewport { mode } => match mode {
            ViewportMode::Mobile => {
                vec!["set".into(), "device".into(), "iPhone 12".into()]
            }
            ViewportMode::Desktop => {
                vec!["set".into(), "viewport".into(), "1440".into(), "900".into()]
            }
        },
    };
    Ok(command)
}

fn action_commands(actions: &[BrowserAction]) -> ApiResult<Vec<Vec<String>>> {
    let mut commands = Vec::with_capacity(actions.len() + 1);
    for action in actions {
        commands.push(action_command(action)?);
        if matches!(
            action,
            BrowserAction::Viewport {
                mode: ViewportMode::Mobile
            }
        ) {
            commands.push(vec![
                "set".into(),
                "viewport".into(),
                "390".into(),
                "700".into(),
                "3".into(),
            ]);
        }
    }
    Ok(commands)
}

fn latest_viewport_mode(actions: &[BrowserAction]) -> Option<ViewportMode> {
    actions.iter().rev().find_map(|action| match action {
        BrowserAction::Viewport { mode } => Some(*mode),
        _ => None,
    })
}

fn browser_capability() -> BrowserCapability {
    let executable = discover_agent_browser();
    BrowserCapability {
        available: executable.is_some(),
        runtime: "agent-browser",
        version: "0.34.0",
        executable: executable.as_ref().map(|path| path.display().to_string()),
        live_stream: executable.is_some(),
        semantic_actions: executable.is_some(),
        agent_chat: executable.is_some(),
        reason: executable.is_none().then(missing_runtime_message),
    }
}

fn missing_runtime_message() -> String {
    "Browser runtime is not installed. Honey: the linux archive ships agent-browser beside mitsuro; run scripts/honey-atlas-repair.sh or set MITSURO_AGENT_BROWSER_PATH. Source checkouts: sh scripts/install-atlas-runtime.sh".to_string()
}

fn discover_agent_browser() -> Option<PathBuf> {
    first_existing_file(agent_browser_candidates())
}

fn agent_browser_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("MITSURO_AGENT_BROWSER_PATH").map(PathBuf::from) {
        candidates.push(path);
    }
    if let Ok(current) = env::current_exe() {
        candidates.extend(sibling_agent_browser_paths(&current));
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        candidates.extend(managed_install_agent_browser_paths(&home));
    }
    let workspace = FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    candidates.push(workspace.join("target/atlas").join(binary_name()));
    candidates.push(
        workspace
            .join("tools/atlas/node_modules/agent-browser/bin")
            .join(package_binary_name()),
    );
    if let Some(paths) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&paths).map(|path| path.join(binary_name())));
    }
    candidates
}

fn sibling_agent_browser_paths(current_exe: &FsPath) -> Vec<PathBuf> {
    let directory = current_exe.parent().unwrap_or_else(|| FsPath::new("."));
    vec![
        directory.join(binary_name()),
        directory.join("libexec/mitsuro").join(binary_name()),
    ]
}

fn managed_install_agent_browser_paths(home: &FsPath) -> Vec<PathBuf> {
    vec![
        home.join(".local/bin/.mitsuro-current").join(binary_name()),
        home.join(".local/bin").join(binary_name()),
        home.join(".local/lib/mitsuro").join(binary_name()),
    ]
}

fn first_existing_file(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|path| path.is_file())
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "agent-browser.exe"
    } else {
        "agent-browser"
    }
}

fn package_binary_name() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "agent-browser-linux-x64"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "agent-browser-linux-arm64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "agent-browser-darwin-x64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "agent-browser-darwin-arm64"
    } else if cfg!(windows) {
        "agent-browser-win32-x64.exe"
    } else {
        "agent-browser"
    }
}

fn normalize_url(input: &str) -> ApiResult<String> {
    let input = bounded_text(input.trim(), 8 * 1024)?;
    if input == "about:blank" || input.starts_with("http://") || input.starts_with("https://") {
        Ok(input)
    } else {
        Err(api_error(
            StatusCode::BAD_REQUEST,
            "Browser URLs must use http or https",
        ))
    }
}

fn safe_target(value: &str) -> ApiResult<String> {
    safe_argument(value, 4 * 1024)
}

fn safe_argument(value: &str, max: usize) -> ApiResult<String> {
    let value = bounded_text(value, max)?;
    if value.starts_with('-') {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Browser arguments cannot begin with a flag",
        ));
    }
    Ok(value)
}

fn bounded_text(value: &str, max: usize) -> ApiResult<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > max || value.contains('\0') {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Browser input is empty or too large",
        ));
    }
    Ok(value.to_string())
}

fn find_stream_port(value: &Value) -> Option<u16> {
    value
        .pointer("/data/port")
        .or_else(|| value.get("port"))
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
}

fn latest_url(value: &Value) -> Option<String> {
    value
        .as_array()?
        .iter()
        .rev()
        .find_map(|item| item.pointer("/data/url").and_then(Value::as_str))
        .map(str::to_string)
}

fn owner_key(user_id: Option<&str>) -> String {
    user_id.unwrap_or("local").to_string()
}

fn api_error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.into() })))
}

fn runtime_error(message: String) -> (StatusCode, Json<Value>) {
    api_error(StatusCode::BAD_GATEWAY, message)
}

fn internal_error(error: serde_json::Error) -> (StatusCode, Json<Value>) {
    tracing::error!(%error, "Could not encode Browser command batch");
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Could not encode Browser command batch",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_flag_shaped_targets() {
        let action = BrowserAction::Click {
            target: "--eval".into(),
        };
        assert!(action_command(&action).is_err());
    }

    #[test]
    fn semantic_actions_are_argument_arrays() {
        let action = BrowserAction::Fill {
            target: "@e2".into(),
            value: "hello world".into(),
        };
        assert_eq!(
            action_command(&action).unwrap(),
            vec!["fill", "@e2", "hello world"]
        );
    }

    #[test]
    fn viewport_actions_use_real_browser_dimensions() {
        assert_eq!(
            action_command(&BrowserAction::Viewport {
                mode: ViewportMode::Mobile,
            })
            .unwrap(),
            vec!["set", "device", "iPhone 12"]
        );
        assert_eq!(
            action_command(&BrowserAction::Viewport {
                mode: ViewportMode::Desktop,
            })
            .unwrap(),
            vec!["set", "viewport", "1440", "900"]
        );
        assert_eq!(
            action_commands(&[BrowserAction::Viewport {
                mode: ViewportMode::Mobile,
            }])
            .unwrap(),
            vec![
                vec!["set", "device", "iPhone 12"],
                vec!["set", "viewport", "390", "700", "3"],
            ]
        );
    }

    #[test]
    fn navigation_accepts_local_and_public_web_pages() {
        for url in ["http://127.0.0.1:5173", "https://example.com/docs"] {
            let action = BrowserAction::Navigate { url: url.into() };
            assert_eq!(action_command(&action).unwrap(), vec!["open", url]);
        }

        let action = BrowserAction::Navigate {
            url: "file:///etc/passwd".into(),
        };
        assert!(action_command(&action).is_err());
    }

    #[test]
    fn stream_proxy_filters_controller_input() {
        assert!(stream_input_allowed(r#"{"type":"ack","seq":1}"#, false));
        assert!(!stream_input_allowed(r#"{"type":"input_mouse"}"#, false));
        assert!(stream_input_allowed(r#"{"type":"input_mouse"}"#, true));
        assert!(!stream_input_allowed(r#"{"type":"eval"}"#, true));
    }

    #[test]
    fn finds_nested_stream_port() {
        assert_eq!(
            find_stream_port(&json!({"data":{"port":43123}})),
            Some(43123)
        );
    }

    #[test]
    fn first_existing_file_skips_missing_candidates() {
        let dir = std::env::temp_dir().join(format!(
            "mitsuro-atlas-discover-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing");
        let present = dir.join("agent-browser");
        std::fs::write(&present, b"ok").unwrap();
        assert_eq!(
            first_existing_file([missing, present.clone()]),
            Some(present)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn managed_install_paths_include_current_release_sidecar() {
        let home = FsPath::new("/home/honey");
        let paths = managed_install_agent_browser_paths(home);
        assert!(paths
            .iter()
            .any(|path| { path.ends_with(".local/bin/.mitsuro-current/agent-browser") }));
        assert!(paths
            .iter()
            .any(|path| path.ends_with(".local/lib/mitsuro/agent-browser")));
    }

    #[test]
    fn sibling_discovery_looks_beside_the_running_binary() {
        let paths = sibling_agent_browser_paths(FsPath::new(
            "/home/honey/.local/bin/.mitsuro-releases/v0.9.23/mitsuro",
        ));
        assert_eq!(
            paths[0],
            PathBuf::from("/home/honey/.local/bin/.mitsuro-releases/v0.9.23/agent-browser")
        );
    }
}
