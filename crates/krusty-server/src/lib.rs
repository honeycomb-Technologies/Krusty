//! Krusty Server
//!
//! Self-hosted API server for chat, tools, sessions, and local workspace access.
//! This is a library crate — the server is started via `start_server()`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::{header, Method, Response, StatusCode, Uri},
    middleware,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use rust_embed::Embed;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use krusty_core::agent::{
    AgentCancellation, LoggingHook, PlanModeHook, SafetyHook, UserHookManager, UserPostToolHook,
    UserPreToolHook,
};
use krusty_core::ai::client::{AiClient, AiClientConfig};
use krusty_core::ai::models::{create_model_registry, ModelMetadata, SharedModelRegistry};
use krusty_core::ai::providers::{builtin_providers, get_provider, ProviderId};
use krusty_core::constants;
use krusty_core::mcp::McpManager;
use krusty_core::paths;
use krusty_core::process::ProcessRegistry;
use krusty_core::skills::SkillsManager;
use krusty_core::storage::credentials::CredentialStore;
use krusty_core::storage::Database;
use krusty_core::tools::implementations::{
    register_agent_tool, register_all_tools, register_mako_tools,
};
use krusty_core::tools::registry::ToolRegistry;

type SessionGuard = Arc<Mutex<()>>;
type SessionLockMap = HashMap<String, (SessionGuard, Instant)>;
type SessionInputMap =
    HashMap<String, tokio::sync::mpsc::UnboundedSender<krusty_core::agent::LoopInput>>;
type SessionPresenceMap = presence::SessionPresenceMap;
type DelegatedStateMap = HashMap<String, Vec<types::DelegatedToolStateResponse>>;
pub mod apns;
pub mod auth;
pub mod error;
pub mod presence;
pub mod push;
pub mod remote_access;
pub mod routes;
pub mod types;
pub mod utils;
pub mod ws;

/// Embedded PWA frontend assets.
///
/// At compile time, rust-embed includes all files from the PWA build directory.
/// When the build directory is absent, this will be empty and the server
/// gracefully falls back to API-only mode.
#[derive(Embed)]
#[folder = "../../apps/pwa/app/build"]
#[prefix = ""]
#[allow_missing = true]
struct PwaAssets;

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
    pub oauth_flows: Arc<Mutex<HashMap<String, routes::oauth::OAuthFlowState>>>,
}

impl AppState {
    /// Resolve a fresh AI client using the current credential store and requested model.
    pub async fn resolve_ai_client(&self, requested_model: Option<&str>) -> Option<Arc<AiClient>> {
        let credentials = self.credential_store.read().await.clone();
        create_ai_client_for_model(&credentials, &self.model_registry, requested_model)
            .await
            .map(Arc::new)
    }
}

/// Build an AI client from configured credentials and env overrides.
pub fn create_ai_client(credentials: &CredentialStore) -> Option<AiClient> {
    let provider = std::env::var("KRUSTY_PROVIDER")
        .ok()
        .as_deref()
        .and_then(utils::providers::parse_provider)
        .unwrap_or(ProviderId::MiniMax);

    let provider_cfg = get_provider(provider)?;
    let model =
        std::env::var("KRUSTY_MODEL").unwrap_or_else(|_| provider_cfg.default_model().to_string());

    create_ai_client_for_provider(credentials, provider, model)
}

pub async fn create_ai_client_for_model(
    credentials: &CredentialStore,
    model_registry: &SharedModelRegistry,
    requested_model: Option<&str>,
) -> Option<AiClient> {
    let requested_model = requested_model
        .map(str::trim)
        .filter(|model| !model.is_empty());

    let provider = std::env::var("KRUSTY_PROVIDER")
        .ok()
        .as_deref()
        .and_then(utils::providers::parse_provider)
        .unwrap_or(ProviderId::MiniMax);

    let provider = if let Some(model) = requested_model {
        model_registry
            .get_model(model)
            .await
            .map(|metadata| metadata.provider)
            .unwrap_or(provider)
    } else {
        provider
    };

    let provider_cfg = get_provider(provider)?;
    let model = requested_model.map(ToOwned::to_owned).unwrap_or_else(|| {
        std::env::var("KRUSTY_MODEL").unwrap_or_else(|_| provider_cfg.default_model().to_string())
    });

    create_ai_client_for_provider(credentials, provider, model)
}

fn create_ai_client_for_provider(
    credentials: &CredentialStore,
    provider: ProviderId,
    model: String,
) -> Option<AiClient> {
    let auth = credentials.get_auth(&provider).or_else(|| {
        let env_key = match provider {
            ProviderId::MiniMax => "MINIMAX_API_KEY",
            ProviderId::OpenRouter => "OPENROUTER_API_KEY",
            ProviderId::ZAi => "Z_AI_API_KEY",
            ProviderId::Anthropic => "ANTHROPIC_API_KEY",
            ProviderId::OpenAI => "OPENAI_API_KEY",
        };
        std::env::var(env_key).ok()
    });

    let provider_cfg = get_provider(provider)?;

    let (config, api_key) = match provider {
        ProviderId::OpenAI => {
            let config = AiClientConfig::for_openai_with_auth_detection(&model, credentials);
            let resolved = krusty_core::auth::resolve_openai_auth(credentials, &model);

            let auth = resolved
                .credential
                .or_else(|| std::env::var("OPENAI_API_KEY").ok());
            let api_key = match auth {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        "No OpenAI credentials found for resolved auth mode ({:?}); chat API unavailable",
                        resolved.auth_type
                    );
                    return None;
                }
            };
            (config, api_key)
        }
        ProviderId::Anthropic => {
            let config = AiClientConfig::for_anthropic_with_auth_detection(&model, credentials);
            let api_key = match auth {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        "No credentials found for provider {}; chat API will be unavailable until credentials are configured",
                        provider
                    );
                    return None;
                }
            };
            (config, api_key)
        }
        _ => {
            let api_key = match auth {
                Some(key) => key,
                None => {
                    tracing::warn!(
                        "No credentials found for provider {}; chat API will be unavailable until credentials are configured",
                        provider
                    );
                    return None;
                }
            };
            (
                AiClientConfig {
                    model,
                    max_tokens: constants::ai::MAX_OUTPUT_TOKENS,
                    base_url: Some(provider_cfg.base_url.clone()),
                    auth_header: provider_cfg.auth_header,
                    provider_id: provider,
                    api_format: Default::default(),
                    custom_headers: provider_cfg.custom_headers.clone(),
                },
                api_key,
            )
        }
    };

    Some(AiClient::new(config, api_key))
}

/// Initialize models in the shared registry.
async fn initialize_models(registry: &SharedModelRegistry, credentials: &CredentialStore) {
    for provider in builtin_providers() {
        let models: Vec<ModelMetadata> = provider
            .models
            .iter()
            .map(|m| {
                let mut model = ModelMetadata::new(&m.id, &m.display_name, provider.id)
                    .with_context(m.context_window, m.max_output);

                if let Some(reasoning) = m.reasoning {
                    model = model.with_thinking(reasoning);
                }

                model.supports_tools = provider.supports_tools;
                model
            })
            .collect();

        registry.set_models(provider.id, models).await;
    }

    if let Some(api_key) = credentials.get(&ProviderId::OpenRouter) {
        match krusty_core::ai::openrouter::fetch_models(api_key).await {
            Ok(models) => {
                tracing::info!("Fetched {} OpenRouter models", models.len());
                registry.set_models(ProviderId::OpenRouter, models).await;
            }
            Err(e) => tracing::warn!("Failed to fetch OpenRouter models: {}", e),
        }
    }

    let openai_credential =
        krusty_core::auth::resolve_openai_auth(credentials, "gpt-5.3-codex").credential;
    if let Some(api_key) = openai_credential {
        match krusty_core::ai::openai::fetch_models(&api_key).await {
            Ok(models) => {
                tracing::info!("Fetched {} OpenAI models", models.len());
                registry.set_models(ProviderId::OpenAI, models).await;
            }
            Err(e) => tracing::warn!("Failed to fetch OpenAI models: {}", e),
        }
    }
}

/// Build the Axum router with all routes and embedded PWA assets.
pub async fn build_router(config: &ServerConfig) -> anyhow::Result<(Router, AppState)> {
    let db_path = paths::config_dir().join("krusty.db");
    let _db = Database::new(&db_path)?;
    reconcile_transient_agent_states(&db_path)?;

    let credential_store_inner = CredentialStore::load().unwrap_or_default();
    let credential_store = Arc::new(RwLock::new(credential_store_inner.clone()));
    let ai_client = create_ai_client(&credential_store_inner).map(Arc::new);

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
    // Auto-classifier for Mako autonomous sessions (no-op when permission_mode != Autonomous)
    if let Some(ref client) = ai_client {
        use krusty_core::agent::auto_classifier::AutoClassifierHook;
        tool_registry_inner.add_pre_hook(Arc::new(AutoClassifierHook::new(client.clone())));
    }
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

    let model_registry = create_model_registry();
    initialize_models(&model_registry, &credential_store_inner).await;

    // MCP server connections + tool registration
    let mcp_manager = Arc::new(McpManager::new(config.working_dir.clone()));
    if let Err(e) = mcp_manager.load_config().await {
        tracing::warn!("Failed to load MCP config: {}", e);
    } else if let Err(e) = mcp_manager.connect_all().await {
        tracing::warn!("Failed to connect MCP servers: {}", e);
    }
    // Register MCP tools so they're visible to the AI
    krusty_core::mcp::tool::register_mcp_tools(mcp_manager.clone(), &tool_registry).await;
    let mcp_tool_count = tool_registry.get_ai_tools().await.len();
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
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::PUT,
            Method::DELETE,
        ])
        .allow_headers(Any);

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
        .fallback(serve_pwa)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        // Allow large request bodies for image uploads (up to 100MB)
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
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
    let (app, _state) = build_router(&config).await?;

    tracing::info!(
        bind_address = %addr,
        local_url = %format!("http://localhost:{}", config.port),
        "Krusty server listening"
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// Serve embedded PWA assets with SPA fallback.
async fn serve_pwa(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    // Try exact file match first
    if let Some(file) = PwaAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .header(header::CACHE_CONTROL, cache_control(path))
            .body(Body::from(file.data.to_vec()))
            .expect("static response builder");
    }

    // SPA fallback: serve index.html for all non-file routes
    match PwaAssets::get("index.html") {
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
                "Krusty API server running. PWA frontend not embedded in this build.",
            ))
            .expect("static response builder"),
    }
}

/// Cache-control header value based on file type.
fn cache_control(path: &str) -> &'static str {
    if path.contains("/_app/immutable/") {
        // SvelteKit immutable assets — hash in filename, cache forever
        "public, max-age=31536000, immutable"
    } else if path.ends_with(".html") {
        "no-cache"
    } else {
        "public, max-age=3600"
    }
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        features: HashMap::from([("chat".to_string(), true), ("tools".to_string(), true)]),
    })
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    features: HashMap<String, bool>,
}
