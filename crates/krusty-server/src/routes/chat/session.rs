use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{Mutex, OwnedMutexGuard};

use krusty_core::agent::autonomy::coordinator_prompt::system_prompt_for_session;
use krusty_core::ai::client::{AiClient, CallOptions};
use krusty_core::ai::providers::ProviderId;
use krusty_core::ai::types::ModelMessage;
use krusty_core::plan::PlanManager;
use krusty_core::storage::{Database, MakoRuntimeStateStore, SessionType, WorkMode};
use krusty_core::SessionManager;

use super::super::session_access::{current_user_id, load_owned_session, request_workspace_scope};
use super::tools::{apply_thinking_config, chat_system_prompt, filter_tools_for_session_type};
use super::{SESSION_LOCK_MAX_AGE, SESSION_LOCK_MAX_ENTRIES};
use crate::ai_bootstrap::resolve_preferred_model;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::ThinkingLevel;
use crate::utils::messages::parse_stored_model_messages;
use crate::utils::text::trimmed_nonempty;
use crate::utils::workspace::{resolve_optional_workspace_path, resolve_session_working_dir};
use crate::AppState;

pub(super) struct ChatSessionContext {
    pub(super) ai_client: Arc<AiClient>,
    pub(super) options: CallOptions,
    pub(super) conversation: Vec<ModelMessage>,
    pub(super) session_id: String,
    pub(super) session_manager: SessionManager,
    pub(super) working_dir: PathBuf,
    pub(super) project_dir: Option<PathBuf>,
    pub(super) work_mode: WorkMode,
    pub(super) session_type: SessionType,
    pub(super) mako_crew_slug: Option<String>,
    pub(super) user_id: Option<String>,
    pub(super) guard: OwnedMutexGuard<()>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RequestedModel<'a> {
    Unspecified,
    Clear,
    Set(&'a str),
}

impl<'a> RequestedModel<'a> {
    pub(super) fn from_request(value: Option<&'a str>) -> Self {
        match value {
            Some(raw) => trimmed_nonempty(Some(raw))
                .map(Self::Set)
                .unwrap_or(Self::Clear),
            None => Self::Unspecified,
        }
    }

    pub(super) fn effective(self, session_model: Option<&'a str>) -> Option<&'a str> {
        match self {
            Self::Unspecified => trimmed_nonempty(session_model),
            Self::Clear => None,
            Self::Set(model) => Some(model),
        }
    }

    pub(super) fn persisted(self) -> Option<Option<&'a str>> {
        match self {
            Self::Unspecified => None,
            Self::Clear => Some(None),
            Self::Set(model) => Some(Some(model)),
        }
    }
}

async fn model_supports_vision(state: &AppState, model_id: &str) -> bool {
    if let Some(metadata) = state.model_registry.get_model(model_id).await {
        return metadata.supports_vision;
    }

    let Some(ai_client) = state.resolve_ai_client(Some(model_id)).await else {
        return false;
    };

    krusty_core::ai::models::resolve_model_metadata(
        ai_client.provider_id(),
        model_id,
        ai_client.config().api_format,
    )
    .supports_vision
}

async fn resolve_default_vision_model(state: &AppState) -> Option<String> {
    let providers_with_auth = {
        let store = state.credential_store.read().await;
        store.providers_with_auth()
    };

    let (recent_models, models_by_provider) = state
        .model_registry
        .get_organized_models(&providers_with_auth)
        .await;

    if let Some(model) = recent_models.iter().find(|model| model.supports_vision) {
        return Some(model.id.clone());
    }

    for provider in ProviderId::all() {
        if !providers_with_auth.contains(provider) {
            continue;
        }
        if let Some(models) = models_by_provider.get(provider) {
            if let Some(model) = models.iter().find(|model| model.supports_vision) {
                return Some(model.id.clone());
            }
        }
    }

    None
}

pub(super) async fn select_model_for_chat_request(
    state: &AppState,
    requested_model: RequestedModel<'_>,
    session_model: Option<&str>,
    requires_vision: bool,
) -> Result<Option<String>, AppError> {
    let effective_model = requested_model
        .effective(session_model)
        .map(ToOwned::to_owned);
    if !requires_vision {
        return Ok(effective_model);
    }

    if let RequestedModel::Set(model) = requested_model {
        if !model_supports_vision(state, model).await {
            return Err(AppError::BadRequest(format!(
                "Model '{}' does not support image input. Select a vision-capable model and try again.",
                model
            )));
        }
    }

    if let Some(model_id) = effective_model.as_deref() {
        if model_supports_vision(state, model_id).await {
            return Ok(effective_model);
        }
    }

    resolve_default_vision_model(state)
        .await
        .map(Some)
        .ok_or_else(|| {
            AppError::BadRequest(
                "No vision-capable model is configured. Configure OpenAI, Anthropic, or another vision model before sending images.".to_string(),
            )
        })
}

pub(super) async fn setup_chat_session(
    state: &AppState,
    user: Option<&CurrentUser>,
    session_id: &str,
    requested_model: RequestedModel<'_>,
    thinking_level: ThinkingLevel,
    research_enabled: bool,
    requires_vision: bool,
) -> Result<ChatSessionContext, AppError> {
    let user_id = current_user_id(user).map(ToOwned::to_owned);
    let workspace_scope = request_workspace_scope(state, user);

    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);
    let session = load_owned_session(&session_manager, session_id, user)?;

    let session_model = trimmed_nonempty(session.model.as_deref());
    let effective_model =
        select_model_for_chat_request(state, requested_model, session_model, requires_vision)
            .await?;
    let selected_model = effective_model.clone().or_else(|| {
        resolve_preferred_model(state.db_path.as_ref().as_path(), session.user_id.as_deref())
    });
    if selected_model.is_none() {
        return Err(AppError::BadRequest(
            "No model selected. Choose a model and try again.".to_string(),
        ));
    }

    let ai_client = state
        .resolve_ai_client_for_user(effective_model.as_deref(), session.user_id.as_deref())
        .await
        .ok_or_else(|| AppError::BadRequest("No AI credentials configured".to_string()))?;
    let resolved_model = ai_client.config().model.clone();

    let should_persist_model = (requires_vision && effective_model.as_deref() != session_model)
        || (matches!(requested_model, RequestedModel::Unspecified)
            && session_model.is_none()
            && effective_model.is_none());
    if should_persist_model {
        session_manager.update_session_model(session_id, Some(resolved_model.as_str()))?;
    }

    let working_dir = resolve_session_working_dir(
        session.working_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;
    let project_dir = resolve_optional_workspace_path(
        session.project_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?
    .map(PathBuf::from);

    let session_lock = {
        let mut locks = state.session_locks.write().await;
        if locks.len() > SESSION_LOCK_MAX_ENTRIES {
            locks.retain(|_, (lock, created_at)| {
                created_at.elapsed() < SESSION_LOCK_MAX_AGE || Arc::strong_count(lock) > 1
            });
        }
        let (lock, _) = locks
            .entry(session_id.to_string())
            .or_insert_with(|| (Arc::new(Mutex::new(())), Instant::now()));
        lock.clone()
    };
    let guard = Arc::clone(&session_lock)
        .try_lock_owned()
        .map_err(|_| AppError::Conflict(format!("Session {} is busy", session_id)))?;

    let raw_messages = session_manager.load_session_messages(session_id)?;
    let conversation = parse_stored_model_messages(session_id, raw_messages, "chat conversation");

    tracing::info!(
        session_type = ?session.session_type,
        session_id = %session_id,
        "Filtering tools for session type"
    );
    let ai_tools = filter_tools_for_session_type(
        state.tool_registry.get_ai_tools().await,
        session.session_type,
        research_enabled,
    );
    let mut options = CallOptions {
        tools: if ai_tools.is_empty() {
            None
        } else {
            Some(ai_tools)
        },
        session_id: Some(session_id.to_string()),
        codex_parallel_tool_calls: true,
        system_prompt: match session.session_type {
            SessionType::Chat => Some(chat_system_prompt(research_enabled)),
            SessionType::Mako => system_prompt_for_session(SessionType::Mako),
            SessionType::Code => None, // uses default Krusty coding assistant prompt
        },
        ..Default::default()
    };
    if thinking_level.is_enabled() {
        apply_thinking_config(&ai_client, thinking_level, &mut options);
    }

    let effective_work_mode = PlanManager::new((*state.db_path).clone())
        .ok()
        .and_then(|pm| pm.get_lifecycle_state(session_id, session.work_mode).ok())
        .map(|state| state.effective_work_mode)
        .unwrap_or(session.work_mode);
    let mako_runtime = if session.session_type == SessionType::Mako {
        MakoRuntimeStateStore::new(Database::new(&state.db_path)?).get_state(session_id)?
    } else {
        None
    };

    Ok(ChatSessionContext {
        ai_client,
        options,
        conversation,
        session_id: session_id.to_string(),
        session_manager,
        working_dir,
        project_dir,
        work_mode: effective_work_mode,
        session_type: session.session_type,
        mako_crew_slug: mako_runtime.and_then(|runtime| runtime.crew_slug),
        user_id,
        guard,
    })
}
