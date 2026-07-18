use std::path::Path as StdPath;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use krusty_core::storage::{SessionType, WorkspaceMode};

use super::{current_user_id, load_owned_session, open_session_manager, request_workspace_scope};
use crate::ai_bootstrap::persist_current_model_selection;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{
    CreateSessionRequest, MessageResponse, SessionResponse, SessionWithMessagesResponse,
    UpdateSessionRequest,
};
use crate::utils::text::trimmed_nonempty;
use crate::utils::workspace::{
    normalize_resolved_requested_workspace, resolve_optional_workspace_path, NormalizedWorkspace,
    WorkspaceNormalizationPolicy,
};
use crate::AppState;

/// Query params for listing sessions
#[derive(Debug, Deserialize)]
pub(super) struct ListSessionsQuery {
    /// Filter sessions by working directory
    pub working_dir: Option<String>,
}

/// Query params for retrieving a session with messages (pagination)
#[derive(Debug, Deserialize)]
pub(super) struct GetSessionQuery {
    /// Maximum number of messages to return
    pub limit: Option<usize>,
    /// Number of messages to skip (from the beginning)
    pub offset: Option<usize>,
}

/// List all sessions, optionally filtered by working directory
pub(super) async fn list_sessions(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionResponse>>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());
    let working_dir_filter = resolve_optional_workspace_path(
        query.working_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let sessions =
        session_manager.list_sessions_for_user(working_dir_filter.as_deref(), user_id)?;
    let response: Vec<SessionResponse> = sessions.into_iter().map(Into::into).collect();

    Ok(Json(response))
}

/// List all directories that have sessions
pub(super) async fn list_directories(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<Vec<String>>, AppError> {
    let session_manager = open_session_manager(&state)?;

    let user_id = user.as_ref().and_then(|u| u.0.user_id.as_deref());
    let directories = session_manager.list_session_directories_for_user(user_id)?;

    Ok(Json(directories))
}

/// Create a new session
pub(super) async fn create_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionResponse>), AppError> {
    let session_type = req.session_type.unwrap_or(SessionType::Code);
    if session_type == SessionType::Mako {
        return Err(AppError::Conflict(
            "Mako sessions must be created through POST /mako/dispatch so the daemon owns the durable controller".into(),
        ));
    }
    let session_manager = open_session_manager(&state)?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());

    // Prefer an explicit placeholder so list/search UIs never depend on every
    // client synthesizing titles for empty strings.
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New Session");
    let requested_model = trimmed_nonempty(req.model.as_deref());
    let workspace = normalize_resolved_requested_workspace(
        req.working_dir.as_deref(),
        req.project_dir.as_deref(),
        req.workspace_mode,
        WorkspaceNormalizationPolicy {
            default_mode_without_paths: WorkspaceMode::Neutral,
            selected_fallback_dir: None,
        },
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;
    validate_workspace_payload(
        &workspace,
        trimmed_nonempty(req.project_dir.as_deref())
            .or(trimmed_nonempty(req.working_dir.as_deref())),
    )?;
    let target_branch = req
        .target_branch
        .as_deref()
        .map(str::trim)
        .filter(|branch| !branch.is_empty());
    let session_id = session_manager.create_session_for_user_with_config_and_permission(
        title,
        requested_model,
        workspace.working_dir.as_deref(),
        workspace.project_dir.as_deref(),
        workspace.workspace_mode,
        current_user_id(user.as_ref()),
        target_branch,
        session_type,
        req.permission_mode.unwrap_or_default(),
    )?;

    if let Some(model) = requested_model {
        persist_current_model_selection(
            &state.model_registry,
            state.db_path.as_ref().as_path(),
            current_user_id(user.as_ref()),
            model,
        )
        .await?;
    }

    let session = session_manager
        .get_session(&session_id)?
        .ok_or_else(|| AppError::Internal("Failed to fetch created session".to_string()))?;

    Ok((StatusCode::CREATED, Json(session.into())))
}

/// Get a session with its messages, with optional pagination
pub(super) async fn get_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Query(query): Query<GetSessionQuery>,
) -> Result<Json<SessionWithMessagesResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let session = load_owned_session(&session_manager, &id, user.as_ref())?;

    let raw_messages = session_manager.load_session_messages(&id)?;
    let offset = query.offset.unwrap_or(0);
    const MAX_MESSAGE_LIMIT: usize = 10_000;
    let limit = query
        .limit
        .unwrap_or(MAX_MESSAGE_LIMIT)
        .min(MAX_MESSAGE_LIMIT);

    let messages: Vec<MessageResponse> = raw_messages
        .into_iter()
        .skip(offset)
        .take(limit)
        .filter_map(
            |(role, content_json)| match serde_json::from_str(&content_json) {
                Ok(content) => Some(MessageResponse { role, content }),
                Err(e) => {
                    tracing::warn!("Failed to parse message content for role '{}': {}", role, e);
                    None
                }
            },
        )
        .collect();

    Ok(Json(SessionWithMessagesResponse {
        session: session.into(),
        messages,
    }))
}

/// Update a session's title and workspace settings.
pub(super) async fn update_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<SessionResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let session = load_owned_session(&session_manager, &id, user.as_ref())?;
    if session.session_type == SessionType::Mako {
        return Err(AppError::Conflict(
            "Mako session metadata is daemon-owned and cannot be changed through /sessions".into(),
        ));
    }
    let workspace_scope = request_workspace_scope(&state, user.as_ref());

    if req.title.is_none()
        && req.working_dir.is_none()
        && req.project_dir.is_none()
        && req.workspace_mode.is_none()
        && req.mode.is_none()
        && req.model.is_none()
        && req.target_branch.is_none()
        && req.permission_mode.is_none()
    {
        return Err(AppError::BadRequest(
            "At least one of title, working_dir, project_dir, workspace_mode, mode, model, target_branch, or permission_mode must be provided".to_string(),
        ));
    }

    if let Some(title) = req.title.as_deref() {
        session_manager.update_session_title(&id, title)?;
    }

    let workspace_update = resolve_workspace_update(
        &req,
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;

    if workspace_update.is_none() {
        let working_dir = resolve_optional_workspace_path(
            req.working_dir.as_deref(),
            &workspace_scope.base_dir,
            &workspace_scope.allowed_root,
        )?;
        if req.working_dir.is_some() {
            session_manager.update_session_working_dir(&id, working_dir.as_deref())?;
        }
    }

    if let Some(workspace) = workspace_update {
        session_manager.update_session_workspace(
            &id,
            workspace.project_dir.as_deref(),
            workspace.workspace_mode,
        )?;
    }

    if let Some(mode) = req.mode {
        session_manager.update_session_work_mode(&id, mode)?;
    }

    if let Some(model) = req.model.as_deref() {
        let normalized = trimmed_nonempty(Some(model));
        session_manager.update_session_model(&id, normalized)?;
        if let Some(model) = normalized {
            persist_current_model_selection(
                &state.model_registry,
                state.db_path.as_ref().as_path(),
                current_user_id(user.as_ref()),
                model,
            )
            .await?;
        }
    }

    if let Some(target_branch) = req.target_branch.as_ref() {
        let normalized = target_branch
            .as_deref()
            .and_then(|target_branch| trimmed_nonempty(Some(target_branch)));
        session_manager.update_session_target_branch(&id, normalized)?;
    }

    if let Some(permission_mode) = req.permission_mode {
        session_manager.update_session_permission_mode(&id, permission_mode)?;
    }

    let session = session_manager
        .get_session(&id)?
        .ok_or_else(|| AppError::Internal("Failed to fetch updated session".to_string()))?;

    Ok(Json(session.into()))
}

/// Delete a session
pub(super) async fn delete_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let session_manager = open_session_manager(&state)?;
    let session = load_owned_session(&session_manager, &id, user.as_ref())?;

    if session.session_type == SessionType::Mako {
        state
            .mako_runtime
            .delete_session_for_user(&state, &id, current_user_id(user.as_ref()), None)
            .await
            .map_err(crate::mako_runtime::control_plane_app_error)?;
        return Ok(StatusCode::NO_CONTENT);
    }

    session_manager.delete_session(&id)?;

    let mut locks = state.session_locks.write().await;
    locks.remove(&id);

    Ok(StatusCode::NO_CONTENT)
}

fn resolve_workspace_update(
    req: &UpdateSessionRequest,
    workspace_base: &StdPath,
    allowed_root: &StdPath,
) -> Result<Option<NormalizedWorkspace>, AppError> {
    if req.project_dir.is_none() && req.workspace_mode.is_none() {
        return Ok(None);
    }

    let project_hint = trimmed_nonempty(req.project_dir.as_deref())
        .or(trimmed_nonempty(req.working_dir.as_deref()));
    let workspace = normalize_resolved_requested_workspace(
        req.working_dir.as_deref(),
        req.project_dir.as_deref(),
        req.workspace_mode,
        WorkspaceNormalizationPolicy {
            default_mode_without_paths: WorkspaceMode::Neutral,
            selected_fallback_dir: None,
        },
        workspace_base,
        allowed_root,
    )?;

    validate_workspace_payload(&workspace, project_hint)?;
    Ok(Some(workspace))
}

fn validate_workspace_payload(
    workspace: &NormalizedWorkspace,
    project_hint: Option<&str>,
) -> Result<(), AppError> {
    match workspace.workspace_mode {
        WorkspaceMode::Neutral if project_hint.is_some() => Err(AppError::BadRequest(
            "workspace mode 'neutral' cannot include a project_dir".to_string(),
        )),
        WorkspaceMode::Selected | WorkspaceMode::Created if workspace.project_dir.is_none() => {
            Err(AppError::BadRequest(
                "workspace modes 'selected' and 'created' require a project_dir".to_string(),
            ))
        }
        _ => Ok(()),
    }
}
