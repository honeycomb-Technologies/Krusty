use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::OwnedMutexGuard;

use krusty_core::agent::autonomy::coordinator_prompt::system_prompt_for_session;
use krusty_core::ai::client::{AiClient, CallOptions};
use krusty_core::ai::providers::ProviderId;
use krusty_core::ai::types::{ModelMessage, WebFetchConfig, WebSearchConfig};
use krusty_core::plan::PlanManager;
use krusty_core::storage::{
    Database, MakoRuntimeStateStore, ProjectSettings, SessionInfo, SessionType, WorkMode,
    WorkspaceMode,
};
use krusty_core::tools::registry::{MutationToolSurface, PermissionMode, ToolRequestPolicy};
use krusty_core::SessionManager;

use super::super::session_access::{
    current_user_id, ensure_owned_session, load_owned_session, request_workspace_scope,
};
use super::tools::{apply_thinking_config, chat_system_prompt, filter_tools_for_session_type};
use crate::ai_bootstrap::{persist_current_model_selection, resolve_preferred_model};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{ChatRequest, ThinkingLevel};
use crate::utils::messages::parse_stored_model_messages;
use crate::utils::text::trimmed_nonempty;
use crate::utils::workspace::{
    normalize_resolved_requested_workspace, resolve_optional_workspace_path,
    resolve_session_working_dir, WorkspaceNormalizationPolicy,
};
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
    pub(super) permission_mode: PermissionMode,
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

pub(super) struct PreparedChatRouteSession {
    pub(super) session_id: String,
    pub(super) is_first_message: bool,
    pub(super) pending_model_update: Option<Option<String>>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChatContinuationContract {
    pub(super) session_id: String,
    pub(super) is_first_message: bool,
    pub(super) working_dir: PathBuf,
    pub(super) project_dir: Option<PathBuf>,
    pub(super) workspace_mode: WorkspaceMode,
    pub(super) session_type: SessionType,
    pub(super) work_mode: WorkMode,
    pub(super) model: Option<String>,
    pub(super) target_branch: Option<String>,
    pub(super) permission_mode: PermissionMode,
    pub(super) fast_mode: bool,
    pub(super) user_id: Option<String>,
}

pub(super) async fn prepare_chat_route_session(
    state: &AppState,
    user: Option<&CurrentUser>,
    req: &ChatRequest,
    requested_model: RequestedModel<'_>,
    requested_session_type: SessionType,
    requires_vision: bool,
) -> Result<PreparedChatRouteSession, AppError> {
    let user_id = current_user_id(user).map(ToOwned::to_owned);
    let workspace_scope = request_workspace_scope(state, user);

    match req.session_id.as_deref() {
        Some(id) => {
            let db = Database::new(&state.db_path)?;
            let sm = SessionManager::new(db);
            ensure_owned_session(&sm, id, user)?;
            if let Some(target_branch) = req.target_branch.as_deref() {
                sm.update_session_target_branch(id, trimmed_nonempty(Some(target_branch)))?;
            }
            let messages = sm.load_session_messages(id)?;
            Ok(PreparedChatRouteSession {
                session_id: id.to_string(),
                is_first_message: messages.is_empty(),
                pending_model_update: requested_model
                    .persisted()
                    .map(|model| model.map(ToOwned::to_owned)),
            })
        }
        None => {
            let db = Database::new(&state.db_path)?;
            let sm = SessionManager::new(db);
            let title = SessionManager::generate_title_from_content(&req.message);
            let default_mode_without_paths = if requested_session_type == SessionType::Chat {
                WorkspaceMode::Neutral
            } else {
                WorkspaceMode::Selected
            };
            let default_workspace = workspace_scope.base_dir.to_string_lossy().to_string();
            let workspace = normalize_resolved_requested_workspace(
                req.working_dir.as_deref(),
                req.project_dir.as_deref(),
                req.workspace_mode,
                WorkspaceNormalizationPolicy {
                    default_mode_without_paths,
                    selected_fallback_dir: Some(default_workspace.as_str()),
                },
                &workspace_scope.base_dir,
                &workspace_scope.allowed_root,
            )?;
            let preferred_model =
                resolve_preferred_model(state.db_path.as_ref().as_path(), user_id.as_deref());
            let initial_model = select_model_for_chat_request(
                state,
                requested_model,
                preferred_model
                    .as_deref()
                    .filter(|_| matches!(requested_model, RequestedModel::Unspecified)),
                requires_vision,
            )
            .await?;
            if initial_model.is_none() && preferred_model.is_none() {
                return Err(AppError::BadRequest(
                    "No model selected. Choose a model and try again.".to_string(),
                ));
            }
            let initial_target_branch = trimmed_nonempty(req.target_branch.as_deref());
            let session_id = sm.create_session_for_user_with_config_and_permission(
                &title,
                initial_model.as_deref(),
                workspace.working_dir.as_deref(),
                workspace.project_dir.as_deref(),
                workspace.workspace_mode,
                user_id.as_deref(),
                initial_target_branch,
                requested_session_type,
                req.permission_mode.unwrap_or_default(),
            )?;
            let should_persist_current_model = match requested_model {
                RequestedModel::Set(_) => true,
                RequestedModel::Unspecified => {
                    preferred_model.as_deref() == initial_model.as_deref()
                }
                RequestedModel::Clear => false,
            };
            if should_persist_current_model {
                if let Some(model) = initial_model.as_deref() {
                    persist_current_model_selection(
                        &state.model_registry,
                        state.db_path.as_ref().as_path(),
                        user_id.as_deref(),
                        model,
                    )
                    .await?;
                }
            }

            Ok(PreparedChatRouteSession {
                session_id,
                is_first_message: true,
                pending_model_update: None,
            })
        }
    }
}

#[cfg(test)]
pub(super) fn build_chat_continuation_contract(
    session: &SessionInfo,
    working_dir: PathBuf,
    project_dir: Option<PathBuf>,
    work_mode: WorkMode,
    fast_mode: bool,
    is_first_message: bool,
) -> ChatContinuationContract {
    ChatContinuationContract {
        session_id: session.id.clone(),
        is_first_message,
        working_dir,
        project_dir,
        workspace_mode: session.workspace_mode,
        session_type: session.session_type,
        work_mode,
        model: session.model.clone(),
        target_branch: session.target_branch.clone(),
        permission_mode: session.permission_mode,
        fast_mode,
        user_id: session.user_id.clone(),
    }
}

#[cfg(test)]
pub(super) async fn load_chat_continuation_contract(
    state: &AppState,
    user: Option<&CurrentUser>,
    prepared: &PreparedChatRouteSession,
    fast_mode: bool,
) -> Result<ChatContinuationContract, AppError> {
    let workspace_scope = request_workspace_scope(state, user);
    let db = Database::new(&state.db_path)?;
    let session_manager = SessionManager::new(db);
    let session = load_owned_session(&session_manager, &prepared.session_id, user)?;
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
    let work_mode = effective_session_work_mode(state, &session);

    Ok(build_chat_continuation_contract(
        &session,
        working_dir,
        project_dir,
        work_mode,
        fast_mode,
        prepared.is_first_message,
    ))
}

#[cfg(test)]
pub(super) async fn prepare_chat_contract_for_test(
    state: &AppState,
    user: Option<CurrentUser>,
    req: ChatRequest,
) -> Result<ChatContinuationContract, AppError> {
    let requested_model = RequestedModel::from_request(req.model.as_deref());
    let requested_session_type = req.session_type.unwrap_or(SessionType::Code);
    let requires_vision = false;
    let prepared = prepare_chat_route_session(
        state,
        user.as_ref(),
        &req,
        requested_model,
        requested_session_type,
        requires_vision,
    )
    .await?;

    load_chat_continuation_contract(state, user.as_ref(), &prepared, req.fast_mode).await
}

fn effective_session_work_mode(state: &AppState, session: &SessionInfo) -> WorkMode {
    PlanManager::new((*state.db_path).clone())
        .ok()
        .and_then(|pm| pm.get_lifecycle_state(&session.id, session.work_mode).ok())
        .map(|state| state.effective_work_mode)
        .unwrap_or(session.work_mode)
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
    fast_mode: bool,
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
    let effective_work_mode = effective_session_work_mode(state, &session);
    let has_active_plan = PlanManager::new((*state.db_path).clone())
        .ok()
        .and_then(|manager| manager.get_active_plan(session_id).ok())
        .flatten()
        .is_some();
    let project_settings = ProjectSettings::load(project_dir.as_deref().unwrap_or(&working_dir));

    let guard = state
        .try_lock_session(session_id)
        .await
        .ok_or_else(|| AppError::Conflict(format!("Session {} is busy", session_id)))?;

    let recovered_steering = session_manager.promote_orphaned_pending_steering(session_id)?;
    if recovered_steering > 0 {
        tracing::info!(
            session_id,
            recovered_steering,
            "Recovered durable steering left by an interrupted active run"
        );
    }

    let raw_messages = session_manager.load_session_messages(session_id)?;
    let conversation = parse_stored_model_messages(session_id, raw_messages, "chat conversation");

    tracing::info!(
        session_type = ?session.session_type,
        session_id = %session_id,
        "Filtering tools for session type"
    );
    let all_tools = state.tool_registry.get_ai_tools_all().await;
    let ai_tools = if session.session_type == SessionType::Code {
        ToolRequestPolicy::code(
            session.permission_mode,
            effective_work_mode == WorkMode::Plan,
            has_active_plan,
            true,
            project_settings.disabled_tools.as_deref().unwrap_or(&[]),
        )
        .with_mutation_surface(MutationToolSurface::for_model(
            ai_client.provider_id(),
            &ai_client.config().model,
        ))
        .filter(all_tools)
    } else {
        let disabled_tools = project_settings.disabled_tools.unwrap_or_default();
        filter_tools_for_session_type(all_tools, session.session_type, research_enabled)
            .into_iter()
            .filter(|tool| !disabled_tools.iter().any(|name| name == &tool.name))
            .collect()
    };
    let hosted_web_tools = session.session_type != SessionType::Code;
    let mut options = CallOptions {
        tools: if ai_tools.is_empty() {
            None
        } else {
            Some(ai_tools)
        },
        session_id: Some(session_id.to_string()),
        codex_parallel_tool_calls: true,
        web_search: hosted_web_tools.then(WebSearchConfig::default),
        web_fetch: hosted_web_tools.then(WebFetchConfig::default),
        fast_mode,
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
        permission_mode: session.permission_mode,
        mako_crew_slug: mako_runtime.and_then(|runtime| runtime.crew_slug),
        user_id,
        guard,
    })
}
