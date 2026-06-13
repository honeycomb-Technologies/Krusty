//! Model listing endpoint

use axum::{
    extract::State,
    routing::{get, put},
    Json, Router,
};
use serde::Deserialize;

use krusty_core::ai::providers::ProviderId;

use crate::ai_bootstrap::{
    clear_current_model_preference, persist_current_model_selection, resolve_preferred_model,
};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{ModelResponse, ModelsListResponse, SimpleOkResponse};
use crate::AppState;

/// Build the models router
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_models))
        .route("/current", put(set_current_model))
        .route("/:id", get(get_model))
}

#[derive(Debug, Deserialize)]
struct CurrentModelRequest {
    model: Option<String>,
}

fn resolve_default_model(active_model: Option<&str>) -> Option<String> {
    active_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToOwned::to_owned)
}

/// List all available models from configured providers
async fn list_models(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<ModelsListResponse>, AppError> {
    let configured_providers: Vec<ProviderId> = ProviderId::all().to_vec();

    let (recent_models, models_by_provider) = state
        .model_registry
        .get_organized_models(&configured_providers)
        .await;

    let mut models: Vec<ModelResponse> = Vec::new();

    for m in recent_models {
        models.push(ModelResponse {
            id: m.id.clone(),
            display_name: m.display_name.clone(),
            provider: crate::utils::providers::provider_display_name(m.provider).to_string(),
            context_window: m.context_window,
            max_output: m.max_output,
            supports_thinking: m.supports_thinking,
            supports_tools: m.supports_tools,
            supports_vision: m.supports_vision,
        });
    }

    for provider_id in ProviderId::all() {
        if let Some(provider_models) = models_by_provider.get(provider_id) {
            for m in provider_models {
                if models.iter().any(|existing| existing.id == m.id) {
                    continue;
                }
                models.push(ModelResponse {
                    id: m.id.clone(),
                    display_name: m.display_name.clone(),
                    provider: crate::utils::providers::provider_display_name(m.provider)
                        .to_string(),
                    context_window: m.context_window,
                    max_output: m.max_output,
                    supports_thinking: m.supports_thinking,
                    supports_tools: m.supports_tools,
                    supports_vision: m.supports_vision,
                });
            }
        }
    }

    let user_id = user
        .as_ref()
        .and_then(|current_user| current_user.0.user_id.as_deref());
    let default_model = resolve_default_model(
        resolve_preferred_model(state.db_path.as_ref().as_path(), user_id).as_deref(),
    );

    Ok(Json(ModelsListResponse {
        models,
        default_model,
    }))
}

async fn set_current_model(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<CurrentModelRequest>,
) -> Result<Json<SimpleOkResponse>, AppError> {
    let user_id = user
        .as_ref()
        .and_then(|current_user| current_user.0.user_id.as_deref());
    let next_model = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty());

    if let Some(model_id) = next_model {
        if state.model_registry.get_model(model_id).await.is_none() {
            return Err(AppError::BadRequest(format!(
                "Model '{}' is not available",
                model_id
            )));
        }
        persist_current_model_selection(
            &state.model_registry,
            state.db_path.as_ref().as_path(),
            user_id,
            model_id,
        )
        .await?;
    } else {
        clear_current_model_preference(state.db_path.as_ref().as_path(), user_id)?;
    }

    Ok(Json(SimpleOkResponse { ok: true }))
}

/// Get a specific model by ID
async fn get_model(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ModelResponse>, AppError> {
    if let Some(model) = state.model_registry.get_model(&id).await {
        return Ok(Json(ModelResponse {
            id: model.id.clone(),
            display_name: model.display_name.clone(),
            provider: crate::utils::providers::provider_display_name(model.provider).to_string(),
            context_window: model.context_window,
            max_output: model.max_output,
            supports_thinking: model.supports_thinking,
            supports_tools: model.supports_tools,
            supports_vision: model.supports_vision,
        }));
    }

    Err(AppError::NotFound(format!("Model {} not found", id)))
}

#[cfg(test)]
mod tests {
    use super::resolve_default_model;

    #[test]
    fn resolve_default_model_prefers_active_model() {
        let default_model = resolve_default_model(Some("claude-opus-4.6"));
        assert_eq!(default_model.as_deref(), Some("claude-opus-4.6"));
    }

    #[test]
    fn resolve_default_model_returns_none_when_no_model_is_selected() {
        assert_eq!(resolve_default_model(None), None);
    }
}
