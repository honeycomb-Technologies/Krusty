use axum::{
    extract::{Path, State},
    Json,
};

use krusty_core::agent::{create_pinched_session, CreatePinchedSessionRequest};
use krusty_core::storage::WorkspaceMode;
use std::path::Path as StdPath;

use super::{load_owned_session, open_session_manager, request_workspace_scope};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::{PinchRequest, PinchResponse};
use crate::utils::messages::parse_stored_model_messages;
use crate::utils::workspace::{resolve_optional_workspace_path, resolve_session_working_dir};
use crate::AppState;

/// Pinch a session - create a child session with summarized context
pub(super) async fn pinch_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<PinchRequest>,
) -> Result<Json<PinchResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    let source_session = load_owned_session(&session_manager, &id, user.as_ref())?;
    let workspace_scope = request_workspace_scope(&state, user.as_ref());

    let raw_messages = session_manager.load_session_messages(&id)?;
    let messages = parse_stored_model_messages(&id, raw_messages, "pinch context");

    if messages.is_empty() {
        return Err(AppError::BadRequest(
            "Cannot pinch session with no messages".to_string(),
        ));
    }

    let working_dir = resolve_session_working_dir(
        source_session.working_dir.as_deref(),
        &workspace_scope.base_dir,
        &workspace_scope.allowed_root,
    )?;
    let legacy_relative_workspace =
        has_relative_workspace_path(source_session.working_dir.as_deref())
            || has_relative_workspace_path(source_session.project_dir.as_deref());
    let resolved_working_dir = working_dir.to_string_lossy().into_owned();
    let resolved_project_dir =
        if legacy_relative_workspace && source_session.workspace_mode != WorkspaceMode::Neutral {
            Some(
                resolve_optional_workspace_path(
                    source_session.project_dir.as_deref(),
                    &workspace_scope.base_dir,
                    &workspace_scope.allowed_root,
                )?
                .unwrap_or_else(|| resolved_working_dir.clone()),
            )
        } else {
            None
        };

    let summary_model = source_session.model.as_deref();
    let summary_client = state
        .resolve_ai_client_for_user(summary_model, source_session.user_id.as_deref())
        .await;
    let pinch_result = create_pinched_session(CreatePinchedSessionRequest {
        db_path: &state.db_path,
        ai_client: summary_client.as_ref().map(|client| client.as_ref()),
        session_id: &id,
        source_session_title: &source_session.title,
        conversation: &messages,
        working_dir: &working_dir,
        model: summary_model,
        target_branch: source_session.target_branch.as_deref(),
        permission_mode: source_session.permission_mode,
        preservation_hints: req.preservation_hints,
        direction: req.direction,
        initial_user_message: None,
    })
    .await?;

    if legacy_relative_workspace {
        let (working_dir, project_dir) = match source_session.workspace_mode {
            WorkspaceMode::Neutral => (None, None),
            WorkspaceMode::Selected | WorkspaceMode::Created => (
                Some(resolved_working_dir.as_str()),
                resolved_project_dir.as_deref(),
            ),
        };
        session_manager.update_session_workspace_contract(
            &pinch_result.new_session_id,
            working_dir,
            project_dir,
            source_session.workspace_mode,
        )?;
    }

    let new_session = session_manager
        .get_session(&pinch_result.new_session_id)?
        .ok_or_else(|| AppError::Internal("Failed to fetch new session".to_string()))?;

    Ok(Json(PinchResponse {
        session: new_session.into(),
        summary: pinch_result.summary.work_summary,
        key_decisions: pinch_result.summary.key_decisions,
        pending_tasks: pinch_result.summary.pending_tasks,
    }))
}

fn has_relative_workspace_path(path: Option<&str>) -> bool {
    path.map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|path| !StdPath::new(path).is_absolute())
        .unwrap_or(false)
}
