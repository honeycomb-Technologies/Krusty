//! App initialization helpers
//!
//! Breaks up the 300+ line App::new() constructor into focused helper functions.

use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::agent::{UserHookManager, UserPostToolHook, UserPreToolHook};
use crate::ai::models::{create_model_registry, ModelMetadata, SharedModelRegistry};
use crate::ai::providers::{builtin_providers, ProviderId};
use crate::extensions::WasmHost;
use crate::paths;
use crate::plan::PlanManager;
use crate::process::ProcessRegistry;
use crate::storage::{CredentialStore, Database, Preferences, SessionManager};
use crate::tools::{register_all_tools, ToolRegistry};
use crate::tui::app::AppServices;
use crate::tui::themes::{Theme, THEME_REGISTRY};
use crate::tui::utils::{AsyncChannels, McpStatusUpdate};
use krusty_core::skills::SkillsManager;

/// Initialize core services (tools, extensions, etc.)
pub async fn init_services(
    working_dir: &Path,
) -> (
    AppServices,
    AsyncChannels,
    Arc<ProcessRegistry>,
    String,
    Arc<Theme>,
    String,
    ProviderId,
) {
    let process_registry = Arc::new(ProcessRegistry::new());

    // WASM extension host
    let extensions_dir = paths::extensions_dir();
    let http_client = reqwest::Client::new();
    let wasm_host = Some(WasmHost::new(http_client, extensions_dir.clone()));
    tracing::info!("WASM extension host initialized at {:?}", extensions_dir);

    // Database path
    let db_path = paths::config_dir().join("krusty.db");

    // User hook manager
    let user_hook_manager = init_user_hooks(&db_path).await;

    // Tool registry with hooks
    let tool_registry = init_tool_registry(&user_hook_manager).await;
    let cached_ai_tools = tool_registry.get_ai_tools().await;

    // Preferences and theme
    let (preferences, theme_name) = init_preferences(&db_path);
    let theme = THEME_REGISTRY.get_or_default(&theme_name);

    // Session manager
    let session_manager = init_session_manager(&db_path);

    // Plan manager
    let plan_manager = init_plan_manager(&db_path);

    // Credentials and active provider
    let credential_store = CredentialStore::load().unwrap_or_else(|e| {
        tracing::warn!("Failed to load credential store: {}", e);
        CredentialStore::default()
    });
    let active_provider = crate::storage::credentials::ActiveProviderStore::load();

    // Model registry
    let model_registry = init_model_registry(&preferences);

    // Current model from preferences
    let current_model = preferences
        .as_ref()
        .map(|p| p.get_current_model())
        .unwrap_or_else(|| "claude-opus-4-5-20251101".to_string());

    // Skills manager
    let global_skills_dir = paths::config_dir().join("skills");
    let project_skills_dir = Some(working_dir.join(".krusty").join("skills"));
    let skills_manager = Arc::new(RwLock::new(SkillsManager::new(
        global_skills_dir,
        project_skills_dir,
    )));

    // MCP manager and channels
    let mcp_manager = Arc::new(krusty_core::mcp::McpManager::new(working_dir.to_path_buf()));
    let (mcp_status_tx, mcp_status_rx) = tokio::sync::mpsc::unbounded_channel();
    let (oauth_status_tx, oauth_status_rx) = tokio::sync::mpsc::unbounded_channel();

    // Connect MCP servers in background
    spawn_mcp_connections(&mcp_manager, &tool_registry, &mcp_status_tx).await;

    // Set up channels
    let mut channels = AsyncChannels::new();
    channels.mcp_status = Some(mcp_status_rx);
    channels.oauth_status = Some(oauth_status_rx);

    let services = AppServices {
        plan_manager,
        session_manager,
        preferences,
        credential_store,
        model_registry,
        tool_registry,
        cached_ai_tools,
        user_hook_manager,
        wasm_host,
        skills_manager,
        mcp_manager,
        mcp_status_tx,
        oauth_status_tx,
    };

    (
        services,
        channels,
        process_registry,
        current_model,
        Arc::new(theme.clone()),
        theme_name,
        active_provider,
    )
}

/// Initialize user hooks from database
async fn init_user_hooks(db_path: &Path) -> Arc<RwLock<UserHookManager>> {
    let user_hook_manager = Arc::new(RwLock::new(UserHookManager::new()));
    if let Ok(db) = Database::new(db_path) {
        let hook_count = {
            let mut mgr = user_hook_manager.write().await;
            if let Err(e) = mgr.load(&db) {
                tracing::warn!("Failed to load user hooks: {}", e);
            }
            mgr.hooks().len()
        };
        if hook_count > 0 {
            tracing::info!("Loaded {} user hooks", hook_count);
        }
    }
    user_hook_manager
}

/// Initialize tool registry with safety hooks
async fn init_tool_registry(user_hook_manager: &Arc<RwLock<UserHookManager>>) -> Arc<ToolRegistry> {
    let mut tool_registry = ToolRegistry::new();
    tool_registry.add_pre_hook(Arc::new(crate::agent::SafetyHook::new()));
    tool_registry.add_pre_hook(Arc::new(crate::agent::PlanModeHook::new()));
    tool_registry.add_post_hook(Arc::new(crate::agent::LoggingHook::new()));
    tool_registry.add_pre_hook(Arc::new(UserPreToolHook::new(user_hook_manager.clone())));
    tool_registry.add_post_hook(Arc::new(UserPostToolHook::new(user_hook_manager.clone())));
    let tool_registry = Arc::new(tool_registry);
    register_all_tools(&tool_registry).await;
    tool_registry
}

/// Initialize preferences and get theme name
fn init_preferences(db_path: &Path) -> (Option<Preferences>, String) {
    match Database::new(db_path) {
        Ok(db) => {
            let prefs = Preferences::new(db);
            let theme = prefs.get_theme();
            (Some(prefs), theme)
        }
        Err(e) => {
            tracing::warn!("Failed to initialize preferences: {}", e);
            (None, "krusty".to_string())
        }
    }
}

/// Initialize session manager
fn init_session_manager(db_path: &Path) -> Option<SessionManager> {
    match Database::new(db_path) {
        Ok(db) => {
            tracing::info!("Session database initialized at {:?}", db_path);
            Some(SessionManager::new(db))
        }
        Err(e) => {
            tracing::warn!("Failed to initialize session database: {}", e);
            None
        }
    }
}

/// Initialize plan manager with migration
fn init_plan_manager(db_path: &Path) -> PlanManager {
    let plan_manager =
        PlanManager::new(db_path.to_path_buf()).expect("Failed to create plan manager");

    // Migrate legacy file-based plans
    match plan_manager.migrate_legacy_plans() {
        Ok((migrated, skipped)) if migrated > 0 => {
            tracing::info!(
                "Migrated {} legacy plans to database ({} skipped)",
                migrated,
                skipped
            );
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("Failed to migrate legacy plans: {}", e);
        }
    }

    plan_manager
}

/// Initialize model registry with static and cached models
fn init_model_registry(preferences: &Option<Preferences>) -> SharedModelRegistry {
    let model_registry = create_model_registry();

    // Load static models from builtin providers
    for provider in builtin_providers() {
        if provider.models.is_empty() {
            continue;
        }
        let models: Vec<ModelMetadata> = provider
            .models
            .iter()
            .map(|m| {
                let mut meta = ModelMetadata::new(&m.id, &m.display_name, provider.id)
                    .with_context(m.context_window, m.max_output);
                if let Some(format) = m.reasoning {
                    meta = meta.with_thinking(format);
                }
                meta
            })
            .collect();
        futures::executor::block_on(model_registry.set_models(provider.id, models));
    }
    tracing::info!("Model registry initialized with static models");

    // Load cached OpenRouter models
    if let Some(ref prefs) = preferences {
        if let Some(cached_models) = prefs.get_cached_openrouter_models() {
            futures::executor::block_on(
                model_registry.set_models(ProviderId::OpenRouter, cached_models.clone()),
            );
            tracing::info!(
                "Loaded {} cached OpenRouter models from preferences",
                cached_models.len()
            );
        }
    }

    // Load recent models
    if let Some(ref prefs) = preferences {
        let recent_ids = prefs.get_recent_models();
        if !recent_ids.is_empty() {
            futures::executor::block_on(model_registry.set_recent_ids(recent_ids.clone()));
            tracing::info!("Loaded {} recent models from preferences", recent_ids.len());
        }
    }

    model_registry
}

/// Spawn MCP server connections in background
async fn spawn_mcp_connections(
    mcp_manager: &Arc<krusty_core::mcp::McpManager>,
    tool_registry: &Arc<ToolRegistry>,
    status_tx: &tokio::sync::mpsc::UnboundedSender<McpStatusUpdate>,
) {
    if let Err(e) = mcp_manager.load_config().await {
        tracing::warn!("Failed to load MCP config: {}", e);
        return;
    }

    if !mcp_manager.has_servers().await {
        return;
    }

    let mcp = mcp_manager.clone();
    let registry = tool_registry.clone();
    let status_tx = status_tx.clone();

    tokio::spawn(async move {
        if let Err(e) = mcp.connect_all().await {
            tracing::warn!("MCP server connection errors: {}", e);
        }
        krusty_core::mcp::tool::register_mcp_tools(mcp.clone(), &registry).await;

        let tool_count = mcp.get_all_tools().await.len();
        if tool_count > 0 {
            let _ = status_tx.send(McpStatusUpdate {
                success: true,
                message: format!("MCP initialized ({} tools)", tool_count),
            });
        }
    });
}
