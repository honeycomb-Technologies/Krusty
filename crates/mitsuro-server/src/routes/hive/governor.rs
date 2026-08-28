//! Aggregate-only Hive Worker governor projection.
//!
//! Policy mutation intentionally does not live here. The daemon-authoritative
//! protocol will own that CAS boundary; this route only reads the migration-74
//! ledger and never exposes provider request content.

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use chrono::Utc;

use mitsuro_core::storage::{Database, HiveWorkerGovernorProjection, HiveWorkerGovernorStore};
use mitsuro_hive_protocol::WorkerGovernorRecoveryResponse;

use super::super::session_access::current_user_id;
use super::idempotency_key_from_headers;
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

pub(super) async fn get_worker_governor(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
) -> Result<Json<HiveWorkerGovernorProjection>, AppError> {
    let owner_user_id = current_user_id(user.as_ref());
    let store = HiveWorkerGovernorStore::new(Database::new(&state.db_path)?);
    let projection = store
        .get_worker_dm_projection(&worker_id, owner_user_id, Utc::now())?
        .ok_or_else(|| AppError::NotFound(format!("Worker {worker_id} not found")))?;
    Ok(Json(projection))
}

pub(super) async fn grant_worker_governor_recovery(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Path(worker_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<WorkerGovernorRecoveryResponse>, AppError> {
    let idempotency_key = idempotency_key_from_headers(&headers)?.ok_or_else(|| {
        AppError::BadRequest("Idempotency-Key is required for Worker provider recovery".into())
    })?;
    let response = state
        .hive_runtime
        .grant_worker_governor_recovery_for_user(
            current_user_id(user.as_ref()),
            &worker_id,
            &idempotency_key,
        )
        .await
        .map_err(crate::hive_runtime::control_plane_app_error)?;
    let valid = match response.status.as_str() {
        "granted" | "already_available" | "response_loss_acknowledged_with_grant" => {
            response.bypass_unresolved_provider_call
                && response
                    .grant_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
                && response
                    .expires_at
                    .as_deref()
                    .is_some_and(|expires_at| !expires_at.is_empty())
        }
        "response_loss_acknowledged" => {
            !response.bypass_unresolved_provider_call
                && response.grant_id.is_none()
                && response.expires_at.is_none()
        }
        _ => false,
    };
    if response.worker_id != worker_id || !valid {
        return Err(AppError::Internal(
            "Hive daemon returned a mismatched Worker governor recovery result".into(),
        ));
    }
    Ok(Json(response))
}
