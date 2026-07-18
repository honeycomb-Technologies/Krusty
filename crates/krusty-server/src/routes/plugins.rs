//! Machine-wide plugin catalog, lifecycle, permission, and runtime endpoints.
//!
//! This router is mounted beneath the server's protected `/api` router. Plugin
//! state is intentionally machine-wide rather than scoped to a chat/session:
//! callers must pass the same local-origin or remote bearer-token checks as the
//! rest of the protected API before they can mutate executable packages.

use std::path::PathBuf;
use std::sync::OnceLock;

use axum::{
    extract::{Path, State},
    routing::{delete, get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use krusty_core::agent::PackageHookConfig;
use krusty_core::mcp::{McpConnectionAuthority, McpPackageConfig};
use krusty_core::plugins::{
    InstalledPlugin, PluginCatalogEntry, PluginInstallOptions, PluginManager, PluginPermission,
    PluginPermissionSet, PluginPermissionStatus, PluginReconcileReport, PluginSourceTrust,
    PluginUpdateReport,
};

use crate::error::AppError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_plugins))
        .route("/catalog", get(list_catalog))
        .route("/install", post(install_plugin))
        .route("/update-all", post(update_all_plugins))
        .route("/reconcile", post(reconcile_plugins))
        .route("/:id", delete(uninstall_plugin))
        .route("/:id/update", post(update_plugin))
        .route("/:id/enabled", patch(set_plugin_enabled))
        .route("/:id/pinned", patch(set_plugin_pinned))
        .route(
            "/:id/permissions",
            get(permission_status).delete(revoke_permissions),
        )
        .route("/:id/permissions/grant", post(grant_permissions))
}

/// All HTTP handlers share one in-process manager. The manager also uses the
/// core's cross-process mutation lease, so a TUI and server cannot corrupt the
/// same lockfile when they mutate plugin state concurrently.
pub(crate) fn plugin_manager() -> PluginManager {
    static MANAGER: OnceLock<PluginManager> = OnceLock::new();
    MANAGER
        .get_or_init(|| {
            PluginManager::new(reqwest::Client::new(), krusty_core::paths::plugins_dir())
        })
        .clone()
}

fn runtime_refresh_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginListResponse {
    plugins: Vec<InstalledPluginResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginCatalogResponse {
    plugins: Vec<PluginCatalogEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstalledPluginResponse {
    id: String,
    name: String,
    version: String,
    publisher: String,
    description: Option<String>,
    runtime: String,
    install_path: String,
    manifest_path: String,
    entry_component_path: Option<String>,
    skill_paths: Vec<String>,
    agent_extension_paths: Vec<String>,
    mcp_servers_path: Option<String>,
    hook_paths: Vec<String>,
    assets_path: Option<String>,
    enabled: bool,
    pinned: bool,
    source: Option<String>,
    source_trust: PluginSourceTrust,
    cryptographically_verified: bool,
    package_scripts_allowed: bool,
    requested_permissions: PluginPermissionSetResponse,
}

impl From<InstalledPlugin> for InstalledPluginResponse {
    fn from(plugin: InstalledPlugin) -> Self {
        Self {
            id: plugin.id,
            name: plugin.name,
            version: plugin.version,
            publisher: plugin.publisher,
            description: plugin.description,
            runtime: format!("{:?}", plugin.runtime).to_ascii_lowercase(),
            install_path: display_path(plugin.install_path),
            manifest_path: display_path(plugin.manifest_path),
            entry_component_path: plugin.entry_component_path.map(display_path),
            skill_paths: plugin.skill_paths.into_iter().map(display_path).collect(),
            agent_extension_paths: plugin
                .agent_extension_paths
                .into_iter()
                .map(display_path)
                .collect(),
            mcp_servers_path: plugin.mcp_servers_path.map(display_path),
            hook_paths: plugin.hook_paths.into_iter().map(display_path).collect(),
            assets_path: plugin.assets_path.map(display_path),
            enabled: plugin.enabled,
            pinned: plugin.pinned,
            source: plugin.source,
            source_trust: plugin.source_trust,
            cryptographically_verified: plugin.source_trust.is_cryptographically_verified(),
            package_scripts_allowed: plugin.package_scripts_allowed,
            requested_permissions: plugin.requested_permissions.into(),
        }
    }
}

fn display_path(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PluginPermissionSetResponse {
    #[serde(default)]
    fs_read: bool,
    #[serde(default)]
    fs_write: bool,
    #[serde(default)]
    network: bool,
    #[serde(default)]
    process: bool,
}

impl From<PluginPermissionSet> for PluginPermissionSetResponse {
    fn from(permissions: PluginPermissionSet) -> Self {
        Self {
            fs_read: permissions.fs_read,
            fs_write: permissions.fs_write,
            network: permissions.network,
            process: permissions.process,
        }
    }
}

impl From<PluginPermissionSetResponse> for PluginPermissionSet {
    fn from(permissions: PluginPermissionSetResponse) -> Self {
        Self {
            fs_read: permissions.fs_read,
            fs_write: permissions.fs_write,
            network: permissions.network,
            process: permissions.process,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginPermissionStatusResponse {
    plugin_id: String,
    plugin_version: String,
    requested: PluginPermissionSetResponse,
    granted: PluginPermissionSetResponse,
    grant_is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    process_trust_warning: Option<&'static str>,
}

impl From<PluginPermissionStatus> for PluginPermissionStatusResponse {
    fn from(status: PluginPermissionStatus) -> Self {
        let process_trust_warning = status.requested.process.then_some(
            "process authorizes trusted native/JS/shell code with full user OS authority",
        );
        Self {
            plugin_id: status.plugin_id,
            plugin_version: status.plugin_version,
            requested: status.requested.into(),
            granted: status.granted.into(),
            grant_is_current: status.grant_is_current,
            process_trust_warning,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginRuntimeRefreshResponse {
    refreshed: bool,
    skill_roots: usize,
    agent_extension_roots: usize,
    package_hook_configs: usize,
    package_hooks: usize,
    mcp_config_paths: usize,
    disabled_executable_plugins: Vec<String>,
    disabled_mcp_plugins: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginMutationResponse {
    plugins: Vec<InstalledPluginResponse>,
    runtime: PluginRuntimeRefreshResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginUninstallResponse {
    plugin_id: String,
    uninstalled: bool,
    runtime: PluginRuntimeRefreshResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginPermissionMutationResponse {
    status: PluginPermissionStatusResponse,
    runtime: PluginRuntimeRefreshResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginPermissionRevokeResponse {
    plugin_id: String,
    revoked: bool,
    runtime: PluginRuntimeRefreshResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginUpdateRecordResponse {
    id: String,
    previous_version: String,
    current_version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginUpdateReportResponse {
    updated: Vec<PluginUpdateRecordResponse>,
    unchanged: Vec<String>,
    removed: Vec<String>,
    skipped_pinned: Vec<String>,
}

impl From<PluginUpdateReport> for PluginUpdateReportResponse {
    fn from(report: PluginUpdateReport) -> Self {
        Self {
            updated: report
                .updated
                .into_iter()
                .map(|record| PluginUpdateRecordResponse {
                    id: record.id,
                    previous_version: record.previous_version,
                    current_version: record.current_version,
                })
                .collect(),
            unchanged: report.unchanged,
            removed: report.removed,
            skipped_pinned: report.skipped_pinned,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginUpdateResponse {
    report: PluginUpdateReportResponse,
    runtime: PluginRuntimeRefreshResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InvalidPluginResponse {
    plugin_id: String,
    error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginReconcileReportResponse {
    valid_plugins: Vec<String>,
    invalid_plugins: Vec<InvalidPluginResponse>,
    removed_orphan_roots: Vec<String>,
    updates: PluginUpdateReportResponse,
}

impl From<PluginReconcileReport> for PluginReconcileReportResponse {
    fn from(report: PluginReconcileReport) -> Self {
        Self {
            valid_plugins: report.valid_plugins,
            invalid_plugins: report
                .invalid_plugins
                .into_iter()
                .map(|(plugin_id, error)| InvalidPluginResponse { plugin_id, error })
                .collect(),
            removed_orphan_roots: report
                .removed_orphan_roots
                .into_iter()
                .map(display_path)
                .collect(),
            updates: report.updates.into(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginReconcileResponse {
    report: PluginReconcileReportResponse,
    runtime: PluginRuntimeRefreshResponse,
}

async fn list_plugins() -> Result<Json<PluginListResponse>, AppError> {
    let plugins = plugin_manager()
        .list_installed_plugins()
        .await
        .map_err(plugin_internal_error)?;
    Ok(Json(PluginListResponse {
        plugins: plugins
            .into_iter()
            .map(InstalledPluginResponse::from)
            .collect(),
    }))
}

async fn list_catalog() -> Result<Json<PluginCatalogResponse>, AppError> {
    let plugins = plugin_manager()
        .list_catalog_plugins()
        .await
        .map_err(|error| AppError::BadGateway(format!("Failed to load plugin catalog: {error}")))?;
    Ok(Json(PluginCatalogResponse { plugins }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallPluginRequest {
    plugin_ref: String,
    #[serde(default)]
    allow_package_scripts: bool,
    #[serde(default)]
    pinned: Option<bool>,
}

async fn install_plugin(
    State(state): State<AppState>,
    Json(request): Json<InstallPluginRequest>,
) -> Result<Json<PluginMutationResponse>, AppError> {
    let plugin_ref = request.plugin_ref.trim();
    if plugin_ref.is_empty() {
        return Err(AppError::BadRequest(
            "pluginRef must not be empty".to_string(),
        ));
    }
    let installed = plugin_manager()
        .install_from_ref_with_options(
            plugin_ref,
            PluginInstallOptions {
                allow_package_scripts: request.allow_package_scripts,
                pinned: request.pinned,
            },
        )
        .await
        .map_err(|error| plugin_mutation_error("Failed to install plugin", error))?;
    mutation_response(&state, installed).await
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdatePluginsRequest {
    #[serde(default)]
    include_pinned: bool,
}

async fn update_plugin(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdatePluginsRequest>,
) -> Result<Json<PluginUpdateResponse>, AppError> {
    let report = plugin_manager()
        .update_plugin(&id, request.include_pinned)
        .await
        .map_err(|error| plugin_mutation_error("Failed to update plugin", error))?;
    let runtime = refresh_plugin_contributions(&state, &plugin_manager()).await;
    Ok(Json(PluginUpdateResponse {
        report: report.into(),
        runtime,
    }))
}

async fn update_all_plugins(
    State(state): State<AppState>,
    Json(request): Json<UpdatePluginsRequest>,
) -> Result<Json<PluginUpdateResponse>, AppError> {
    let report = plugin_manager()
        .update_all_plugins(request.include_pinned)
        .await
        .map_err(|error| plugin_mutation_error("Failed to update plugins", error))?;
    let runtime = refresh_plugin_contributions(&state, &plugin_manager()).await;
    Ok(Json(PluginUpdateResponse {
        report: report.into(),
        runtime,
    }))
}

async fn uninstall_plugin(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PluginUninstallResponse>, AppError> {
    plugin_manager()
        .uninstall_plugin(&id)
        .await
        .map_err(|error| plugin_mutation_error("Failed to uninstall plugin", error))?;
    let runtime = refresh_plugin_contributions(&state, &plugin_manager()).await;
    Ok(Json(PluginUninstallResponse {
        plugin_id: id,
        uninstalled: true,
        runtime,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetEnabledRequest {
    enabled: bool,
}

async fn set_plugin_enabled(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<SetEnabledRequest>,
) -> Result<Json<PluginMutationResponse>, AppError> {
    let manager = plugin_manager();
    manager
        .set_plugin_enabled(&id, request.enabled)
        .await
        .map_err(|error| plugin_mutation_error("Failed to change plugin state", error))?;
    let plugin = installed_plugin(&manager, &id).await?;
    mutation_response(&state, vec![plugin]).await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPinnedRequest {
    pinned: bool,
}

async fn set_plugin_pinned(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<SetPinnedRequest>,
) -> Result<Json<PluginMutationResponse>, AppError> {
    let manager = plugin_manager();
    manager
        .set_plugin_pinned(&id, request.pinned)
        .await
        .map_err(|error| plugin_mutation_error("Failed to change plugin pin", error))?;
    let plugin = installed_plugin(&manager, &id).await?;
    mutation_response(&state, vec![plugin]).await
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReconcilePluginsRequest {
    #[serde(default)]
    update_unpinned: bool,
}

async fn reconcile_plugins(
    State(state): State<AppState>,
    Json(request): Json<ReconcilePluginsRequest>,
) -> Result<Json<PluginReconcileResponse>, AppError> {
    let report = plugin_manager()
        .reconcile_plugins(request.update_unpinned)
        .await
        .map_err(|error| plugin_mutation_error("Failed to reconcile plugins", error))?;
    let runtime = refresh_plugin_contributions(&state, &plugin_manager()).await;
    Ok(Json(PluginReconcileResponse {
        report: report.into(),
        runtime,
    }))
}

async fn permission_status(
    Path(id): Path<String>,
) -> Result<Json<PluginPermissionStatusResponse>, AppError> {
    let status = plugin_manager()
        .permission_status(&id)
        .await
        .map_err(|error| plugin_mutation_error("Failed to read plugin permissions", error))?;
    Ok(Json(status.into()))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrantPermissionsRequest {
    #[serde(default)]
    grant_all: bool,
    permissions: Option<PluginPermissionSetResponse>,
}

async fn grant_permissions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<GrantPermissionsRequest>,
) -> Result<Json<PluginPermissionMutationResponse>, AppError> {
    let manager = plugin_manager();
    let status = match (request.grant_all, request.permissions) {
        (true, None) => manager.grant_all_plugin_permissions(&id).await,
        (false, Some(permissions)) => {
            manager
                .grant_plugin_permissions(&id, permissions.into())
                .await
        }
        _ => {
            return Err(AppError::BadRequest(
                "Set grantAll=true or provide permissions, but not both".to_string(),
            ));
        }
    }
    .map_err(|error| plugin_mutation_error("Failed to grant plugin permissions", error))?;
    let runtime = refresh_plugin_contributions(&state, &manager).await;
    Ok(Json(PluginPermissionMutationResponse {
        status: status.into(),
        runtime,
    }))
}

async fn revoke_permissions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PluginPermissionRevokeResponse>, AppError> {
    let manager = plugin_manager();
    manager
        .revoke_plugin_permissions(&id)
        .await
        .map_err(|error| plugin_mutation_error("Failed to revoke plugin permissions", error))?;
    let runtime = refresh_plugin_contributions(&state, &manager).await;
    Ok(Json(PluginPermissionRevokeResponse {
        plugin_id: id,
        revoked: true,
        runtime,
    }))
}

async fn mutation_response(
    state: &AppState,
    plugins: Vec<InstalledPlugin>,
) -> Result<Json<PluginMutationResponse>, AppError> {
    let runtime = refresh_plugin_contributions(state, &plugin_manager()).await;
    Ok(Json(PluginMutationResponse {
        plugins: plugins
            .into_iter()
            .map(InstalledPluginResponse::from)
            .collect(),
        runtime,
    }))
}

async fn installed_plugin(
    manager: &PluginManager,
    plugin_id: &str,
) -> Result<InstalledPlugin, AppError> {
    manager
        .list_installed_plugins()
        .await
        .map_err(plugin_internal_error)?
        .into_iter()
        .find(|plugin| plugin.id == plugin_id)
        .ok_or_else(|| AppError::NotFound(format!("Plugin '{plugin_id}' is not installed")))
}

/// Replace every package-derived runtime root from one enabled-plugin snapshot.
/// A lifecycle mutation remains successful even if a contributed runtime is
/// temporarily unavailable; those failures are returned as explicit warnings
/// so clients never retry a destructive mutation merely because reload failed.
async fn refresh_plugin_contributions(
    state: &AppState,
    manager: &PluginManager,
) -> PluginRuntimeRefreshResponse {
    let _refresh_guard = runtime_refresh_lock().lock().await;
    let mut response = PluginRuntimeRefreshResponse {
        refreshed: true,
        skill_roots: 0,
        agent_extension_roots: 0,
        package_hook_configs: 0,
        package_hooks: 0,
        mcp_config_paths: 0,
        disabled_executable_plugins: Vec::new(),
        disabled_mcp_plugins: Vec::new(),
        warnings: Vec::new(),
    };

    let installed = match manager.list_installed_plugins().await {
        Ok(plugins) => plugins,
        Err(error) => {
            state.tool_registry.unregister_by_prefix("mcp__").await;
            response.refreshed = false;
            response
                .warnings
                .push(format!("Failed to resolve installed plugins: {error}"));
            return response;
        }
    };

    let mut skill_roots = Vec::new();
    let mut extension_roots = Vec::new();
    let mut package_hook_configs = Vec::new();
    let mut mcp_paths = Vec::new();

    for plugin in installed.iter().filter(|plugin| plugin.enabled) {
        skill_roots.extend(
            plugin
                .skill_paths
                .iter()
                .cloned()
                .map(|path| (plugin.id.clone(), path)),
        );

        if !plugin.agent_extension_paths.is_empty() || !plugin.hook_paths.is_empty() {
            match manager
                .ensure_installed_plugin_permission(plugin, PluginPermission::Process)
                .await
            {
                Ok(()) => {
                    extension_roots.extend(plugin.agent_extension_paths.iter().cloned());
                    for path in &plugin.hook_paths {
                        package_hook_configs.push(PackageHookConfig::new(
                            plugin.id.clone(),
                            path.clone(),
                            plugin.install_path.clone(),
                        ));
                    }
                }
                Err(error) => {
                    response.disabled_executable_plugins.push(plugin.id.clone());
                    response.warnings.push(format!(
                        "Executable contributions from '{}' remain disabled: {error}",
                        plugin.id
                    ));
                }
            }
        }

        if let Some(path) = &plugin.mcp_servers_path {
            match manager.permission_status_for_installed(plugin).await {
                Ok(status) if status.grant_is_current => {
                    let authority =
                        McpConnectionAuthority::new(status.granted.process, status.granted.network);
                    if authority.is_empty() {
                        response.disabled_mcp_plugins.push(plugin.id.clone());
                        response.warnings.push(format!(
                            "MCP contribution from '{}' remains disabled until process or network authority is granted",
                            plugin.id
                        ));
                    } else {
                        mcp_paths.push(McpPackageConfig::new(path.clone(), authority));
                    }
                }
                Ok(_) => {
                    response.disabled_mcp_plugins.push(plugin.id.clone());
                    response.warnings.push(format!(
                        "MCP contribution from '{}' remains disabled until process or network authority is granted",
                        plugin.id
                    ));
                }
                Err(error) => {
                    response.disabled_mcp_plugins.push(plugin.id.clone());
                    response.warnings.push(format!(
                        "Failed to resolve MCP permissions for '{}': {error}",
                        plugin.id
                    ));
                }
            }
        }
    }

    response.skill_roots = skill_roots.len();
    response.agent_extension_roots = extension_roots.len();
    response.package_hook_configs = package_hook_configs.len();
    response.mcp_config_paths = mcp_paths.len();
    response.disabled_executable_plugins.sort();
    response.disabled_mcp_plugins.sort();

    state
        .skills_manager
        .write()
        .await
        .set_package_roots(skill_roots);

    match state
        .hook_manager
        .write()
        .await
        .replace_package_hooks(package_hook_configs)
    {
        Ok(report) => {
            response.package_hook_configs = report.config_count;
            response.package_hooks = report.hook_count;
        }
        Err(error) => {
            response.refreshed = false;
            response
                .warnings
                .push(format!("Failed to reload package hooks: {error}"));
        }
    }

    match state.tool_registry.agent_extension_manager() {
        Some(extension_manager) => {
            if let Err(error) = extension_manager
                .set_package_roots_and_refresh(extension_roots, &state.tool_registry)
                .await
            {
                response.refreshed = false;
                response
                    .warnings
                    .push(format!("Failed to reload agent extensions: {error}"));
            }
        }
        None if response.agent_extension_roots > 0 => {
            response.refreshed = false;
            response
                .warnings
                .push("Agent extension host is not initialized".to_string());
        }
        None => {}
    }

    state.mcp_manager.set_package_configs(mcp_paths).await;
    match state.mcp_manager.load_config().await {
        Ok(()) => {
            if let Err(error) = state.mcp_manager.connect_all().await {
                response.refreshed = false;
                response
                    .warnings
                    .push(format!("Required MCP server failure: {error}"));
            }
            state.tool_registry.unregister_by_prefix("mcp__").await;
            krusty_core::mcp::tool::register_mcp_tools(
                state.mcp_manager.clone(),
                &state.tool_registry,
            )
            .await;
        }
        Err(error) => {
            response.refreshed = false;
            response
                .warnings
                .push(format!("Failed to reload MCP configuration: {error}"));
        }
    }

    response
}

fn plugin_internal_error(error: anyhow::Error) -> AppError {
    tracing::error!(error = ?error, "Plugin manager operation failed");
    AppError::Internal(error.to_string())
}

fn plugin_mutation_error(context: &str, error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if message.contains("is not installed") || message.contains("is not configured") {
        AppError::NotFound(format!("{context}: {message}"))
    } else {
        AppError::BadRequest(format!("{context}: {message}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        plugin_mutation_error, GrantPermissionsRequest, PluginPermissionSetResponse,
        PluginRuntimeRefreshResponse,
    };
    use crate::error::AppError;
    use krusty_core::plugins::PluginPermissionSet;

    #[test]
    fn permission_shape_round_trips_without_widening() {
        let request = PluginPermissionSetResponse {
            fs_read: true,
            fs_write: false,
            network: true,
            process: false,
        };
        let core: PluginPermissionSet = request.into();
        assert!(core.fs_read);
        assert!(!core.fs_write);
        assert!(core.network);
        assert!(!core.process);
    }

    #[test]
    fn runtime_refresh_response_uses_camel_case() {
        let response = PluginRuntimeRefreshResponse {
            refreshed: true,
            skill_roots: 2,
            agent_extension_roots: 1,
            package_hook_configs: 1,
            package_hooks: 2,
            mcp_config_paths: 1,
            disabled_executable_plugins: Vec::new(),
            disabled_mcp_plugins: Vec::new(),
            warnings: Vec::new(),
        };
        let value = serde_json::to_value(response).expect("response should serialize");
        assert_eq!(value["skillRoots"], 2);
        assert_eq!(value["agentExtensionRoots"], 1);
        assert_eq!(value["packageHookConfigs"], 1);
        assert_eq!(value["packageHooks"], 2);
        assert_eq!(value["mcpConfigPaths"], 1);
    }

    #[test]
    fn permission_requests_reject_unknown_capabilities() {
        let request = serde_json::json!({
            "permissions": {
                "process": true,
                "unrestrictedShell": true
            }
        });
        assert!(serde_json::from_value::<GrantPermissionsRequest>(request).is_err());
    }

    #[test]
    fn missing_plugin_mutations_map_to_not_found() {
        let error = plugin_mutation_error(
            "Failed to update plugin",
            anyhow::anyhow!("plugin 'missing' is not installed"),
        );
        assert!(matches!(error, AppError::NotFound(_)));
    }
}
