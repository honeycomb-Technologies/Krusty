//! Preview / port-forwarding endpoints.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::{
    body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Request, State,
    },
    http::{header, HeaderMap, HeaderName, Method, Response, Uri},
    response::IntoResponse,
    routing::{any, get},
    Json, Router,
};
use futures::{stream, SinkExt, StreamExt};
use reqwest::redirect::Policy;
use serde::Serialize;
use tokio_tungstenite::{connect_async, tungstenite::Message as UpstreamMessage};

use krusty_core::ports::{discover_listening_tcp_ports, TcpListenerInfo};
use krusty_core::process::ProcessInfo;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

use super::preview_settings::{load_preview_settings, PreviewSettings};

const MAX_PROXY_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;
const PORT_PROBE_CONCURRENCY: usize = 8;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_ports))
        .route("/:port/proxy", any(proxy_root))
        .route("/:port/proxy/*path", any(proxy_path))
}

#[derive(Debug, Clone, Serialize)]
struct PortEntry {
    port: u16,
    name: String,
    description: Option<String>,
    command: Option<String>,
    pid: Option<u32>,
    source: String,
    active: bool,
    pinned: bool,
    is_http_like: bool,
    is_previewable_http: bool,
    probe_status: ProbeStatus,
    last_probe_ms: Option<u32>,
    preview_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProbeStatus {
    Ok,
    Timeout,
    ConnRefused,
    NonHttp,
    Error,
}

#[derive(Debug, Clone, Copy)]
struct ProbeResult {
    is_previewable_http: bool,
    status: ProbeStatus,
    duration_ms: u32,
}

#[derive(Debug, Clone, Serialize)]
struct PortListResponse {
    ports: Vec<PortEntry>,
    settings: PreviewSettings,
    discovery_error: Option<String>,
}

#[derive(Debug)]
struct ProcessSearchEntry<'a> {
    process: &'a ProcessInfo,
    command_lower: String,
}

async fn list_ports(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<PortListResponse>, AppError> {
    let settings = load_preview_settings(&state, user.as_ref())?;
    if !settings.enabled {
        return Ok(Json(PortListResponse {
            ports: vec![],
            settings,
            discovery_error: None,
        }));
    }

    let (listeners, discovery_error) = match discover_listening_tcp_ports() {
        Ok(listeners) => (listeners, None),
        Err(err) => {
            tracing::warn!(
                "Port discovery failed, falling back to pinned only: {}",
                err
            );
            (
                vec![],
                Some("Port discovery failed. Showing pinned ports only.".to_string()),
            )
        }
    };
    let discovered_by_port: HashMap<u16, TcpListenerInfo> =
        listeners.into_iter().map(|l| (l.port, l)).collect();

    let tracked_processes = match user.as_ref().and_then(|u| u.0.user_id.as_deref()) {
        Some(user_id) => state.process_registry.list_for_user(user_id).await,
        None => state.process_registry.list().await,
    };
    let running_processes: Vec<ProcessInfo> = tracked_processes
        .into_iter()
        .filter(|p| p.is_running())
        .collect();
    let mut tracked_by_pid = HashMap::with_capacity(running_processes.len());
    let mut running_process_search = Vec::with_capacity(running_processes.len());
    for process in &running_processes {
        if let Some(pid) = process.pid {
            tracked_by_pid.insert(pid, process);
        }
        running_process_search.push(ProcessSearchEntry {
            process,
            command_lower: process.command.to_ascii_lowercase(),
        });
    }

    let blocked_ports: HashSet<u16> = settings.blocked_ports.iter().copied().collect();
    let hidden_ports: HashSet<u16> = settings.hidden_ports.iter().copied().collect();
    let pinned_ports: HashSet<u16> = settings.pinned_ports.iter().copied().collect();

    let mut candidate_ports: BTreeSet<u16> = discovered_by_port.keys().copied().collect();
    candidate_ports.extend(pinned_ports.iter().copied());

    let mut ports = Vec::with_capacity(candidate_ports.len());
    for port in candidate_ports {
        if port == state.server_port
            || blocked_ports.contains(&port)
            || hidden_ports.contains(&port)
        {
            continue;
        }

        let listener = discovered_by_port.get(&port);
        let pinned = pinned_ports.contains(&port);
        let active = listener.is_some();
        let source = match (active, pinned) {
            (true, true) => "discovered+pinned",
            (true, false) => "discovered",
            (false, true) => "pinned",
            (false, false) => "discovered",
        }
        .to_string();

        let process_hint =
            resolve_process_hint(port, listener, &tracked_by_pid, &running_process_search);
        let description = process_hint.and_then(|p| p.description.clone());
        let command = process_hint.map(|p| p.command.clone());
        let pid = process_hint.and_then(|p| p.pid);
        let name = infer_display_name(port, description.as_deref(), command.as_deref());
        let is_http_like = infer_http_like(
            port,
            name.as_str(),
            description.as_deref(),
            command.as_deref(),
        );

        if settings.show_only_http_like && !is_http_like && !pinned {
            continue;
        }

        ports.push(PortEntry {
            port,
            name,
            description,
            command,
            pid,
            source,
            active,
            pinned,
            is_http_like,
            is_previewable_http: false,
            probe_status: ProbeStatus::ConnRefused,
            last_probe_ms: None,
            preview_path: format!("/api/ports/{}/proxy", port),
        });
    }

    let probe_results = probe_ports_previewability(&ports, settings.probe_timeout_ms).await;
    for entry in &mut ports {
        if let Some(probe) = probe_results.get(&entry.port) {
            entry.is_previewable_http = probe.is_previewable_http;
            entry.probe_status = probe.status;
            entry.last_probe_ms = Some(probe.duration_ms);
        }
    }

    ports.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.active.cmp(&a.active))
            .then_with(|| a.port.cmp(&b.port))
    });

    Ok(Json(PortListResponse {
        ports,
        settings,
        discovery_error,
    }))
}

async fn proxy_root(
    State(state): State<AppState>,
    Path(port): Path<u16>,
    user: Option<CurrentUser>,
    ws: Option<WebSocketUpgrade>,
    method: Method,
    uri: Uri,
    request: Request,
) -> Result<Response<axum::body::Body>, AppError> {
    proxy_request(ProxyRequest {
        state,
        user,
        port,
        path: None,
        ws,
        method,
        uri,
        request,
    })
    .await
}

async fn proxy_path(
    State(state): State<AppState>,
    Path((port, path)): Path<(u16, String)>,
    user: Option<CurrentUser>,
    ws: Option<WebSocketUpgrade>,
    method: Method,
    uri: Uri,
    request: Request,
) -> Result<Response<axum::body::Body>, AppError> {
    proxy_request(ProxyRequest {
        state,
        user,
        port,
        path: Some(path),
        ws,
        method,
        uri,
        request,
    })
    .await
}

struct ProxyRequest {
    state: AppState,
    user: Option<CurrentUser>,
    port: u16,
    path: Option<String>,
    ws: Option<WebSocketUpgrade>,
    method: Method,
    uri: Uri,
    request: Request,
}

async fn proxy_request(input: ProxyRequest) -> Result<Response<axum::body::Body>, AppError> {
    let ProxyRequest {
        state,
        user,
        port,
        path,
        ws,
        method,
        uri,
        request,
    } = input;

    let settings = load_preview_settings(&state, user.as_ref())?;
    if !settings.enabled {
        return Err(AppError::BadRequest(
            "Preview forwarding is disabled in settings".to_string(),
        ));
    }
    if port == state.server_port {
        return Err(AppError::BadRequest(
            "Refusing to proxy the Krusty server port".to_string(),
        ));
    }
    if settings.is_blocked(port) {
        return Err(AppError::BadRequest(format!(
            "Port {} is blocked by preview settings",
            port
        )));
    }

    let upstream_path = build_upstream_path(path.as_deref(), uri.query());
    let upstream_http_url = format!("http://127.0.0.1:{}{}", port, upstream_path);

    let request_headers = request.headers().clone();
    let wants_ws = method == Method::GET && is_websocket_upgrade(&request_headers);
    if wants_ws {
        let Some(ws) = ws else {
            return Err(AppError::BadRequest(
                "WebSocket upgrade requested but upgrade failed".to_string(),
            ));
        };
        let upstream_ws_url = format!("ws://127.0.0.1:{}{}", port, upstream_path);
        return Ok(ws
            .on_upgrade(move |socket| proxy_websocket(socket, upstream_ws_url))
            .into_response());
    }

    proxy_http_request(method, upstream_http_url, request_headers, request).await
}

async fn proxy_http_request(
    method: Method,
    upstream_url: String,
    request_headers: HeaderMap,
    request: Request,
) -> Result<Response<axum::body::Body>, AppError> {
    let body_bytes = body::to_bytes(request.into_body(), MAX_PROXY_REQUEST_BODY_BYTES)
        .await
        .map_err(|e| AppError::BadRequest(format!("Request body too large: {}", e)))?;

    let mut upstream = proxy_http_client()
        .request(method, &upstream_url)
        .body(body_bytes);

    for (name, value) in &request_headers {
        if should_forward_request_header(name) {
            upstream = upstream.header(name, value);
        }
    }

    if let Some(host) = request_headers.get(header::HOST) {
        upstream = upstream.header("x-forwarded-host", host.clone());
    }
    upstream = upstream
        .header("x-forwarded-proto", "http")
        .header("x-forwarded-for", "127.0.0.1");

    let upstream_response = upstream.send().await.map_err(|e| {
        AppError::BadGateway(format!(
            "Failed to reach upstream on {}: {}",
            upstream_url, e
        ))
    })?;

    let status = upstream_response.status();
    let response_headers = upstream_response.headers().clone();
    let response_body = upstream_response.bytes().await.map_err(|e| {
        AppError::BadGateway(format!("Failed reading upstream response body: {}", e))
    })?;

    let mut response_builder = Response::builder().status(status);
    for (name, value) in &response_headers {
        if should_forward_response_header(name) {
            response_builder = response_builder.header(name, value);
        }
    }

    response_builder
        .body(axum::body::Body::from(response_body))
        .map_err(|e| AppError::Internal(format!("Failed to build proxy response: {}", e)))
}

async fn proxy_websocket(client_socket: WebSocket, upstream_url: String) {
    let upstream = connect_async(&upstream_url).await;
    let (upstream_socket, _) = match upstream {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                "Failed to connect upstream websocket for preview proxy ({}): {}",
                upstream_url,
                err
            );
            return;
        }
    };

    let (mut client_tx, mut client_rx) = client_socket.split();
    let (mut upstream_tx, mut upstream_rx) = upstream_socket.split();

    let client_to_upstream = async {
        while let Some(msg) = client_rx.next().await {
            let Ok(msg) = msg else {
                break;
            };
            match msg {
                Message::Text(text) => {
                    if upstream_tx
                        .send(UpstreamMessage::Text(text.to_string()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Message::Binary(binary) => {
                    if upstream_tx
                        .send(UpstreamMessage::Binary(binary.to_vec()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Message::Ping(data) => {
                    if upstream_tx
                        .send(UpstreamMessage::Ping(data.to_vec()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Message::Pong(data) => {
                    if upstream_tx
                        .send(UpstreamMessage::Pong(data.to_vec()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Message::Close(_) => {
                    let _ = upstream_tx.send(UpstreamMessage::Close(None)).await;
                    break;
                }
            }
        }
    };

    let upstream_to_client = async {
        while let Some(msg) = upstream_rx.next().await {
            let Ok(msg) = msg else {
                break;
            };
            match msg {
                UpstreamMessage::Text(text) => {
                    if client_tx
                        .send(Message::Text(text.to_string()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                UpstreamMessage::Binary(data) => {
                    if client_tx.send(Message::Binary(data)).await.is_err() {
                        break;
                    }
                }
                UpstreamMessage::Ping(data) => {
                    if client_tx.send(Message::Ping(data)).await.is_err() {
                        break;
                    }
                }
                UpstreamMessage::Pong(data) => {
                    if client_tx.send(Message::Pong(data)).await.is_err() {
                        break;
                    }
                }
                UpstreamMessage::Close(_) => {
                    let _ = client_tx.send(Message::Close(None)).await;
                    break;
                }
                UpstreamMessage::Frame(_) => {}
            }
        }
    };

    tokio::select! {
        _ = client_to_upstream => {}
        _ = upstream_to_client => {}
    }
}

fn proxy_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            .expect("proxy client should initialize")
    })
}

async fn probe_ports_previewability(
    entries: &[PortEntry],
    timeout_ms: u16,
) -> HashMap<u16, ProbeResult> {
    let timeout = Duration::from_millis(timeout_ms as u64);
    let ports: Vec<u16> = entries
        .iter()
        .filter(|p| p.active)
        .map(|p| p.port)
        .collect();
    if ports.is_empty() {
        return HashMap::new();
    }

    stream::iter(ports.into_iter())
        .map(|port| async move { (port, probe_previewable_port(port, timeout).await) })
        .buffer_unordered(PORT_PROBE_CONCURRENCY)
        .collect::<Vec<(u16, ProbeResult)>>()
        .await
        .into_iter()
        .collect()
}

async fn probe_previewable_port(port: u16, timeout: Duration) -> ProbeResult {
    let start = Instant::now();
    let request = proxy_http_client()
        .get(format!("http://127.0.0.1:{}/", port))
        .timeout(timeout)
        .send()
        .await;

    let elapsed = start.elapsed();
    let duration_ms = elapsed.as_millis().min(u128::from(u32::MAX)) as u32;

    match request {
        Ok(response) => {
            let _ = response.bytes().await;
            ProbeResult {
                is_previewable_http: true,
                status: ProbeStatus::Ok,
                duration_ms,
            }
        }
        Err(err) => {
            let err_message = err.to_string();
            let status = classify_probe_error(err.is_timeout(), err.is_connect(), &err_message);
            ProbeResult {
                is_previewable_http: false,
                status,
                duration_ms,
            }
        }
    }
}

fn classify_probe_error(is_timeout: bool, is_connect: bool, message: &str) -> ProbeStatus {
    let message_lower;
    let message_lower = if message.bytes().any(|byte| byte.is_ascii_uppercase()) {
        message_lower = message.to_ascii_lowercase();
        message_lower.as_str()
    } else {
        message
    };

    if is_timeout {
        return ProbeStatus::Timeout;
    }
    if is_connect {
        if message_lower.contains("connection refused")
            || message_lower.contains("failed to connect")
        {
            return ProbeStatus::ConnRefused;
        }
        if message_lower.contains("invalid http")
            || message_lower.contains("http parse")
            || message_lower.contains("connection closed before message completed")
        {
            return ProbeStatus::NonHttp;
        }
    }
    if message_lower.contains("invalid http")
        || message_lower.contains("http parse")
        || message_lower.contains("connection closed before message completed")
    {
        return ProbeStatus::NonHttp;
    }
    ProbeStatus::Error
}

fn resolve_process_hint<'a>(
    port: u16,
    listener: Option<&TcpListenerInfo>,
    tracked_by_pid: &'a HashMap<u32, &'a ProcessInfo>,
    running_processes: &'a [ProcessSearchEntry<'a>],
) -> Option<&'a ProcessInfo> {
    if let Some(listener) = listener {
        for pid in &listener.pids {
            if let Some(process) = tracked_by_pid.get(pid) {
                return Some(process);
            }
        }
    }

    let needle_colon = format!(":{}", port);
    let needle_port_eq = format!("--port={}", port);
    let needle_port_sep = format!("--port {}", port);
    let needle_short = format!("-p {}", port);

    running_processes
        .iter()
        .find(|entry| {
            let command = entry.command_lower.as_str();
            command.contains(&needle_colon)
                || command.contains(&needle_port_eq)
                || command.contains(&needle_port_sep)
                || command.contains(&needle_short)
        })
        .map(|entry| entry.process)
}

fn infer_display_name(port: u16, description: Option<&str>, command: Option<&str>) -> String {
    if let Some(description) = description.filter(|s| !s.trim().is_empty()) {
        return description.to_string();
    }

    if let Some(command) = command {
        const DISPLAY_HINTS: [(&str, &str); 9] = [
            ("vite", "Vite Dev Server"),
            ("next", "Next.js Dev Server"),
            ("webpack", "Webpack Dev Server"),
            ("astro", "Astro Dev Server"),
            ("nuxt", "Nuxt Dev Server"),
            ("storybook", "Storybook"),
            ("uvicorn", "Python Web Server"),
            ("gunicorn", "Python Web Server"),
            ("http.server", "Python HTTP Server"),
        ];
        let command = command.to_ascii_lowercase();
        for (needle, label) in DISPLAY_HINTS {
            if command.contains(needle) {
                return label.to_string();
            }
        }
    }

    format!("Port {}", port)
}

fn infer_http_like(
    port: u16,
    name: &str,
    description: Option<&str>,
    command: Option<&str>,
) -> bool {
    const COMMON_HTTP_PORTS: [u16; 18] = [
        80, 3000, 3001, 3002, 4000, 4173, 4200, 4321, 5000, 5173, 5174, 5175, 6006, 8000, 8080,
        8081, 8787, 9000,
    ];
    if COMMON_HTTP_PORTS.contains(&port) {
        return true;
    }

    let name = name.to_ascii_lowercase();
    let desc = description.unwrap_or_default().to_ascii_lowercase();
    let cmd = command.unwrap_or_default().to_ascii_lowercase();
    const HTTP_KEYWORDS: [&str; 13] = [
        "vite",
        "next",
        "webpack",
        "astro",
        "nuxt",
        "storybook",
        "serve",
        "http",
        "uvicorn",
        "gunicorn",
        "flask",
        "django",
        "rails",
    ];
    HTTP_KEYWORDS
        .iter()
        .any(|keyword| name.contains(keyword) || desc.contains(keyword) || cmd.contains(keyword))
}

fn build_upstream_path(path: Option<&str>, query: Option<&str>) -> String {
    let mut full = String::new();
    match path {
        Some(path) if !path.is_empty() => {
            full.push('/');
            full.push_str(path);
        }
        _ => full.push('/'),
    }
    if let Some(query) = query {
        full.push('?');
        full.push_str(query);
    }
    full
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let has_upgrade = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    let has_connection_upgrade = headers
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);

    has_upgrade && has_connection_upgrade
}

fn should_forward_request_header(name: &HeaderName) -> bool {
    !is_hop_by_hop_header(name) && *name != header::HOST
}

fn should_forward_response_header(name: &HeaderName) -> bool {
    !is_hop_by_hop_header(name)
}

fn is_hop_by_hop_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "proxy-connection"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn build_upstream_path_handles_root_and_query() {
        assert_eq!(build_upstream_path(None, None), "/");
        assert_eq!(build_upstream_path(Some("foo/bar"), None), "/foo/bar");
        assert_eq!(
            build_upstream_path(Some("foo"), Some("a=1&b=2")),
            "/foo?a=1&b=2"
        );
    }

    #[test]
    fn infer_http_like_prefers_common_dev_ports() {
        assert!(infer_http_like(5173, "Port 5173", None, None));
        assert!(infer_http_like(3000, "Port 3000", None, None));
        assert!(!infer_http_like(9922, "Port 9922", None, None));
    }

    #[test]
    fn infer_http_like_uses_command_and_description_keywords() {
        assert!(infer_http_like(
            9922,
            "Port 9922",
            Some("local vite frontend"),
            None
        ));
        assert!(infer_http_like(
            9922,
            "Port 9922",
            None,
            Some("uvicorn app.main:app --port 9922")
        ));
    }

    #[test]
    fn classify_probe_error_maps_timeout_connrefused_and_non_http() {
        assert_eq!(
            classify_probe_error(true, false, "deadline elapsed"),
            ProbeStatus::Timeout
        );
        assert_eq!(
            classify_probe_error(true, true, "connection refused"),
            ProbeStatus::Timeout
        );
        assert_eq!(
            classify_probe_error(false, true, "connection refused"),
            ProbeStatus::ConnRefused
        );
        assert_eq!(
            classify_probe_error(false, true, "invalid HTTP version parsed"),
            ProbeStatus::NonHttp
        );
        assert_eq!(
            classify_probe_error(false, false, "http parse error"),
            ProbeStatus::NonHttp
        );
    }

    #[tokio::test]
    async fn probe_previewable_port_accepts_http_statuses() {
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!(
                    "skipping probe_previewable_port_accepts_http_statuses: bind permission denied ({err})"
                );
                return;
            }
            Err(err) => panic!("bind test http listener: {err}"),
        };
        let port = listener.local_addr().expect("listener local addr").port();

        let server = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });

        let result = probe_previewable_port(port, Duration::from_millis(1000)).await;
        server.await.expect("join test http server");

        assert!(result.is_previewable_http);
        assert_eq!(result.status, ProbeStatus::Ok);
        assert!(result.duration_ms <= 1000);
    }
}
