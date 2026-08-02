//! Skill listing endpoints for frontend settings/diagnostics.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use mitsuro_core::skills::{SkillDiagnostic, SkillInfo, SkillPermission};

use crate::error::AppError;
use crate::AppState;

/// Build the skills router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_skills))
        .route("/refresh", post(refresh_skills))
        .route("/diagnostics", get(list_diagnostics))
        .route("/:name/policy", post(update_skill_policy))
}

#[derive(Debug, Default, Deserialize)]
struct SkillsQuery {
    scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillInfoResponse {
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub source: String,
    pub origin: String,
    pub path: String,
    pub enabled: bool,
    pub permission: SkillPermission,
    pub model_invocable: bool,
}

async fn list_skills(
    State(state): State<AppState>,
    Query(query): Query<SkillsQuery>,
) -> Result<Json<Vec<SkillInfoResponse>>, AppError> {
    let mut skills_manager = state.skills_manager.write().await;
    let skills = match query.scope.as_deref() {
        None | Some("all") => skills_manager.list_skills(),
        Some("global") => skills_manager.list_global_skills(),
        Some("model") => skills_manager.list_model_skills(true),
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "Unknown skills scope: {}",
                other
            )));
        }
    };

    Ok(Json(
        skills.into_iter().map(SkillInfoResponse::from).collect(),
    ))
}

impl From<SkillInfo> for SkillInfoResponse {
    fn from(skill: SkillInfo) -> Self {
        Self {
            name: skill.name,
            description: skill.description,
            version: skill.version,
            author: skill.author,
            tags: skill.tags,
            source: skill.source.as_str().to_string(),
            origin: skill.origin,
            path: skill.path.to_string_lossy().into_owned(),
            enabled: skill.enabled,
            permission: skill.permission,
            model_invocable: skill.model_invocable,
        }
    }
}

async fn refresh_skills(
    State(state): State<AppState>,
) -> Result<Json<Vec<SkillInfoResponse>>, AppError> {
    let mut manager = state.skills_manager.write().await;
    manager.refresh();
    Ok(Json(
        manager
            .list_skills()
            .into_iter()
            .map(SkillInfoResponse::from)
            .collect(),
    ))
}

async fn list_diagnostics(
    State(state): State<AppState>,
) -> Result<Json<Vec<SkillDiagnostic>>, AppError> {
    Ok(Json(state.skills_manager.write().await.diagnostics()))
}

#[derive(Debug, Deserialize)]
struct UpdateSkillPolicy {
    enabled: Option<bool>,
    permission: Option<SkillPermission>,
}

async fn update_skill_policy(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(update): Json<UpdateSkillPolicy>,
) -> Result<Json<SkillInfoResponse>, AppError> {
    if update.enabled.is_none() && update.permission.is_none() {
        return Err(AppError::BadRequest(
            "At least one of enabled or permission is required".to_string(),
        ));
    }
    let mut manager = state.skills_manager.write().await;
    if let Some(enabled) = update.enabled {
        manager
            .set_skill_enabled(&name, enabled)
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
    }
    if let Some(permission) = update.permission {
        manager
            .set_skill_permission(&name, permission)
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
    }
    let skill = manager
        .get_skill(&name)
        .map(|skill| skill.to_info())
        .ok_or_else(|| AppError::NotFound(format!("Skill '{name}' not found")))?;
    Ok(Json(SkillInfoResponse::from(skill)))
}
