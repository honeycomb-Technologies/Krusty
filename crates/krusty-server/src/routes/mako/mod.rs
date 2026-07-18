//! Mako dispatch and session management endpoints

#[cfg(test)]
use std::path::PathBuf;

use axum::{
    http::HeaderMap,
    routing::{delete, get, post, put},
    Router,
};
use serde::Serialize;

#[cfg(test)]
use krusty_core::paths as core_paths;
use krusty_core::storage::Database;
use krusty_core::SessionManager;

#[cfg(test)]
use super::session_access::current_user_home_dir;
#[cfg(test)]
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

mod attention;
mod control_plane;
mod current;
mod home;
mod learning;
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
        .route("/learning-candidates", get(learning::list_candidates))
        .route(
            "/learning-candidates/:id/accept",
            post(learning::accept_candidate),
        )
        .route(
            "/learning-candidates/:id/reject",
            post(learning::reject_candidate),
        )
        .route("/daemon/recover", post(sessions::recover_daemon))
        .route("/sessions", get(sessions::list_sessions))
        .route("/sessions/:id/status", get(sessions::session_status))
        .route(
            "/sessions/:id/schedules",
            get(control_plane::list_schedules).post(control_plane::create_schedule),
        )
        .route(
            "/sessions/:id/schedules/:schedule_id",
            get(control_plane::get_schedule)
                .put(control_plane::replace_schedule)
                .delete(control_plane::cancel_schedule),
        )
        .route(
            "/sessions/:id/schedules/:schedule_id/pause",
            post(control_plane::pause_schedule),
        )
        .route(
            "/sessions/:id/schedules/:schedule_id/resume",
            post(control_plane::resume_schedule),
        )
        .route(
            "/sessions/:id/schedules/:schedule_id/occurrences",
            get(control_plane::list_occurrences),
        )
        .route("/sessions/:id/runs", get(control_plane::list_runs))
        .route("/sessions/:id/runs/:run_id", get(control_plane::get_run))
        .route(
            "/sessions/:id/runs/:run_id/attempts",
            get(control_plane::list_attempts),
        )
        .route(
            "/sessions/:id/event-log",
            get(control_plane::list_event_log),
        )
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

const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

pub(super) fn idempotency_key_from_headers(
    headers: &HeaderMap,
) -> Result<Option<String>, AppError> {
    let Some(value) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| AppError::BadRequest("Idempotency-Key is not valid ASCII".into()))?;
    if value.trim().is_empty() || value.len() > MAX_IDEMPOTENCY_KEY_BYTES {
        return Err(AppError::BadRequest(format!(
            "Idempotency-Key must contain 1 to {MAX_IDEMPOTENCY_KEY_BYTES} bytes"
        )));
    }
    if value != value.trim() {
        return Err(AppError::BadRequest(
            "Idempotency-Key must not have surrounding whitespace".into(),
        ));
    }
    Ok(Some(value.to_string()))
}

#[cfg(test)]
fn mako_home_dir_for_user(user: Option<&CurrentUser>) -> PathBuf {
    current_user_home_dir(user)
        .map(core_paths::mako_dir_for_home)
        .unwrap_or_else(core_paths::mako_dir)
}

#[cfg(test)]
mod idempotency_tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::idempotency_key_from_headers;

    #[test]
    fn idempotency_key_enforces_exact_byte_and_whitespace_bounds() {
        let mut headers = HeaderMap::new();
        let accepted = "a".repeat(256);
        headers.insert(
            "idempotency-key",
            HeaderValue::from_str(&accepted).expect("valid header"),
        );
        assert!(matches!(
            idempotency_key_from_headers(&headers),
            Ok(Some(value)) if value == accepted
        ));

        let too_long = "a".repeat(257);
        headers.insert(
            "idempotency-key",
            HeaderValue::from_str(&too_long).expect("valid header"),
        );
        assert!(idempotency_key_from_headers(&headers).is_err());

        headers.insert(
            "idempotency-key",
            HeaderValue::from_static(" leading-space"),
        );
        assert!(idempotency_key_from_headers(&headers).is_err());
    }
}

#[cfg(test)]
mod tests;
