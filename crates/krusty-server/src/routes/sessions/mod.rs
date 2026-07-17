//! Session management endpoints

mod approvals;
mod cancellation;
mod crud;
mod pinch;
mod presence;
mod state;

use axum::{
    routing::{get, post},
    Router,
};

use krusty_core::storage::Database;
use krusty_core::SessionManager;

use self::approvals::tool_approval_for_session;
use self::cancellation::cancel_session;
use self::crud::{
    create_session, delete_session, get_session, list_directories, list_sessions, update_session,
};
use self::pinch::pinch_session;
use self::presence::{get_session_presence, heartbeat_session_presence, remove_session_presence};
use self::state::{get_session_state, get_session_trace};

use super::session_access::{
    current_user_id, ensure_owned_session, load_agent_state_or_idle, load_owned_session,
    request_workspace_scope,
};
use crate::error::AppError;
use crate::AppState;

/// Build the sessions router
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_sessions).post(create_session))
        .route("/directories", get(list_directories))
        .route(
            "/:id",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        .route("/:id/state", get(get_session_state))
        .route("/:id/trace", get(get_session_trace))
        .route("/:id/cancel", post(cancel_session))
        .route(
            "/:id/presence",
            get(get_session_presence).put(heartbeat_session_presence),
        )
        .route(
            "/:id/presence/:client_id",
            axum::routing::delete(remove_session_presence),
        )
        .route("/:id/pinch", post(pinch_session))
        .route("/:id/tool-approval", post(tool_approval_for_session))
}

fn open_session_manager(state: &AppState) -> Result<SessionManager, AppError> {
    Ok(SessionManager::new(Database::new(&state.db_path)?))
}
#[cfg(test)]
mod tests;
