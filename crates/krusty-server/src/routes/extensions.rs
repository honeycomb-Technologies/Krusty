//! Executable agent-extension diagnostics and lifecycle endpoints.

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;

use krusty_core::extensions::{
    AgentExtensionCommand, AgentExtensionDiagnostic, AgentExtensionManager, AgentExtensionStatus,
    ProjectAgentExtensionTrustStatus,
};

use crate::error::AppError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_extensions))
        .route("/reload", post(reload_extensions))
        .route("/commands", get(list_commands))
        .route("/project-trust", get(project_trust_status))
        .route("/project-trust/grant", post(grant_project_trust))
        .route("/project-trust/revoke", post(revoke_project_trust))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionOverview {
    extensions: Vec<AgentExtensionStatus>,
    diagnostics: Vec<AgentExtensionDiagnostic>,
}

fn manager(state: &AppState) -> Result<std::sync::Arc<AgentExtensionManager>, AppError> {
    state
        .tool_registry
        .agent_extension_manager()
        .ok_or_else(|| AppError::Internal("Agent extension host is not initialized".to_string()))
}

async fn list_extensions(
    State(state): State<AppState>,
) -> Result<Json<ExtensionOverview>, AppError> {
    let manager = manager(&state)?;
    Ok(Json(ExtensionOverview {
        extensions: manager.statuses().await,
        diagnostics: manager.diagnostics().await,
    }))
}

async fn reload_extensions(
    State(state): State<AppState>,
) -> Result<Json<ExtensionOverview>, AppError> {
    let manager = manager(&state)?;
    manager
        .refresh_and_register(&state.tool_registry)
        .await
        .map_err(|error| {
            AppError::Internal(format!("Failed to reload agent extensions: {error}"))
        })?;
    list_extensions(State(state)).await
}

async fn list_commands(
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentExtensionCommand>>, AppError> {
    Ok(Json(manager(&state)?.commands().await))
}

async fn project_trust_status(
    State(state): State<AppState>,
) -> Result<Json<ProjectAgentExtensionTrustStatus>, AppError> {
    manager(&state)?
        .project_trust_status()
        .map(Json)
        .map_err(|error| AppError::Internal(format!("Failed to read project trust: {error}")))
}

async fn grant_project_trust(
    State(state): State<AppState>,
) -> Result<Json<ProjectAgentExtensionTrustStatus>, AppError> {
    set_project_trust(state, true).await
}

async fn revoke_project_trust(
    State(state): State<AppState>,
) -> Result<Json<ProjectAgentExtensionTrustStatus>, AppError> {
    set_project_trust(state, false).await
}

async fn set_project_trust(
    state: AppState,
    trusted: bool,
) -> Result<Json<ProjectAgentExtensionTrustStatus>, AppError> {
    let extension_manager = manager(&state)?;
    let status = extension_manager
        .set_project_trusted_and_refresh(trusted, &state.tool_registry)
        .await
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to update project trust and reload: {error}"
            ))
        })?;
    Ok(Json(status))
}
