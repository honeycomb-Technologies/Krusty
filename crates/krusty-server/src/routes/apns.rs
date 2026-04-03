//! APNs device registration and test endpoints

use axum::{
    extract::State,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use krusty_core::storage::{ApnsDeviceStore, Database};

use crate::apns::{ApnsEventType, ApnsPayload};
use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register_device))
        .route("/register", delete(unregister_device))
        .route("/status", get(status))
        .route("/test", post(send_test))
}

#[derive(Deserialize)]
struct RegisterRequest {
    device_token: String,
    bundle_id: Option<String>,
}

#[derive(Serialize)]
struct RegisterResponse {
    id: String,
}

async fn register_device(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, AppError> {
    let user_id = user.and_then(|u| u.0.user_id);
    let db = Database::new(&state.db_path)?;
    let store = ApnsDeviceStore::new(&db);
    let bundle_id = req.bundle_id.as_deref().unwrap_or("io.krusty.mobile");
    let id = store.upsert(user_id.as_deref(), &req.device_token, bundle_id)?;
    Ok(Json(RegisterResponse { id }))
}

#[derive(Deserialize)]
struct UnregisterRequest {
    device_token: String,
}

async fn unregister_device(
    State(state): State<AppState>,
    Json(req): Json<UnregisterRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = Database::new(&state.db_path)?;
    let store = ApnsDeviceStore::new(&db);
    let removed = store.remove_by_token(&req.device_token)?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

#[derive(Serialize)]
struct ApnsStatusResponse {
    apns_configured: bool,
    device_count: usize,
}

async fn status(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Json<ApnsStatusResponse>, AppError> {
    let user_id = user.and_then(|u| u.0.user_id);
    let db = Database::new(&state.db_path)?;
    let store = ApnsDeviceStore::new(&db);
    let device_count = store.count_for_user(user_id.as_deref())?;

    Ok(Json(ApnsStatusResponse {
        apns_configured: state.apns_service.is_some(),
        device_count,
    }))
}

#[derive(Deserialize, Default)]
struct TestRequest {
    title: Option<String>,
    body: Option<String>,
    session_id: Option<String>,
}

#[derive(Serialize)]
struct TestResponse {
    accepted: bool,
    attempted: usize,
    sent: usize,
    failed: usize,
    stale_removed: usize,
}

async fn send_test(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<TestRequest>,
) -> Result<Json<TestResponse>, AppError> {
    let apns_service = state
        .apns_service
        .as_ref()
        .cloned()
        .ok_or_else(|| AppError::Internal("APNs not configured".into()))?;

    let user_id = user.and_then(|u| u.0.user_id);
    let title = req.title.unwrap_or_else(|| "Krusty".into());
    let body = req
        .body
        .unwrap_or_else(|| "Test notification from Krusty".into());

    let stats = apns_service
        .notify_user(
            user_id.as_deref(),
            ApnsPayload {
                title,
                body,
                session_id: req.session_id,
                category: None,
                data: None,
            },
            ApnsEventType::Test,
        )
        .await;

    Ok(Json(TestResponse {
        accepted: stats.attempted > 0,
        attempted: stats.attempted,
        sent: stats.sent,
        failed: stats.failed,
        stale_removed: stats.stale_removed,
    }))
}
