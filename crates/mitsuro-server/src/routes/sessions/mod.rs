//! Session management endpoints

mod approvals;
mod cancellation;
mod crud;
mod pinch;
mod presence;
mod state;
mod workflow;

use axum::{
    extract::Request,
    http::{header::VARY, HeaderValue},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};

use mitsuro_core::storage::Database;
use mitsuro_core::SessionManager;

use self::approvals::tool_approval_for_session;
use self::cancellation::cancel_session;
use self::crud::{
    create_session, delete_session, get_session, list_directories, list_sessions, update_session,
};
use self::pinch::pinch_session;
use self::presence::{get_session_presence, heartbeat_session_presence, remove_session_presence};
use self::state::{get_session_state, get_session_trace};
use self::workflow::{execute_workflow_command, get_workflow};

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
        .route("/:id/workflow", get(get_workflow))
        .route("/:id/workflow/commands", post(execute_workflow_command))
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
        .layer(middleware::from_fn(add_session_wire_vary))
}

async fn add_session_wire_vary(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().append(
        VARY,
        HeaderValue::from_static(crate::legacy_identity::SESSION_WIRE_VERSION_HEADER),
    );
    response
}

fn open_session_manager(state: &AppState) -> Result<SessionManager, AppError> {
    Ok(SessionManager::new(Database::new(&state.db_path)?))
}
#[cfg(test)]
mod tests;
