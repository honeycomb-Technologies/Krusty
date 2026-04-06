//! Skill listing endpoints for frontend settings/diagnostics.

use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use krusty_core::skills::SkillSource;

use crate::error::AppError;
use crate::AppState;

/// Build the skills router.
pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_skills))
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
}

async fn list_skills(
    State(state): State<AppState>,
    Query(query): Query<SkillsQuery>,
) -> Result<Json<Vec<SkillInfoResponse>>, AppError> {
    let mut skills_manager = state.skills_manager.write().await;
    let skills = match query.scope.as_deref() {
        None | Some("all") => skills_manager.list_skills(),
        Some("global") => skills_manager.list_global_skills(),
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "Unknown skills scope: {}",
                other
            )));
        }
    };

    Ok(Json(
        skills
            .into_iter()
            .map(|skill| SkillInfoResponse {
                name: skill.name,
                description: skill.description,
                version: skill.version,
                author: skill.author,
                tags: skill.tags,
                source: match skill.source {
                    SkillSource::Global => "global",
                    SkillSource::Project => "project",
                }
                .to_string(),
            })
            .collect(),
    ))
}
