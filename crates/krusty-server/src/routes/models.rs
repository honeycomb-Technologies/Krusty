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

fn resolve_default_model(
    models: &[ModelResponse],
    active_model: Option<&str>,
    providers_with_auth: &[ProviderId],
) -> String {
    if let Some(model) = active_model {
        return model.to_string();
    }

    if let Some(configured_model) = models.iter().find(|model| {
        ProviderId::all()
            .iter()
            .find(|provider| provider.to_string() == model.provider)
            .map(|provider| providers_with_auth.contains(provider))
            .unwrap_or(false)
    }) {
        return configured_model.id.clone();
    }

    if let Some(first) = models.first() {
        return first.id.clone();
    }

    constants::ai::DEFAULT_MODEL.to_string()
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
            supports_vision: m.supports_vision,
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
                    supports_vision: m.supports_vision,
                });
            }
        }
    }

    let providers_with_auth = {
        let store = state.credential_store.read().await;
        store.providers_with_auth()
    };

    let default_model = resolve_default_model(
        &models,
        state
            .ai_client
            .as_ref()
            .map(|client| client.config().model.as_str()),
        &providers_with_auth,
    );

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
            supports_vision: model.supports_vision,
        }));
    }

    Err(AppError::NotFound(format!("Model {} not found", id)))
}

#[cfg(test)]
mod tests {
    use super::resolve_default_model;
    use crate::types::ModelResponse;
    use krusty_core::ai::providers::ProviderId;

    fn model(id: &str, provider: &str) -> ModelResponse {
        ModelResponse {
            id: id.to_string(),
            display_name: id.to_string(),
            provider: provider.to_string(),
            context_window: 1,
            max_output: 1,
            supports_thinking: false,
            supports_tools: true,
            supports_vision: false,
        }
    }

    #[test]
    fn resolve_default_model_prefers_active_model() {
        let models = vec![
            model("MiniMax-M2.5", "MiniMax"),
            model("claude-opus-4.6", "Anthropic"),
        ];
        let default_model =
            resolve_default_model(&models, Some("claude-opus-4.6"), &[ProviderId::Anthropic]);

        assert_eq!(default_model, "claude-opus-4.6");
    }

    #[test]
    fn resolve_default_model_prefers_configured_provider_when_no_active_model() {
        let models = vec![
            model("MiniMax-M2.5", "MiniMax"),
            model("claude-opus-4.6", "Anthropic"),
        ];
        let default_model = resolve_default_model(&models, None, &[ProviderId::Anthropic]);

        assert_eq!(default_model, "claude-opus-4.6");
    }
}
