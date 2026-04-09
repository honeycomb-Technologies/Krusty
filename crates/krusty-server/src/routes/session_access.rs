use std::path::{Path, PathBuf};

use krusty_core::storage::{AgentState, SessionInfo, SessionType};
use krusty_core::SessionManager;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::workspace::{allowed_root, workspace_base};
use crate::AppState;

pub(super) struct RequestWorkspaceScope {
    pub base_dir: PathBuf,
    pub allowed_root: PathBuf,
}

pub(super) fn current_user_id(user: Option<&CurrentUser>) -> Option<&str> {
    user.and_then(|u| u.0.user_id.as_deref())
}

pub(super) fn current_user_home_dir(user: Option<&CurrentUser>) -> Option<&Path> {
    user.and_then(|u| u.0.home_dir.as_deref())
}

pub(super) fn request_workspace_scope(
    state: &AppState,
    user: Option<&CurrentUser>,
) -> RequestWorkspaceScope {
    let home_dir = current_user_home_dir(user);
    let base_dir = workspace_base(home_dir, state.working_dir.as_ref());
    let allowed_root = allowed_root(home_dir, &base_dir);

    RequestWorkspaceScope {
        base_dir,
        allowed_root,
    }
}

pub(super) fn session_visible_to_user(session: &SessionInfo, user_id: Option<&str>) -> bool {
    match user_id {
        Some(user_id) => session.user_id.as_deref() == Some(user_id),
        None => true,
    }
}

pub(super) fn ensure_owned_session(
    session_manager: &SessionManager,
    session_id: &str,
    user: Option<&CurrentUser>,
) -> Result<(), AppError> {
    load_owned_session(session_manager, session_id, user).map(|_| ())
}

pub(super) fn load_owned_session(
    session_manager: &SessionManager,
    session_id: &str,
    user: Option<&CurrentUser>,
) -> Result<SessionInfo, AppError> {
    let session = session_manager
        .get_session(session_id)?
        .ok_or_else(|| session_not_found(session_id))?;
    if !session_visible_to_user(&session, current_user_id(user)) {
        return Err(session_not_found(session_id));
    }
    Ok(session)
}

pub(super) fn ensure_owned_session_of_type(
    session_manager: &SessionManager,
    session_id: &str,
    session_type: SessionType,
    session_type_label: &str,
    user: Option<&CurrentUser>,
) -> Result<(), AppError> {
    load_owned_session_of_type(
        session_manager,
        session_id,
        session_type,
        session_type_label,
        user,
    )
    .map(|_| ())
}

pub(super) fn load_owned_session_of_type(
    session_manager: &SessionManager,
    session_id: &str,
    session_type: SessionType,
    session_type_label: &str,
    user: Option<&CurrentUser>,
) -> Result<SessionInfo, AppError> {
    let session = load_owned_session(session_manager, session_id, user)?;
    if session.session_type != session_type {
        return Err(AppError::BadRequest(format!(
            "Session {} is not a {} session",
            session_id, session_type_label
        )));
    }
    Ok(session)
}

pub(super) fn load_agent_state_or_idle(
    session_manager: &SessionManager,
    session_id: &str,
) -> Result<AgentState, AppError> {
    Ok(session_manager
        .try_get_agent_state(session_id)?
        .unwrap_or_else(idle_agent_state))
}

fn session_not_found(session_id: &str) -> AppError {
    AppError::NotFound(format!("Session {} not found", session_id))
}

fn idle_agent_state() -> AgentState {
    AgentState {
        state: "idle".to_string(),
        started_at: None,
        last_event_at: None,
    }
}
