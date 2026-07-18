use axum::{
    extract::{Path, State},
    Json,
};

use krusty_core::agent::LoopInput;

use super::{current_user_id, load_owned_session, open_session_manager};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::types::SimpleOkResponse;
use crate::AppState;

pub(super) async fn cancel_session(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(session_id): Path<String>,
) -> Result<Json<SimpleOkResponse>, AppError> {
    let manager = open_session_manager(&state)?;
    let session = load_owned_session(&manager, &session_id, user.as_ref())?;

    if session.session_type == krusty_core::storage::SessionType::Mako {
        state
            .mako_runtime
            .cancel_session_for_user(&state, &session_id, current_user_id(user.as_ref()), None)
            .await
            .map_err(crate::mako_runtime::control_plane_app_error)?;
        return Ok(Json(SimpleOkResponse { ok: true }));
    }

    let sender = state.session_inputs.read().await.get(&session_id).cloned();
    if let Some(sender) = sender {
        sender.send(LoopInput::Cancel).map_err(|_| {
            AppError::Conflict(format!(
                "Session {session_id} is no longer accepting cancellation"
            ))
        })?;
    }

    Ok(Json(SimpleOkResponse { ok: true }))
}
