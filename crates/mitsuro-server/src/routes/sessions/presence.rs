use axum::{
    extract::{Path, State},
    Json,
};

use super::{current_user_id, ensure_owned_session, open_session_manager};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::presence::{
    remove_presence, snapshot_presence, upsert_presence, SessionPresenceRecord,
    SessionPresenceSnapshot,
};
use crate::types::{
    SessionPresenceClientResponse, SessionPresenceHeartbeatRequest, SessionPresenceResponse,
};
use crate::AppState;

pub(super) async fn get_session_presence(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<SessionPresenceResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;

    let mut registry = state.session_presence.write().await;
    Ok(Json(map_presence_snapshot(snapshot_presence(
        &mut registry,
        &id,
    ))))
}

pub(super) async fn heartbeat_session_presence(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(id): Path<String>,
    Json(req): Json<SessionPresenceHeartbeatRequest>,
) -> Result<Json<SessionPresenceResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;

    let mut registry = state.session_presence.write().await;
    let snapshot = upsert_presence(
        &mut registry,
        &id,
        SessionPresenceRecord {
            client_id: req.client_id,
            surface: req.surface,
            capability: req.capability,
            user_id: current_user_id(user.as_ref()).map(ToOwned::to_owned),
            last_seen_at: chrono::Utc::now(),
            last_event_sequence: req.last_event_sequence,
        },
    );

    Ok(Json(map_presence_snapshot(snapshot)))
}

pub(super) async fn remove_session_presence(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path((id, client_id)): Path<(String, String)>,
) -> Result<Json<SessionPresenceResponse>, AppError> {
    let session_manager = open_session_manager(&state)?;
    ensure_owned_session(&session_manager, &id, user.as_ref())?;

    let mut registry = state.session_presence.write().await;
    Ok(Json(map_presence_snapshot(remove_presence(
        &mut registry,
        &id,
        &client_id,
    ))))
}

fn map_presence_snapshot(snapshot: SessionPresenceSnapshot) -> SessionPresenceResponse {
    SessionPresenceResponse {
        session_id: snapshot.session_id,
        active_viewers: snapshot.active_viewers,
        active_controllers: snapshot.active_controllers,
        stale_clients: snapshot.stale_clients,
        clients: snapshot
            .clients
            .into_iter()
            .map(|client| SessionPresenceClientResponse {
                client_id: client.client_id,
                surface: client.surface,
                capability: client.capability,
                user_id: client.user_id,
                last_seen_at: client.last_seen_at,
                last_event_sequence: client.last_event_sequence,
                stale: client.stale,
            })
            .collect(),
    }
}
