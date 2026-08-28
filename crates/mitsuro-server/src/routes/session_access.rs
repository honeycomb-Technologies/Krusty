use std::path::{Path, PathBuf};

use mitsuro_core::storage::{AgentState, Database, HiveWorkerStore, SessionInfo, SessionType};
use mitsuro_core::SessionManager;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::utils::workspace::{allowed_root, workspace_base};
use crate::AppState;

pub(super) struct RequestWorkspaceScope {
    pub base_dir: PathBuf,
    pub allowed_root: PathBuf,
}

/// The ownership-checked product surface behind a generic Hive session ID.
///
/// Hidden group lanes are rejected by `load_owned_session_of_type` before this
/// value is constructed. Callers must then explicitly choose whether a Worker
/// DM is a valid target for the requested operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HiveSessionControlScope {
    PrimaryHive,
    WorkerDm { worker_id: String },
}

impl HiveSessionControlScope {
    pub(super) fn reject_generic_worker_control(&self) -> Result<(), AppError> {
        match self {
            Self::PrimaryHive => Ok(()),
            Self::WorkerDm { worker_id } => Err(AppError::Conflict(format!(
                "Hive Worker DM lifecycle controls must use the Worker-specific API under /api/hive/workers/{worker_id}"
            ))),
        }
    }

    pub(super) fn require_exact_worker_schedule(
        &self,
        schedule_worker_id: Option<&str>,
    ) -> Result<(), AppError> {
        match self {
            Self::PrimaryHive => Ok(()),
            Self::WorkerDm { worker_id }
                if schedule_worker_id == Some(worker_id.as_str()) =>
            {
                Ok(())
            }
            Self::WorkerDm { worker_id } => Err(AppError::Conflict(format!(
                "Hive Worker DM schedules must be explicitly bound to the exact Worker with worker_id {worker_id}"
            ))),
        }
    }
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
    // Local single-user sessions are represented by a NULL owner. Treat that
    // as an exact ownership scope, never as a wildcard for authenticated
    // users' sessions.
    session.user_id.as_deref() == user_id
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
    if session_manager.is_internal_hive_group_lane(session_id)? {
        // Group lanes are an execution detail, not user-addressable sessions.
        // Returning the same shape as a missing session avoids leaking their
        // identifiers and blocks generic read/update/delete/chat routes.
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

pub(super) fn load_owned_hive_session_control_scope(
    state: &AppState,
    session_manager: &SessionManager,
    session_id: &str,
    user: Option<&CurrentUser>,
) -> Result<HiveSessionControlScope, AppError> {
    load_owned_session_of_type(session_manager, session_id, SessionType::Hive, "Hive", user)?;

    let store = HiveWorkerStore::new(Database::new(&state.db_path)?);
    let Some(worker) = store.get_by_dm_session(session_id)? else {
        return Ok(HiveSessionControlScope::PrimaryHive);
    };
    if worker.user_id.as_deref() != current_user_id(user)
        || worker.dm_session_id.as_deref() != Some(session_id)
    {
        return Err(session_not_found(session_id));
    }
    Ok(HiveSessionControlScope::WorkerDm {
        worker_id: worker.id,
    })
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
