//! Krusty Server
//!
//! Self-hosted API server for chat, tools, sessions, and local workspace access.
//! This is a library crate — the server is started via `start_server()`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{header, Method, Response, StatusCode, Uri},
    middleware,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use rust_embed::Embed;
use serde::Serialize;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    trace::TraceLayer,
};

use krusty_core::agent::{
    AgentCancellation, LoggingHook, PlanModeHook, SafetyHook, UserHookManager, UserPostToolHook,
    UserPreToolHook,
};
use krusty_core::ai::client::AiClient;
use krusty_core::ai::models::{create_model_registry, SharedModelRegistry};
use krusty_core::mcp::McpManager;
use krusty_core::paths;
use krusty_core::process::ProcessRegistry;
use krusty_core::skills::SkillsManager;
use krusty_core::storage::credentials::CredentialStore;
use krusty_core::storage::Database;
use krusty_core::tools::{
    register_agent_tool, register_all_tools, register_mako_tools, ToolRegistry,
};

use self::ai_bootstrap::{
    create_ai_client, create_ai_client_for_model, initialize_models, spawn_model_catalog_refresh,
};

type SessionGuard = Arc<Mutex<()>>;
const SESSION_LOCK_MAX_ENTRIES: usize = 1000;
const SESSION_LOCK_MAX_AGE: Duration = Duration::from_secs(3600);
mod ai_bootstrap;
type SessionLockMap = HashMap<String, (SessionGuard, Instant)>;
type SessionInputMap =
    HashMap<String, tokio::sync::mpsc::UnboundedSender<krusty_core::agent::LoopInput>>;
type SessionPresenceMap = presence::SessionPresenceMap;
type DelegatedStateMap = HashMap<String, Vec<types::DelegatedToolStateResponse>>;
pub mod apns;
pub mod auth;
pub mod error;
pub mod mako_runtime;
pub mod notifications;
pub(crate) mod oauth_flow;
pub mod presence;
pub mod push;
pub mod remote_access;
pub mod routes;
pub mod types;
pub mod utils;
pub mod ws;

/// Embedded web frontend assets.
///
/// At compile time, rust-embed includes all files from the Expo web build directory.
/// When the build directory is absent, this will be empty and the server
/// gracefully falls back to API-only mode.
#[derive(Embed)]
#[folder = "../../apps/mobile/dist"]
#[prefix = ""]
#[allow_missing = true]
struct WebAssets;

pub(crate) const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 100 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct ServerHttpPolicy {
    allow_any_browser_origin: bool,
    max_request_body_bytes: usize,
    immutable_asset_max_age_secs: u64,
    mutable_asset_max_age_secs: u64,
}

impl Default for ServerHttpPolicy {
    fn default() -> Self {
        Self {
            allow_any_browser_origin: false,
            max_request_body_bytes: DEFAULT_MAX_REQUEST_BODY_BYTES,
            immutable_asset_max_age_secs: 31_536_000,
            mutable_asset_max_age_secs: 3_600,
        }
    }
}

impl ServerHttpPolicy {
    fn cors_layer(self) -> CorsLayer {
        let cors = CorsLayer::new()
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PATCH,
                Method::PUT,
                Method::DELETE,
            ])
            .allow_headers(Any);

        if self.allow_any_browser_origin {
            cors.allow_origin(Any)
        } else {
            cors.allow_origin(AllowOrigin::predicate(|origin, _request_parts| {
                origin
                    .to_str()
                    .ok()
                    .is_some_and(crate::auth::is_trusted_local_origin)
            }))
        }
    }

    fn cache_control(self, path: &str) -> String {
        if path.contains("/_app/immutable/") || path.starts_with("_expo/static/") {
            format!(
                "public, max-age={}, immutable",
                self.immutable_asset_max_age_secs
            )
        } else if path.ends_with(".html") {
            "no-cache".to_string()
        } else {
            format!("public, max-age={}", self.mutable_asset_max_age_secs)
        }
    }
}

/// Configuration for starting the server.
pub struct ServerConfig {
    /// Port to listen on (default: 3000).
    pub port: u16,
    /// Working directory for file/tools APIs.
    pub working_dir: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3000,
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub server_port: u16,
    pub db_path: Arc<PathBuf>,
    pub working_dir: Arc<PathBuf>,
    pub ai_client: Option<Arc<AiClient>>,
    pub tool_registry: Arc<ToolRegistry>,
    pub process_registry: Arc<ProcessRegistry>,
    pub model_registry: SharedModelRegistry,
    pub credential_store: Arc<RwLock<CredentialStore>>,
    pub mcp_manager: Arc<McpManager>,
    pub hook_manager: Arc<RwLock<UserHookManager>>,
    pub skills_manager: Arc<RwLock<SkillsManager>>,
    pub cancellation: AgentCancellation,
    /// Per-session locks to prevent concurrent agentic loops on the same session.
    pub session_locks: Arc<RwLock<SessionLockMap>>,
    /// Active orchestrator input channels for tool approvals / cancellation.
    pub session_inputs: Arc<RwLock<SessionInputMap>>,
    /// Presence registry for active viewers/controllers per session.
    pub session_presence: Arc<RwLock<SessionPresenceMap>>,
    /// Active delegated tool snapshots per session for reconnect/reload parity.
    pub delegated_state: Arc<RwLock<DelegatedStateMap>>,
    /// Cached remote-access authority configuration.
    pub remote_access: Arc<RwLock<remote_access::RemoteAccessConfig>>,
    /// Count of currently active agent SSE streams.
    pub active_agent_streams: Arc<AtomicUsize>,
    /// Peak observed RSS bytes for this server process.
    pub peak_rss_bytes: Arc<AtomicU64>,
    /// Peak observed virtual bytes for this server process.
    pub peak_virtual_bytes: Arc<AtomicU64>,
    /// Web Push notification service (None if VAPID init failed).
    pub push_service: Option<Arc<push::PushService>>,
    /// APNs (Apple Push Notification service) for iOS devices.
    pub apns_service: Option<Arc<apns::ApnsService>>,
    /// Active OAuth flows keyed by provider storage key.
    pub oauth_flows: Arc<Mutex<HashMap<String, oauth_flow::OAuthFlowState>>>,
    /// Background runtime owner for autonomous Mako sessions.
    pub mako_runtime: Arc<mako_runtime::MakoRuntimeManager>,
}

impl AppState {
    /// Acquire the canonical per-session mutation lock without waiting.
    /// Chat, autonomous runs, and manual compaction all share this guard.
    pub(crate) async fn try_lock_session(&self, session_id: &str) -> Option<OwnedMutexGuard<()>> {
        let lock = {
            let mut locks = self.session_locks.write().await;
            if locks.len() > SESSION_LOCK_MAX_ENTRIES {
                locks.retain(|_, (lock, created_at)| {
                    created_at.elapsed() < SESSION_LOCK_MAX_AGE || Arc::strong_count(lock) > 1
                });
            }
            let (lock, _) = locks
                .entry(session_id.to_string())
                .or_insert_with(|| (Arc::new(Mutex::new(())), Instant::now()));
            Arc::clone(lock)
        };

        lock.try_lock_owned().ok()
    }

    /// Resolve a fresh AI client using the current credential store and requested model.
    pub async fn resolve_ai_client(&self, requested_model: Option<&str>) -> Option<Arc<AiClient>> {
        self.resolve_ai_client_for_user(requested_model, None).await
    }

    pub async fn resolve_ai_client_for_user(
        &self,
        requested_model: Option<&str>,
        user_id: Option<&str>,
    ) -> Option<Arc<AiClient>> {
        let credentials = self.credential_store.read().await.clone();
        let client = create_ai_client_for_model(
            &credentials,
            &self.model_registry,
            self.db_path.as_ref().as_path(),
            requested_model,
            user_id,
        )
        .await
        .map(Arc::new)?;
        self.ensure_agent_tool_registered(client.clone()).await;
        Some(client)
    }

    async fn ensure_agent_tool_registered(&self, client: Arc<AiClient>) {
        if self.tool_registry.get("agent").await.is_some() {
            return;
        }

        register_agent_tool(&self.tool_registry, client, self.cancellation.clone()).await;
        tracing::info!("Registered unified agent sub-agent tool");
    }
}

fn register_autonomous_classifier_hook(
    tool_registry_inner: &mut ToolRegistry,
    ai_client: Option<Arc<AiClient>>,
) {
    use krusty_core::agent::autonomy::auto_classifier::AutoClassifierHook;

    let hook = match ai_client {
        Some(client) => AutoClassifierHook::new(client),
        None => AutoClassifierHook::without_bootstrap_client(),
    };
    tool_registry_inner.add_pre_hook(Arc::new(hook));
}

/// Build the Axum router with all routes and embedded web assets.
pub async fn build_router(config: &ServerConfig) -> anyhow::Result<(Router, AppState)> {
    let http_policy = ServerHttpPolicy::default();
    let db_path = paths::config_dir().join("krusty.db");
    let _db = Database::new(&db_path)?;
    reconcile_transient_agent_states(&db_path)?;

    let credential_store_inner = CredentialStore::load().unwrap_or_default();
    let credential_store = Arc::new(RwLock::new(credential_store_inner.clone()));
    let model_registry = create_model_registry();
    initialize_models(&model_registry, &db_path).await;
    let ai_client = create_ai_client(&credential_store_inner, &model_registry, &db_path)
        .await
        .map(Arc::new);

    let process_registry = Arc::new(ProcessRegistry::new());
    let cancellation = AgentCancellation::new();

    // Load user hooks from database
    let mut hook_manager_inner = UserHookManager::new();
    if let Ok(db) = Database::new(&db_path) {
        if let Err(e) = hook_manager_inner.load(&db) {
            tracing::warn!("Failed to load hooks: {}", e);
        }
    }
    let hook_manager = Arc::new(RwLock::new(hook_manager_inner));

    // Build tool registry with full hook chain (matches TUI's init_tool_registry)
    let mut tool_registry_inner = ToolRegistry::new();
    tool_registry_inner.add_pre_hook(Arc::new(SafetyHook::new()));
    tool_registry_inner.add_pre_hook(Arc::new(PlanModeHook::new()));
    // Auto-classifier for Mako autonomous sessions (no-op when permission_mode != Autonomous).
    // Register even when no bootstrap client exists so later per-user Mako clients
    // are classified via ToolContext instead of bypassing the hook chain.
    register_autonomous_classifier_hook(&mut tool_registry_inner, ai_client.clone());
    tool_registry_inner.add_post_hook(Arc::new(LoggingHook::new()));
    tool_registry_inner.add_pre_hook(Arc::new(UserPreToolHook::new(hook_manager.clone())));
    tool_registry_inner.add_post_hook(Arc::new(UserPostToolHook::new(hook_manager.clone())));
    let tool_registry = Arc::new(tool_registry_inner);
    register_all_tools(&tool_registry).await;
    register_mako_tools(&tool_registry).await;

    // Register unified agent tool (explore, plan, verify, build) if AI client is available
    if let Some(ref client) = ai_client {
        register_agent_tool(&tool_registry, client.clone(), cancellation.clone()).await;
        tracing::info!("Registered unified agent sub-agent tool");
    }

    // MCP server connections + tool registration
    let mcp_manager = Arc::new(McpManager::new(config.working_dir.clone()));
    if let Err(e) = mcp_manager.load_config().await {
        tracing::warn!("Failed to load MCP config: {}", e);
    } else if let Err(e) = mcp_manager.connect_all().await {
        tracing::warn!("Failed to connect MCP servers: {}", e);
    }
    // Register MCP tools so they're visible to the AI
    krusty_core::mcp::tool::register_mcp_tools(mcp_manager.clone(), &tool_registry).await;
    let mcp_tool_count = tool_registry.get_ai_tools_all().await.len();
    tracing::info!("Tool registry initialized with {} tools", mcp_tool_count);

    let push_service =
        match push::PushService::init(&paths::vapid_key_path(), Arc::new(db_path.clone())) {
            Ok(svc) => {
                tracing::info!("Web Push service initialized");
                Some(Arc::new(svc))
            }
            Err(e) => {
                tracing::warn!("Push notifications unavailable: {}", e);
                None
            }
        };
    let apns_service = apns::ApnsService::from_env(Arc::new(db_path.clone())).map(Arc::new);
    if apns_service.is_some() {
        tracing::info!("APNs service initialized");
    }
    let remote_access = Arc::new(RwLock::new(
        remote_access::RemoteAccessConfig::load_or_create(&db_path)?,
    ));

    let state = AppState {
        server_port: config.port,
        db_path: Arc::new(db_path),
        working_dir: Arc::new(config.working_dir.clone()),
        ai_client,
        tool_registry,
        process_registry,
        model_registry,
        credential_store,
        mcp_manager,
        hook_manager,
        skills_manager: Arc::new(RwLock::new(SkillsManager::with_defaults(
            &config.working_dir,
        ))),
        cancellation,
        session_locks: Arc::new(RwLock::new(HashMap::new())),
        session_inputs: Arc::new(RwLock::new(HashMap::new())),
        session_presence: Arc::new(RwLock::new(HashMap::new())),
        delegated_state: Arc::new(RwLock::new(HashMap::new())),
        remote_access,
        active_agent_streams: Arc::new(AtomicUsize::new(0)),
        peak_rss_bytes: Arc::new(AtomicU64::new(0)),
        peak_virtual_bytes: Arc::new(AtomicU64::new(0)),
        push_service,
        apns_service,
        oauth_flows: Arc::new(Mutex::new(HashMap::new())),
        mako_runtime: mako_runtime::MakoRuntimeManager::new(),
    };

    spawn_model_catalog_refresh(
        state.model_registry.clone(),
        state.credential_store.clone(),
        state.db_path.clone(),
    );

    state
        .mako_runtime
        .restore_persisted_sessions(state.clone())
        .await?;

    let cors = http_policy.cors_layer();

    let protected_routes = Router::new()
        .route("/ws/terminal", get(ws::terminal::handler))
        .nest("/api", routes::api_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ));

    let app = Router::new()
        .route("/health", get(health))
        .merge(routes::oauth::callback_router())
        .merge(protected_routes)
        .fallback(serve_web_app)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        // Allow large request bodies for image uploads (up to 100MB)
        .layer(DefaultBodyLimit::max(http_policy.max_request_body_bytes))
        .with_state(state.clone());

    Ok((app, state))
}

fn reconcile_transient_agent_states(db_path: &std::path::Path) -> anyhow::Result<()> {
    let session_manager = krusty_core::SessionManager::new(Database::new(db_path)?);
    let repaired = session_manager.reset_transient_agent_states()?;
    let cleared_recovery = session_manager.clear_stale_transient_recovery_states()?;
    if repaired > 0 {
        tracing::info!(
            repaired_sessions = repaired,
            "Cleared transient agent execution states during server startup"
        );
    }
    if cleared_recovery > 0 {
        tracing::info!(
            cleared_sessions = cleared_recovery,
            "Cleared stale non-resumable recovery state during server startup"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProcessMemorySample {
    pub rss_bytes: Option<u64>,
    pub virtual_bytes: Option<u64>,
}

pub(crate) fn read_process_memory_status() -> ProcessMemorySample {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return ProcessMemorySample::default();
    };

    let mut rss_bytes = None;
    let mut virtual_bytes = None;

    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            rss_bytes = parse_proc_status_kib_line(line).map(|value| value * 1024);
        } else if line.starts_with("VmSize:") {
            virtual_bytes = parse_proc_status_kib_line(line).map(|value| value * 1024);
        }
    }

    ProcessMemorySample {
        rss_bytes,
        virtual_bytes,
    }
}

pub(crate) fn observe_process_memory(state: &AppState) -> ProcessMemorySample {
    let sample = read_process_memory_status();
    if let Some(rss_bytes) = sample.rss_bytes {
        state.peak_rss_bytes.fetch_max(rss_bytes, Ordering::Relaxed);
    }
    if let Some(virtual_bytes) = sample.virtual_bytes {
        state
            .peak_virtual_bytes
            .fetch_max(virtual_bytes, Ordering::Relaxed);
    }
    sample
}

fn parse_proc_status_kib_line(line: &str) -> Option<u64> {
    line.split_whitespace().nth(1)?.parse::<u64>().ok()
}

/// Start the Krusty server and block until shutdown.
pub async fn start_server(config: ServerConfig) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    start_server_with_listener(config, listener).await
}

/// Start the Krusty server from a pre-bound listener.
pub async fn start_server_with_listener(
    config: ServerConfig,
    listener: tokio::net::TcpListener,
) -> anyhow::Result<()> {
    let local_addr = listener.local_addr()?;
    let (app, _state) = build_router(&config).await?;

    tracing::info!(
        bind_address = %local_addr,
        local_url = %format!("http://localhost:{}", local_addr.port()),
        "Krusty server listening"
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// Serve embedded web assets with SPA fallback.
async fn serve_web_app(uri: Uri) -> impl IntoResponse {
    let http_policy = ServerHttpPolicy::default();
    let path = uri.path().trim_start_matches('/');

    // Try exact file match first
    if let Some(file) = WebAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .header(header::CACHE_CONTROL, cache_control(path, http_policy))
            .body(Body::from(file.data.to_vec()))
            .expect("static response builder");
    }

    // SPA fallback: serve index.html for all non-file routes
    match WebAssets::get("index.html") {
        Some(index) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from(index.data.to_vec()))
            .expect("static response builder"),
        None => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from(
                "Krusty API server running. Web frontend not embedded in this build.",
            ))
            .expect("static response builder"),
    }
}

/// Cache-control header value based on file type.
fn cache_control(path: &str, http_policy: ServerHttpPolicy) -> String {
    // Bundled immutable assets use content-hashed filenames, so cache policy lives
    // in the shared HTTP policy rather than as route-local string literals.
    http_policy.cache_control(path)
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let chat_available = state.ai_client.is_some();
    let tools_available = !state.tool_registry.get_ai_tools_all().await.is_empty();

    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        features: HashMap::from([
            ("chat".to_string(), chat_available),
            ("tools".to_string(), tools_available),
        ]),
    })
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    features: HashMap<String, bool>,
}

#[cfg(test)]
mod tests {
    use axum::async_trait;
    use serde_json::{json, Value};

    use krusty_core::tools::registry::{
        PermissionMode, Tool, ToolContext, ToolRegistry, ToolResult,
    };

    use super::*;

    struct TestWriteTool;

    #[async_trait]
    impl Tool for TestWriteTool {
        fn name(&self) -> &str {
            "write"
        }

        fn description(&self) -> &str {
            "test write tool"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }

        async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::success_data(json!({"executed": true}))
        }
    }

    #[tokio::test]
    async fn no_bootstrap_tool_registry_uses_deterministic_autonomous_policy() {
        let mut registry = ToolRegistry::new();
        register_autonomous_classifier_hook(&mut registry, None);
        let registry = Arc::new(registry);
        registry.register(Arc::new(TestWriteTool)).await;
        let ctx = ToolContext {
            permission_mode: PermissionMode::Autonomous,
            ..Default::default()
        };

        let result = registry
            .execute(
                "write",
                json!({"path": "src/lib.rs", "content": "unsafe autonomous write"}),
                &ctx,
            )
            .await
            .expect("test write tool should be registered");

        assert!(
            !result.is_error,
            "workspace mutation should not require a classifier model: {}",
            result.output
        );

        let blocked = registry
            .execute(
                "write",
                json!({"path": "/etc/shadow", "content": "unsafe autonomous write"}),
                &ctx,
            )
            .await
            .expect("test write tool should be registered");

        assert!(blocked.is_error, "sensitive path must fail closed");
        assert!(
            blocked.output.contains("credential or system path"),
            "unexpected result output: {}",
            blocked.output
        );
    }
}
