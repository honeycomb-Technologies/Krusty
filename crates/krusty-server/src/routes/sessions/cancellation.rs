use axum::{
    extract::{Path, State},
    Json,
};

use krusty_core::agent::LoopInput;

use super::{ensure_owned_session, open_session_manager};
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
    ensure_owned_session(&manager, &session_id, user.as_ref())?;

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
