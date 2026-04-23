//! Mako dispatch and session management endpoints

use std::path::PathBuf;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use serde::Serialize;

use krusty_core::paths as core_paths;
use krusty_core::storage::Database;
use krusty_core::SessionManager;

use super::session_access::current_user_home_dir;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

mod attention;
mod current;
mod home;
mod sessions;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/dispatch", post(sessions::dispatch))
        .route("/home", get(home::home))
        .route("/home/bootstrap", post(home::bootstrap_home))
        .route("/home/:kind", put(home::update_home_document))
        .route("/home/crew/:slug/:kind", put(home::update_crew_document))
        .route("/crew", get(home::crew))
        .route("/channels", get(home::channels))
        .route("/current", get(current::current))
        .route("/attention", get(attention::attention))
        .route("/attention/:id/read", post(attention::set_attention_read))
        .route(
            "/attention/:id/clear",
            post(attention::set_attention_cleared),
        )
        .route("/daemon/recover", post(sessions::recover_daemon))
        .route("/sessions", get(sessions::list_sessions))
        .route("/sessions/:id/status", get(sessions::session_status))
        .route("/sessions/:id/events", get(sessions::observe_events))
        .route("/sessions/:id/message", post(sessions::send_message))
        .route("/sessions/:id/schedule", post(sessions::schedule_session))
        .route("/sessions/:id/priority", post(sessions::set_priority))
        .route("/sessions/:id/crew", post(sessions::set_crew))
        .route("/sessions/:id/pause", post(sessions::pause_session))
        .route("/sessions/:id/resume", post(sessions::resume_session))
        .route("/sessions/:id", delete(sessions::cancel_session))
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

fn open_session_manager(state: &AppState) -> Result<SessionManager, AppError> {
    Ok(SessionManager::new(Database::new(&state.db_path)?))
}

fn mako_home_dir_for_user(user: Option<&CurrentUser>) -> PathBuf {
    current_user_home_dir(user)
        .map(core_paths::mako_dir_for_home)
        .unwrap_or_else(core_paths::mako_dir)
}
#[cfg(test)]
mod tests;
