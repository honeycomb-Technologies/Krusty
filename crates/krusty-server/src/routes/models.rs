//! Model listing endpoint

use axum::{extract::State, routing::get, Json, Router};

use krusty_core::ai::providers::ProviderId;
use krusty_core::constants;

use crate::error::AppError;
use crate::types::{ModelResponse, ModelsListResponse};
use crate::AppState;

/// Build the models router
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_models))
        .route("/:id", get(get_model))
}

/// List all available models from configured providers
async fn list_models(State(state): State<AppState>) -> Result<Json<ModelsListResponse>, AppError> {
    let configured_providers: Vec<ProviderId> = ProviderId::all().to_vec();

    // Get organized models from registry
    let (recent_models, models_by_provider) = state
        .model_registry
        .get_organized_models(&configured_providers)
        .await;

    // Flatten into a single list, preserving provider grouping
    let mut models: Vec<ModelResponse> = Vec::new();

    // Add recent models first (if any)
    for m in recent_models {
        models.push(ModelResponse {
            id: m.id.clone(),
            display_name: m.display_name.clone(),
            provider: m.provider.to_string(),
            context_window: m.context_window,
            max_output: m.max_output,
            supports_thinking: m.supports_thinking,
            supports_tools: m.supports_tools,
        });
    }

    // Add models by provider in order
    for provider_id in ProviderId::all() {
        if let Some(provider_models) = models_by_provider.get(provider_id) {
            for m in provider_models {
                // Skip if already added in recent
                if models.iter().any(|existing| existing.id == m.id) {
                    continue;
                }
                models.push(ModelResponse {
                    id: m.id.clone(),
                    display_name: m.display_name.clone(),
                    provider: m.provider.to_string(),
                    context_window: m.context_window,
                    max_output: m.max_output,
                    supports_thinking: m.supports_thinking,
                    supports_tools: m.supports_tools,
                });
            }
        }
    }

    // Pick the best default: active provider's model, or first available, or hardcoded fallback
    let default_model = if let Some(ref client) = state.ai_client {
        client.config().model.clone()
    } else if let Some(first) = models.first() {
        first.id.clone()
    } else {
        constants::ai::DEFAULT_MODEL.to_string()
    };

    Ok(Json(ModelsListResponse {
        models,
        default_model,
    }))
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
            provider: model.provider.to_string(),
            context_window: model.context_window,
            max_output: model.max_output,
            supports_thinking: model.supports_thinking,
            supports_tools: model.supports_tools,
        }));
    }

    Err(AppError::NotFound(format!("Model {} not found", id)))
}
